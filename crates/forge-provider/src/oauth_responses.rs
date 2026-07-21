//! Shared OpenAI-style Responses API streaming helpers used by subscription OAuth providers
//! (`xai-oauth`, `codex-oauth`). Pure SSE framing + request mapping + one HTTP+SSE execute path
//! with optional extra headers (e.g. ChatGPT-Account-Id). Provider-specific host pins, auth, and
//! status classification stay in each provider module.

use forge_types::{Message, QuotaHint, Role, ToolCall, Usage};
use futures::StreamExt;

use crate::{CompletionOptions, EventSink, ModelResponse, ProviderError, StreamEvent, ToolSpec};

/// Refresh the access token this long before it actually expires.
pub const REFRESH_SKEW_SECS: i64 = 120;

pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Extract `error.message` (or `message`) from a JSON error body; falls back to the first line of
/// the raw body, capped so a huge/binary body can't flood the CLI or logs.
pub fn error_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(m) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return m.to_string();
        }
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    let line = body.lines().next().unwrap_or(body).trim();
    if line.chars().count() > 200 {
        let cut: String = line.chars().take(197).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

/// Whether a completion failure should trigger the one-hop next-account retry: rate limits (429)
/// and connection-level `Unavailable`. Permanent `Auth` is deliberately excluded.
pub fn should_hop_account(e: &ProviderError) -> bool {
    e.is_rate_limited() || matches!(e, ProviderError::Unavailable(_))
}

/// Classify an error surfaced INSIDE the SSE stream (an `error`/`response.failed` event has no HTTP
/// status). A transient in-stream 429/quota or 5xx/overload must map to the same variant the
/// non-streamed [`classify_responses_status`] path would produce, so it triggers the next-account
/// hop instead of being buried as a hard [`ProviderError::Request`]. Text-only heuristic — the
/// backend echoes the status class in the message.
pub fn classify_stream_error(message: String) -> ProviderError {
    let lower = message.to_ascii_lowercase();
    let rate_limited = lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("quota")
        || lower.contains("insufficient_quota");
    if rate_limited {
        return ProviderError::RateLimited {
            message,
            retry_after: None,
        };
    }
    let unavailable = lower.contains("overload")
        || lower.contains("unavailable")
        || lower.contains("temporarily")
        || lower.contains("provider request failed")
        || lower.contains(" 500")
        || lower.contains("500 ")
        || lower.contains(" 502")
        || lower.contains(" 503")
        || lower.contains(" 504")
        || lower.contains("internal server error")
        || lower.contains("bad gateway")
        || lower.contains("gateway timeout");
    if unavailable {
        return ProviderError::Unavailable(message);
    }
    ProviderError::Request(message)
}

/// Strip a `provider::` namespace: `"xai-oauth::grok-4"` → `"grok-4"`.
pub fn bare_model(model: &str) -> &str {
    model
        .split_once("::")
        .map(|(_, name)| name)
        .unwrap_or(model)
}

/// Map Forge messages/tools into an OpenAI Responses API request body (`stream: true`).
pub fn build_responses_request(
    model: &str,
    messages: &[Message],
    tools: &[ToolSpec],
    opts: &CompletionOptions,
    max_output_tokens: u32,
) -> serde_json::Value {
    let mut instructions = String::new();
    let mut input = Vec::new();
    for m in messages {
        match m.role {
            Role::System => {
                if !instructions.is_empty() {
                    instructions.push_str("\n\n");
                }
                instructions.push_str(&m.content);
            }
            Role::User => {
                input.push(serde_json::json!({"role": "user", "content": m.content}));
            }
            Role::Assistant => {
                if !m.content.is_empty() {
                    input.push(serde_json::json!({"role": "assistant", "content": m.content}));
                }
                for call in &m.tool_calls {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.args.to_string(),
                    }));
                }
            }
            Role::Tool => {
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": m.tool_call_id.clone().unwrap_or_default(),
                    "output": m.content,
                }));
            }
        }
    }

    let mut body = serde_json::json!({
        "model": bare_model(model),
        "input": input,
        "stream": true,
    });
    if !instructions.is_empty() {
        body["instructions"] = serde_json::Value::String(instructions);
    }
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(
            tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.schema,
                    })
                })
                .collect(),
        );
    }
    if max_output_tokens > 0 {
        body["max_output_tokens"] = serde_json::json!(max_output_tokens);
    }
    if let Some(temp) = opts.temperature {
        body["temperature"] = serde_json::json!(temp);
    }
    body
}

/// Accumulates a streamed Responses-API completion.
#[derive(Debug, Default)]
pub struct ResponseAccumulator {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// Whether a `response.completed` (or `.failed`) event arrived.
    pub saw_terminal: bool,
}

/// Fold one decoded SSE event into `acc`. Event-name matching is intentionally loose.
pub fn apply_sse_event(
    acc: &mut ResponseAccumulator,
    event: &str,
    data: &serde_json::Value,
    on_event: &mut EventSink<'_>,
) -> Result<(), ProviderError> {
    if event == "error" || event == "response.failed" {
        let msg = data
            .get("error")
            .or_else(|| data.get("response").and_then(|r| r.get("error")))
            .and_then(|e| e.get("message").and_then(|m| m.as_str()).or(e.as_str()))
            .unwrap_or("provider returned a stream error")
            .to_string();
        return Err(classify_stream_error(msg));
    }
    if event.ends_with("output_text.delta") {
        if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
            acc.content.push_str(delta);
            on_event(StreamEvent::Text(delta.to_string()));
        }
    } else if event.contains("reasoning") && event.ends_with(".delta") {
        if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
            on_event(StreamEvent::Reasoning(delta.to_string()));
        }
    } else if event == "response.output_item.done" {
        if let Some(item) = data.get("item") {
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let args_str = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                let args = serde_json::from_str(args_str).unwrap_or_else(|_| serde_json::json!({}));
                acc.tool_calls.push(ToolCall { id, name, args });
            }
        }
    } else if event == "response.completed" {
        acc.saw_terminal = true;
        if let Some(resp) = data.get("response") {
            if let Some(u) = resp.get("usage") {
                let input_tokens = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let output_tokens = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cached_input_tokens = u
                    .get("input_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                acc.usage = Usage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    cost_usd: 0.0,
                };
            }
            if acc.content.is_empty() {
                if let Some(output) = resp.get("output").and_then(|o| o.as_array()) {
                    for item in output {
                        if item.get("type").and_then(|t| t.as_str()) != Some("message") {
                            continue;
                        }
                        if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    acc.content.push_str(text);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Parse one SSE event block: `event:`/`data:` lines, comments ignored.
pub fn parse_sse_frame(block: &str) -> (Option<String>, String) {
    let mut event = None;
    let mut data_lines: Vec<String> = Vec::new();
    for line in block.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => event = Some(value.to_string()),
            "data" => data_lines.push(value.to_string()),
            _ => {}
        }
    }
    (event, data_lines.join("\n"))
}

pub fn take_event(buf: &mut String) -> Option<String> {
    buf.find("\n\n").map(|pos| buf.drain(..pos + 2).collect())
}

/// Classify a generic Responses-style HTTP status. Callers wrap messages for provider-specific
/// entitlement copy when needed.
pub fn classify_responses_status(
    status: u16,
    body: &str,
    retry_after: Option<std::time::Duration>,
    auth_401: &str,
    auth_403: &str,
) -> ProviderError {
    let message = error_message(body);
    match status {
        403 => ProviderError::Auth(format!("{auth_403} ({message})")),
        401 => ProviderError::Auth(format!("{auth_401} ({message})")),
        429 => ProviderError::RateLimited {
            message,
            retry_after,
        },
        500..=599 => ProviderError::Unavailable(message),
        _ => ProviderError::Request(message),
    }
}

/// One HTTP+SSE completion against a Responses endpoint with a fixed bearer token and optional
/// extra headers (e.g. `ChatGPT-Account-Id`).
///
/// `quota_from_headers` is an optional vendor-specific hook that inspects the raw response headers
/// (captured before the SSE body is consumed) and returns [`QuotaHint`]s — e.g. codex-oauth's
/// `x-codex-*` account-wide quota headers. This module stays transport/vendor-neutral: callers
/// that have nothing to extract (xai-oauth) pass `None`.
#[allow(clippy::too_many_arguments)] // shared low-level transport; grouping into a struct would
                                     // just move the same 8 knobs one level of indirection out.
pub async fn execute_responses_request(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    body: &serde_json::Value,
    extra_headers: &[(&str, &str)],
    on_event: &mut EventSink<'_>,
    classify: impl Fn(u16, &str, Option<std::time::Duration>) -> ProviderError,
    quota_from_headers: Option<fn(&reqwest::header::HeaderMap) -> Vec<QuotaHint>>,
) -> Result<ModelResponse, ProviderError> {
    let mut req = http
        .post(url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .json(body);
    for (k, v) in extra_headers {
        req = req.header(*k, *v);
    }
    let resp = tokio::time::timeout(CONNECT_TIMEOUT, req.send())
        .await
        .map_err(|_| {
            ProviderError::Unavailable(format!(
                "no response while connecting (no data for {}s)",
                CONNECT_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| ProviderError::Unavailable(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(std::time::Duration::from_secs);
        let text = resp.text().await.unwrap_or_default();
        return Err(classify(status.as_u16(), &text, retry_after));
    }

    let quotas = quota_from_headers
        .map(|f| f(resp.headers()))
        .unwrap_or_default();

    let mut acc = ResponseAccumulator::default();
    let mut buf = String::new();
    let mut stream = resp.bytes_stream();
    // Raw bytes of an incomplete UTF-8 codepoint that straddled the previous chunk boundary.
    // reqwest yields chunks at arbitrary TCP/H2 boundaries, so decoding each chunk independently
    // with from_utf8_lossy would corrupt any multi-byte codepoint (emoji/CJK/em-dash/curly quote)
    // split across chunks into U+FFFD in both the live stream and the persisted answer.
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let chunk = tokio::time::timeout(IDLE_TIMEOUT, stream.next())
            .await
            .map_err(|_| {
                ProviderError::Unavailable(format!(
                    "stream stalled (no data for {}s)",
                    IDLE_TIMEOUT.as_secs()
                ))
            })?;
        let Some(chunk) = chunk else { break };
        let bytes = chunk.map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        pending.extend_from_slice(&bytes);
        // Decode only the longest valid UTF-8 prefix; keep any incomplete trailing codepoint's
        // bytes in `pending` for the next chunk. `from_utf8_lossy` on the valid prefix never
        // substitutes.
        let valid_up_to = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(e) => e.valid_up_to(),
        };
        buf.extend(
            String::from_utf8_lossy(&pending[..valid_up_to])
                .chars()
                .filter(|&c| c != '\r'),
        );
        pending.drain(..valid_up_to);
        while let Some(raw) = take_event(&mut buf) {
            let (event, data) = parse_sse_frame(&raw);
            let Some(event) = event else { continue };
            if data.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
                continue;
            };
            apply_sse_event(&mut acc, &event, &value, on_event)?;
        }
    }

    if !pending.is_empty() {
        buf.extend(
            String::from_utf8_lossy(&pending)
                .chars()
                .filter(|&c| c != '\r'),
        );
        let raw = std::mem::take(&mut buf);
        let (event, data) = parse_sse_frame(&raw);
        if let Some(event) = event {
            if !data.is_empty() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) {
                    apply_sse_event(&mut acc, &event, &value, on_event)?;
                }
            }
        }
    } else if !buf.trim().is_empty() {
        let raw = std::mem::take(&mut buf);
        let (event, data) = parse_sse_frame(&raw);
        if let Some(event) = event {
            if !data.is_empty() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) {
                    apply_sse_event(&mut acc, &event, &value, on_event)?;
                }
            }
        }
    }

    if !acc.saw_terminal
        && acc.tool_calls.is_empty()
        && acc.usage.input_tokens == 0
        && acc.usage.output_tokens == 0
    {
        return Err(ProviderError::Unavailable(
            "stream closed without a completion signal (truncated mid-generation)".to_string(),
        ));
    }

    Ok(ModelResponse {
        content: acc.content,
        tool_calls: acc.tool_calls,
        usage: acc.usage,
        quotas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_model_strips_namespace() {
        assert_eq!(bare_model("xai-oauth::grok-4"), "grok-4");
        assert_eq!(bare_model("codex-oauth::gpt-5.5"), "gpt-5.5");
        assert_eq!(bare_model("grok-4"), "grok-4");
    }

    #[test]
    fn should_hop_covers_rate_limit_and_unavailable_not_auth() {
        let rl = ProviderError::RateLimited {
            message: "slow".into(),
            retry_after: None,
        };
        assert!(should_hop_account(&rl));
        assert!(should_hop_account(&ProviderError::Unavailable(
            "stalled".into()
        )));
        assert!(!should_hop_account(&ProviderError::Auth("401".into())));
    }

    #[test]
    fn stream_error_maps_to_hoppable_variants() {
        assert!(matches!(
            classify_stream_error("Error 429: rate limit exceeded".into()),
            ProviderError::RateLimited { .. }
        ));
        assert!(matches!(
            classify_stream_error("You have exceeded your quota".into()),
            ProviderError::RateLimited { .. }
        ));
        assert!(matches!(
            classify_stream_error("upstream is overloaded, try again".into()),
            ProviderError::Unavailable(_)
        ));
        assert!(matches!(
            classify_stream_error("internal server error".into()),
            ProviderError::Unavailable(_)
        ));
        for message in [
            "provider request failed",
            "Codex provider request failed",
            "PROVIDER REQUEST FAILED: retry later",
        ] {
            assert!(matches!(
                classify_stream_error(message.into()),
                ProviderError::Unavailable(_)
            ));
        }
        // A genuine non-transient stream error stays a hard Request failure.
        assert!(matches!(
            classify_stream_error("invalid tool schema".into()),
            ProviderError::Request(_)
        ));
    }

    #[test]
    fn stream_error_event_is_classified() {
        let mut acc = ResponseAccumulator::default();
        let sink: &mut EventSink<'_> = &mut |_| {};
        let err = apply_sse_event(
            &mut acc,
            "response.failed",
            &serde_json::json!({"response": {"error": {"message": "429 Too Many Requests"}}}),
            sink,
        )
        .expect_err("failed event must error");
        assert!(should_hop_account(&err), "in-stream 429 must be hoppable");
    }

    #[test]
    fn sse_text_deltas_fold() {
        let mut acc = ResponseAccumulator::default();
        let mut events = Vec::new();
        let sink: &mut EventSink<'_> = &mut |ev| events.push(ev);
        apply_sse_event(
            &mut acc,
            "response.output_text.delta",
            &serde_json::json!({"delta": "hi"}),
            sink,
        )
        .unwrap();
        apply_sse_event(
            &mut acc,
            "response.completed",
            &serde_json::json!({"response": {"usage": {"input_tokens": 1, "output_tokens": 1}}}),
            sink,
        )
        .unwrap();
        assert_eq!(acc.content, "hi");
        assert!(acc.saw_terminal);
        assert_eq!(acc.usage.cost_usd, 0.0);
    }
}
