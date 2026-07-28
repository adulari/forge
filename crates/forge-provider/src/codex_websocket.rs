//! Responses WebSocket transport for `codex-oauth::` models. `gpt-5.6-luna` requires
//! `wss://chatgpt.com/backend-api/codex/responses` because the plain HTTPS path 404s it. Sol and
//! Terra use the same transport for session-scoped incremental response chains when Forge supplies
//! a checkpoint, with the battle-tested HTTPS path retained as their no-output fallback. Mandatory
//! WebSocket-only models remain gated by [`CODEX_WEBSOCKET_MODELS`].
//!
//! Protocol (reverse-engineered from the vendored codex Rust source):
//! - Auth on the upgrade request is IDENTICAL to HTTP: `Authorization: Bearer <token>` +
//!   `ChatGPT-Account-Id: <id>`. The only WS-specific addition is the opt-in header
//!   `OpenAI-Beta: responses_websockets=2026-02-06` — its absence is what makes the upgrade 401,
//!   not a different credential.
//! - The client sends exactly ONE `Message::Text` frame: the SAME body the HTTP path builds
//!   (`build_responses_request` + codex shaping — `store: false`, `CODEX_UNSUPPORTED_PARAMS`
//!   stripped), plus a top-level `"type": "response.create"`.
//! - The server sends one JSON object per `Message::Text` frame, dispatched on its own
//!   `"type"`: `codex.rate_limits` (mapped to [`forge_types::QuotaHint`] here — the live fix for
//!   the HTTP path's `quotas: Vec::new()` hardcode, WS-only), `error` (mapped via the SAME status
//!   classifier the HTTP path uses), or any Responses-API event name, folded through the SAME
//!   [`crate::oauth_responses::apply_sse_event`] the SSE path uses (identical event schema).

use forge_types::QuotaHint;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

use crate::oauth_responses::{apply_sse_event, ResponseAccumulator, CONNECT_TIMEOUT, IDLE_TIMEOUT};
use crate::{EventSink, ModelResponse, ProviderError};

type CodexWsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// `codex-oauth::` model ids the ChatGPT backend serves ONLY over WebSocket. Keep this list, and
/// only this list, on the WS path — greppable + extensible for future "v1"-tagged models.
pub const CODEX_WEBSOCKET_MODELS: &[&str] = &["gpt-5.6-luna"];

/// The opt-in header the ChatGPT backend requires to accept the WS upgrade. Literal string, as
/// specified — its absence is the likely cause of a prior probe's 401 (missing header, not a
/// different auth mechanism).
const OPENAI_BETA_WEBSOCKETS: &str = "responses_websockets=2026-02-06";

/// True iff `model` (namespaced or bare) is one of [`CODEX_WEBSOCKET_MODELS`].
pub fn is_websocket_model(model: &str) -> bool {
    let bare = crate::oauth_responses::bare_model(model);
    CODEX_WEBSOCKET_MODELS.contains(&bare)
}

/// `https://…/responses` → `wss://…/responses` (scheme only, same host/path). `http://` (test
/// mock servers) maps to `ws://` so this stays testable without a real TLS endpoint.
pub fn to_ws_url(url: &str) -> Result<String, ProviderError> {
    if let Some(rest) = url.strip_prefix("https://") {
        Ok(format!("wss://{rest}"))
    } else if let Some(rest) = url.strip_prefix("http://") {
        Ok(format!("ws://{rest}"))
    } else {
        Err(ProviderError::Request(format!(
            "codex WS transport: unexpected URL scheme in {url:?}"
        )))
    }
}

/// Clone `body` and stamp the WS-only external tag. `body` must already be codex-shaped
/// (`store: false`, `CODEX_UNSUPPORTED_PARAMS` stripped — same shaping `codex_oauth.rs` applies
/// for the HTTP path); this adds only what the WS protocol needs on top.
fn to_ws_frame(body: &serde_json::Value) -> serde_json::Value {
    let mut framed = body.clone();
    framed["type"] = serde_json::json!("response.create");
    framed
}

/// Models known to accept the Responses WebSocket protocol used by native Codex. Luna requires
/// it; Sol and Terra can fall back to HTTPS if the optional incremental path is unavailable.
pub fn supports_incremental_session(model: &str) -> bool {
    matches!(
        crate::oauth_responses::bare_model(model),
        "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna"
    )
}

fn request_properties_match(previous: &serde_json::Value, current: &serde_json::Value) -> bool {
    let mut previous = previous.clone();
    let mut current = current.clone();
    previous
        .as_object_mut()
        .map(|object| object.remove("input"));
    current.as_object_mut().map(|object| object.remove("input"));
    previous == current
}

fn incremental_input(
    previous: &serde_json::Value,
    previous_response_items: &[serde_json::Value],
    current: &serde_json::Value,
) -> Option<Vec<serde_json::Value>> {
    if !request_properties_match(previous, current) {
        return None;
    }
    let previous_input = previous.get("input")?.as_array()?;
    let current_input = current.get("input")?.as_array()?;
    let expected_len = previous_input.len() + previous_response_items.len();
    if current_input.len() < expected_len
        || current_input[..previous_input.len()] != previous_input[..]
        || current_input[previous_input.len()..expected_len] != *previous_response_items
    {
        return None;
    }
    Some(current_input[expected_len..].to_vec())
}

/// One session-scoped Codex Responses WebSocket chain. Requests inside the agent/tool loop append
/// only new items through `previous_response_id`; Forge may retain the same connection for a
/// classifier-confirmed dependent user continuation on the exact session/model/account. The
/// provider boundary is replaced for independent turns and every identity change.
pub struct CodexTurnWebsocket {
    stream: CodexWsStream,
    turn_state: Option<String>,
    last_request: Option<serde_json::Value>,
    last_response_id: Option<String>,
    last_response_items: Vec<serde_json::Value>,
}

/// Last fully acknowledged point in one turn's incremental response chain. A transport reconnect
/// may safely resume from this state because it excludes the failed/in-flight request. The backend
/// still validates the response id; `previous_response_not_found` resets to a full logical request.
#[derive(Clone, Debug)]
pub(crate) struct IncrementalHistory {
    last_request: serde_json::Value,
    last_response_id: String,
    last_response_items: Vec<serde_json::Value>,
}

impl CodexTurnWebsocket {
    pub async fn connect(
        ws_url: &str,
        token: &str,
        chatgpt_account_id: &str,
        prior_turn_state: Option<&str>,
        classify: &impl Fn(u16, &str, Option<std::time::Duration>) -> ProviderError,
    ) -> Result<Self, ProviderError> {
        let uri: Uri = ws_url.parse().map_err(|error| {
            ProviderError::Request(format!("codex WS transport: bad URL {ws_url:?}: {error}"))
        })?;
        let mut request = ClientRequestBuilder::new(uri)
            .with_header("Authorization", format!("Bearer {token}"))
            .with_header("ChatGPT-Account-Id", chatgpt_account_id.to_string())
            .with_header("OpenAI-Beta", OPENAI_BETA_WEBSOCKETS.to_string())
            .with_header("originator", "codex_cli_rs".to_string())
            .with_header("User-Agent", user_agent());
        if let Some(turn_state) = prior_turn_state {
            request = request.with_header("x-codex-turn-state", turn_state.to_string());
        }
        let (stream, response) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(request))
            .await
            .map_err(|_| {
                ProviderError::Unavailable(format!(
                    "codex WS: no response while connecting (no data for {}s)",
                    CONNECT_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|error| ws_error_to_provider_error(error, classify))?;
        let turn_state = response
            .headers()
            .get("x-codex-turn-state")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .or_else(|| prior_turn_state.map(str::to_owned));
        Ok(Self {
            stream,
            turn_state,
            last_request: None,
            last_response_id: None,
            last_response_items: Vec::new(),
        })
    }

    pub fn turn_state(&self) -> Option<&str> {
        self.turn_state.as_deref()
    }

    pub(crate) fn incremental_history(&self) -> Option<IncrementalHistory> {
        Some(IncrementalHistory {
            last_request: self.last_request.clone()?,
            last_response_id: self.last_response_id.clone()?,
            last_response_items: self.last_response_items.clone(),
        })
    }

    pub(crate) fn restore_incremental_history(&mut self, history: IncrementalHistory) {
        self.last_request = Some(history.last_request);
        self.last_response_id = Some(history.last_response_id);
        self.last_response_items = history.last_response_items;
    }

    /// Keep the authenticated live socket and turn-state route, but start the next logical request
    /// from its full visible transcript. This drops hidden reasoning carried by
    /// `previous_response_id` without paying another WebSocket handshake.
    pub(crate) fn reset_incremental_history(&mut self) {
        self.last_request = None;
        self.last_response_id = None;
        self.last_response_items.clear();
    }

    fn frame_for(&self, body: &serde_json::Value) -> serde_json::Value {
        let mut frame = to_ws_frame(body);
        let incremental = self
            .last_request
            .as_ref()
            .zip(self.last_response_id.as_ref())
            .and_then(|(previous, response_id)| {
                incremental_input(previous, &self.last_response_items, body)
                    .map(|input| (response_id, input))
            });
        if let Some((response_id, input)) = incremental {
            frame["previous_response_id"] = serde_json::json!(response_id);
            frame["input"] = serde_json::Value::Array(input);
        }
        frame
    }

    pub async fn complete(
        &mut self,
        body: &serde_json::Value,
        on_event: &mut EventSink<'_>,
        classify: impl Fn(u16, &str, Option<std::time::Duration>) -> ProviderError,
    ) -> Result<ModelResponse, ProviderError> {
        let frame = self.frame_for(body);
        let frame_json = serde_json::to_string(&frame)
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        self.stream
            .send(Message::Text(frame_json.into()))
            .await
            .map_err(|error| ws_error_to_provider_error(error, &classify))?;

        let mut acc = ResponseAccumulator::default();
        let mut quotas: Vec<QuotaHint> = Vec::new();
        loop {
            let next = tokio::time::timeout(IDLE_TIMEOUT, self.stream.next())
                .await
                .map_err(|_| {
                    ProviderError::Unavailable(format!(
                        "codex WS: stream stalled (no data for {}s)",
                        IDLE_TIMEOUT.as_secs()
                    ))
                })?;
            let Some(message) = next else { break };
            let message = message.map_err(|error| ws_error_to_provider_error(error, &classify))?;
            match message {
                Message::Text(text) => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    let Some(event_type) = value.get("type").and_then(|event| event.as_str())
                    else {
                        continue;
                    };
                    match event_type {
                        "codex.rate_limits" => quotas = parse_rate_limits_frame(&value),
                        "error" => return Err(classify_error_frame(&value, &classify)),
                        "response.completed" => {
                            apply_sse_event(&mut acc, event_type, &value, on_event)?;
                            break;
                        }
                        other => apply_sse_event(&mut acc, other, &value, on_event)?,
                    }
                }
                Message::Close(_) => {
                    return Err(ProviderError::Unavailable(
                        "codex WS connection closed by server".to_string(),
                    ));
                }
                Message::Ping(payload) => {
                    self.stream
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| ws_error_to_provider_error(error, &classify))?;
                }
                Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
            }
        }

        if !acc.saw_terminal
            && acc.tool_calls.is_empty()
            && acc.usage.input_tokens == 0
            && acc.usage.output_tokens == 0
        {
            return Err(ProviderError::Unavailable(
                "codex WS stream closed without a completion signal (truncated mid-generation)"
                    .to_string(),
            ));
        }

        let last_response_items = std::mem::take(&mut acc.output_items);
        let response = ModelResponse {
            content: acc.content,
            tool_calls: acc.tool_calls,
            usage: acc.usage,
            quotas,
        };
        self.last_request = Some(body.clone());
        self.last_response_id = acc.response_id;
        self.last_response_items = last_response_items;
        Ok(response)
    }
}

fn user_agent() -> String {
    format!("codex_cli_rs/{} (forge)", env!("CARGO_PKG_VERSION"))
}

/// Map a tungstenite error to a [`ProviderError`]. `Error::Http` (the upgrade was rejected with a
/// real HTTP status) goes through the SAME classifier the HTTP path uses, so a WS 401/403/429
/// reads identically to the mesh. Every other tungstenite error (IO/TLS/protocol/closed) is a
/// connection-level failure — `Unavailable`, same as the HTTP path's stall/connect handling.
fn ws_error_to_provider_error(
    e: WsError,
    classify: &impl Fn(u16, &str, Option<std::time::Duration>) -> ProviderError,
) -> ProviderError {
    match e {
        WsError::Http(resp) => {
            let status = resp.status().as_u16();
            let body = resp
                .body()
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();
            classify(status, &body, None)
        }
        other => ProviderError::Unavailable(other.to_string()),
    }
}

/// Map a `{"type":"error","status":N,"error":{...}}` frame. Preserves
/// `websocket_connection_limit_reached` as a retryable `Unavailable` (per spec); everything else
/// goes through the SAME classifier the HTTP path uses.
fn classify_error_frame(
    value: &serde_json::Value,
    classify: &impl Fn(u16, &str, Option<std::time::Duration>) -> ProviderError,
) -> ProviderError {
    let error_obj = value
        .get("error")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let code = error_obj.get("code").and_then(|c| c.as_str()).unwrap_or("");
    if code == "websocket_connection_limit_reached" {
        return ProviderError::Unavailable(
            "codex WS: websocket_connection_limit_reached (retryable)".to_string(),
        );
    }
    let status = value.get("status").and_then(|s| s.as_u64()).unwrap_or(0) as u16;
    if status == 0 {
        // Some backend-internal failures arrive without an HTTP-style status. Reuse the
        // in-stream classifier so a generic retryable server failure does not become a hard
        // `Request` merely because the frame omitted `status`; malformed/schema errors remain
        // non-retryable.
        return crate::oauth_responses::classify_stream_error(
            crate::oauth_responses::error_message(&error_obj.to_string()),
        );
    }
    classify(status, &error_obj.to_string(), None)
}

/// Build [`QuotaHint`]s from a `{"type":"codex.rate_limits","rate_limits":{...},...}` frame. The
/// `rate_limits` shape mirrors the HTTP path's `x-codex-*`/rollout `rate_limits` object (see
/// `cli_provider::codex_quota_from_rollout`): primary/secondary each with `used_percent`,
/// `window_minutes` (300→"five_hour", 10080→"weekly"), and the window's absolute reset time.
/// NOTE the field-name trap: the live WS frame spells it `reset_at` (singular — verified against
/// codex's own event parser, `codex-rs/codex-api/src/rate_limits.rs::RateLimitEventWindow`, and
/// its WS test fixtures), while the CLI-written rollout files spell it `resets_at`. Both are read
/// here, `reset_at` first. Any window missing `used_percent` is skipped — no hint rather than a
/// wrong one.
fn parse_rate_limits_frame(value: &serde_json::Value) -> Vec<QuotaHint> {
    let Some(rl) = value.get("rate_limits").filter(|r| r.is_object()) else {
        return Vec::new();
    };
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let reached_type = rl.get("rate_limit_reached_type").and_then(|v| v.as_str());

    let mut hints = Vec::new();
    for (key, reached_key) in [("primary", "primary"), ("secondary", "secondary")] {
        let Some(w) = rl.get(key) else { continue };
        let Some(used) = w.get("used_percent").and_then(|v| v.as_f64()) else {
            continue;
        };
        let resets = w
            .get("reset_at")
            .or_else(|| w.get("resets_at"))
            .and_then(|v| v.as_i64());
        let mins = w
            .get("window_minutes")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        // Skip stale windows (the period has already reset) — same rule as the HTTP/rollout path.
        if let Some(r) = resets {
            if r <= now_secs {
                continue;
            }
        }
        let fraction = used / 100.0;
        let reached = reached_type.is_some_and(|rt| rt == reached_key);
        let status = if reached || fraction >= 0.98 {
            forge_types::QuotaStatus::Exhausted
        } else if fraction >= 0.80 {
            forge_types::QuotaStatus::Warning
        } else {
            forge_types::QuotaStatus::Ok
        };
        let label = match mins {
            300 => "five_hour".to_string(),
            10080 => "weekly".to_string(),
            m if m > 0 => format!("{m}m"),
            _ => key.to_string(),
        };
        hints.push(QuotaHint {
            provider: crate::codex_oauth::CODEX_OAUTH_NAMESPACE.to_string(),
            window: label,
            status,
            resets_at: resets,
            fraction_used: Some(fraction),
        });
    }
    hints
}

/// One WS request/response cycle: upgrade, send exactly one `response.create` frame, fold every
/// server frame until `response.completed` or the connection ends. `body` must already be
/// codex-shaped (see [`to_ws_frame`]). `classify` is the SAME status classifier the HTTP path uses
/// (`codex_oauth::classify_codex_status`), so WS and HTTP failures read identically to the mesh.
/// Does NOT refresh/retry on 401 — that's the caller's job (mirrors the HTTP `execute` /
/// `execute_ws` split in `codex_oauth.rs`).
pub async fn run(
    ws_url: &str,
    token: &str,
    chatgpt_account_id: &str,
    body: &serde_json::Value,
    on_event: &mut EventSink<'_>,
    classify: impl Fn(u16, &str, Option<std::time::Duration>) -> ProviderError,
) -> Result<ModelResponse, ProviderError> {
    let uri: Uri = ws_url.parse().map_err(|e| {
        ProviderError::Request(format!("codex WS transport: bad URL {ws_url:?}: {e}"))
    })?;
    let request = ClientRequestBuilder::new(uri)
        .with_header("Authorization", format!("Bearer {token}"))
        .with_header("ChatGPT-Account-Id", chatgpt_account_id.to_string())
        .with_header("OpenAI-Beta", OPENAI_BETA_WEBSOCKETS.to_string())
        .with_header("originator", "codex_cli_rs".to_string())
        .with_header("User-Agent", user_agent());

    let (mut ws_stream, _resp) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(request))
        .await
        .map_err(|_| {
            ProviderError::Unavailable(format!(
                "codex WS: no response while connecting (no data for {}s)",
                CONNECT_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| ws_error_to_provider_error(e, &classify))?;

    let frame = to_ws_frame(body);
    let frame_json = serde_json::to_string(&frame).map_err(|e| {
        ProviderError::Request(format!("codex WS transport: body serialize failed: {e}"))
    })?;
    ws_stream
        .send(Message::Text(frame_json.into()))
        .await
        .map_err(|e| ws_error_to_provider_error(e, &classify))?;

    let mut acc = ResponseAccumulator::default();
    let mut quotas: Vec<QuotaHint> = Vec::new();
    loop {
        let next = tokio::time::timeout(IDLE_TIMEOUT, ws_stream.next())
            .await
            .map_err(|_| {
                ProviderError::Unavailable(format!(
                    "codex WS: stream stalled (no data for {}s)",
                    IDLE_TIMEOUT.as_secs()
                ))
            })?;
        let Some(msg) = next else { break };
        let msg = msg.map_err(|e| ws_error_to_provider_error(e, &classify))?;
        match msg {
            Message::Text(text) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                let Some(event_type) = value.get("type").and_then(|t| t.as_str()) else {
                    continue;
                };
                match event_type {
                    "codex.rate_limits" => quotas = parse_rate_limits_frame(&value),
                    "error" => return Err(classify_error_frame(&value, &classify)),
                    "response.completed" => {
                        apply_sse_event(&mut acc, event_type, &value, on_event)?;
                        break;
                    }
                    other => apply_sse_event(&mut acc, other, &value, on_event)?,
                }
            }
            Message::Close(_) => {
                return Err(ProviderError::Unavailable(
                    "codex WS connection closed by server".to_string(),
                ));
            }
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
        }
    }

    if !acc.saw_terminal
        && acc.tool_calls.is_empty()
        && acc.usage.input_tokens == 0
        && acc.usage.output_tokens == 0
    {
        return Err(ProviderError::Unavailable(
            "codex WS stream closed without a completion signal (truncated mid-generation)"
                .to_string(),
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
    use crate::codex_oauth::CODEX_UNSUPPORTED_PARAMS;
    use crate::oauth_responses::build_responses_request;
    use crate::CompletionOptions;
    use forge_types::{Message as FMessage, QuotaStatus};

    #[test]
    fn is_websocket_model_gates_to_named_ids_only() {
        assert!(is_websocket_model("codex-oauth::gpt-5.6-luna"));
        assert!(is_websocket_model("gpt-5.6-luna"), "bare id also matches");
        assert!(!is_websocket_model("codex-oauth::gpt-5.6-sol"));
        assert!(!is_websocket_model("codex-oauth::gpt-5.6-terra"));
        assert!(!is_websocket_model("codex-oauth::gpt-5.5"));
        assert!(!is_websocket_model("codex-oauth::gpt-5.4-mini"));
    }

    #[test]
    fn incremental_session_support_is_explicitly_model_bounded() {
        assert!(supports_incremental_session("codex-oauth::gpt-5.6-sol"));
        assert!(supports_incremental_session("codex-oauth::gpt-5.6-terra"));
        assert!(supports_incremental_session("codex-oauth::gpt-5.6-luna"));
        assert!(!supports_incremental_session("codex-oauth::gpt-5.5"));
    }

    #[test]
    fn incremental_input_requires_an_exact_logical_extension() {
        let previous = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [{"role": "user", "content": "task"}],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
        });
        let response = vec![serde_json::json!({
            "type": "function_call",
            "call_id": "call-1",
            "name": "shell",
            "arguments": "{\"command\":\"true\"}",
        })];
        let tool_result = serde_json::json!({
            "type": "function_call_output",
            "call_id": "call-1",
            "output": "ok",
        });
        let current = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [
                {"role": "user", "content": "task"},
                response[0].clone(),
                tool_result.clone(),
            ],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
        });
        assert_eq!(
            incremental_input(&previous, &response, &current),
            Some(vec![tool_result])
        );

        let changed_tools = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": current["input"].clone(),
            "tools": [{"type": "function", "name": "different"}],
            "store": false,
        });
        assert_eq!(
            incremental_input(&previous, &response, &changed_tools),
            None,
            "a changed reusable prefix must force a full request"
        );

        let compacted = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [
                {"role": "system", "content": "Earlier conversation summarized."},
                {"role": "user", "content": "continue"},
            ],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
        });
        assert_eq!(
            incremental_input(&previous, &response, &compacted),
            None,
            "compaction that rewrites the logical prefix must send a full request"
        );
    }

    #[tokio::test]
    #[allow(
        clippy::result_large_err,
        reason = "tungstenite's required handshake callback owns its large HTTP error response"
    )]
    async fn turn_websocket_reuses_response_id_and_sends_only_incremental_items() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (frames_tx, frames_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_hdr_async(
                stream,
                |_: &tokio_tungstenite::tungstenite::handshake::server::Request,
                 mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "x-codex-turn-state",
                    tokio_tungstenite::tungstenite::http::HeaderValue::from_static("turn-state-1"),
                );
                Ok(response)
            },
            )
            .await
            .unwrap();
            let mut frames = Vec::new();
            for (response_id, with_tool_call) in [("resp-1", true), ("resp-2", false)] {
                let Message::Text(frame) = websocket.next().await.unwrap().unwrap() else {
                    panic!("expected text request");
                };
                frames.push(serde_json::from_str::<serde_json::Value>(&frame).unwrap());
                if with_tool_call {
                    websocket
                        .send(Message::Text(
                            serde_json::json!({
                                "type": "response.output_item.done",
                                "item": {
                                    "type": "function_call",
                                    "call_id": "call-1",
                                    "name": "shell",
                                    "arguments": "{\"command\":\"true\"}",
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                }
                websocket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "response.completed",
                            "response": {
                                "id": response_id,
                                "usage": {
                                    "input_tokens": 1200,
                                    "output_tokens": 10,
                                    "input_tokens_details": {"cached_tokens": 1024}
                                },
                                "output": []
                            }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
            }
            frames_tx.send(frames).unwrap();
        });

        let url = format!("ws://{address}/responses");
        let mut session = CodexTurnWebsocket::connect(
            &url,
            "test-token",
            "test-account",
            None,
            &|status, body, _| ProviderError::Request(format!("{status}: {body}")),
        )
        .await
        .unwrap();
        assert_eq!(session.turn_state(), Some("turn-state-1"));
        let first = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [{"role": "user", "content": "task"}],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
            "stream": true,
            "prompt_cache_key": "session-1",
        });
        let mut sink = |_: crate::StreamEvent| {};
        let first_response = session
            .complete(&first, &mut sink, |status, body, _| {
                ProviderError::Request(format!("{status}: {body}"))
            })
            .await
            .unwrap();
        assert_eq!(first_response.tool_calls.len(), 1);

        let second = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [
                {"role": "user", "content": "task"},
                {
                    "type": "function_call",
                    "call_id": "call-1",
                    "name": "shell",
                    "arguments": "{\"command\":\"true\"}",
                },
                {
                    "type": "function_call_output",
                    "call_id": "call-1",
                    "output": "ok",
                }
            ],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
            "stream": true,
            "prompt_cache_key": "session-1",
        });
        session
            .complete(&second, &mut sink, |status, body, _| {
                ProviderError::Request(format!("{status}: {body}"))
            })
            .await
            .unwrap();

        let frames = frames_rx.await.unwrap();
        server.await.unwrap();
        assert_eq!(frames[0].get("previous_response_id"), None);
        assert_eq!(frames[0]["input"], first["input"]);
        assert_eq!(frames[1]["previous_response_id"], "resp-1");
        assert_eq!(
            frames[1]["input"],
            serde_json::json!([{
                "type": "function_call_output",
                "call_id": "call-1",
                "output": "ok",
            }])
        );
    }

    #[tokio::test]
    #[allow(
        clippy::result_large_err,
        reason = "tungstenite's required handshake callback owns its large HTTP error response"
    )]
    async fn dependent_user_turn_reuses_live_socket_and_sends_only_new_user_input() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (frames_tx, frames_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_hdr_async(
                stream,
                |_: &tokio_tungstenite::tungstenite::handshake::server::Request,
                 mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    response.headers_mut().insert(
                        "x-codex-turn-state",
                        tokio_tungstenite::tungstenite::http::HeaderValue::from_static(
                            "turn-state-1",
                        ),
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let mut frames = Vec::new();
            for (response_id, answer) in [
                ("resp-1", "The initial implementation is complete."),
                ("resp-2", "The continuation is complete."),
            ] {
                let Message::Text(frame) = websocket.next().await.unwrap().unwrap() else {
                    panic!("expected text request");
                };
                frames.push(serde_json::from_str::<serde_json::Value>(&frame).unwrap());
                websocket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "response.output_text.delta",
                            "delta": answer,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                websocket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "response.output_item.done",
                            "item": {
                                "type": "message",
                                "role": "assistant",
                                "content": [{
                                    "type": "output_text",
                                    "text": answer,
                                }],
                            }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                websocket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "response.completed",
                            "response": {
                                "id": response_id,
                                "usage": {
                                    "input_tokens": 1200,
                                    "output_tokens": 10,
                                    "input_tokens_details": {"cached_tokens": 1024}
                                }
                            }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
            }
            frames_tx.send(frames).unwrap();
        });

        let url = format!("ws://{address}/responses");
        let mut session = CodexTurnWebsocket::connect(
            &url,
            "test-token",
            "test-account",
            None,
            &|status, body, _| ProviderError::Request(format!("{status}: {body}")),
        )
        .await
        .unwrap();
        let first = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [{"role": "user", "content": "implement the scheduler fix"}],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
            "stream": true,
            "prompt_cache_key": "session-1",
        });
        let mut sink = |_: crate::StreamEvent| {};
        let first_response = session
            .complete(&first, &mut sink, |status, body, _| {
                ProviderError::Request(format!("{status}: {body}"))
            })
            .await
            .unwrap();
        assert_eq!(
            first_response.content,
            "The initial implementation is complete."
        );

        let second = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [
                {"role": "user", "content": "implement the scheduler fix"},
                {"role": "assistant", "content": "The initial implementation is complete."},
                {"role": "user", "content": "continue"},
            ],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
            "stream": true,
            "prompt_cache_key": "session-1",
        });
        session
            .complete(&second, &mut sink, |status, body, _| {
                ProviderError::Request(format!("{status}: {body}"))
            })
            .await
            .unwrap();

        let frames = frames_rx.await.unwrap();
        server.await.unwrap();
        assert_eq!(frames[0].get("previous_response_id"), None);
        assert_eq!(frames[0]["input"], first["input"]);
        assert_eq!(frames[1]["previous_response_id"], "resp-1");
        assert_eq!(
            frames[1]["input"],
            serde_json::json!([{"role": "user", "content": "continue"}])
        );
    }

    #[tokio::test]
    #[allow(
        clippy::result_large_err,
        reason = "tungstenite's required handshake callback owns its large HTTP error response"
    )]
    async fn reconnect_restores_last_acknowledged_incremental_chain() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (frames_tx, frames_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut frames = Vec::new();
            for connection in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut websocket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |_: &tokio_tungstenite::tungstenite::handshake::server::Request,
                     mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                        response.headers_mut().insert(
                            "x-codex-turn-state",
                            tokio_tungstenite::tungstenite::http::HeaderValue::from_static(
                                "turn-state-1",
                            ),
                        );
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                let Message::Text(frame) = websocket.next().await.unwrap().unwrap() else {
                    panic!("expected text request");
                };
                frames.push(serde_json::from_str::<serde_json::Value>(&frame).unwrap());
                if connection == 0 {
                    websocket
                        .send(Message::Text(
                            serde_json::json!({
                                "type": "response.output_item.done",
                                "item": {
                                    "type": "function_call",
                                    "call_id": "call-1",
                                    "name": "shell",
                                    "arguments": "{\"command\":\"true\"}",
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                }
                websocket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "response.completed",
                            "response": {
                                "id": if connection == 0 { "resp-1" } else { "resp-2" },
                                "usage": {
                                    "input_tokens": 1200,
                                    "output_tokens": 10,
                                    "input_tokens_details": {"cached_tokens": 1024}
                                },
                                "output": []
                            }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
            }
            frames_tx.send(frames).unwrap();
        });

        let url = format!("ws://{address}/responses");
        let classify = |status, body: &str, _| ProviderError::Request(format!("{status}: {body}"));
        let mut first_socket =
            CodexTurnWebsocket::connect(&url, "test-token", "test-account", None, &classify)
                .await
                .unwrap();
        let first = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [{"role": "user", "content": "task"}],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
            "stream": true,
            "prompt_cache_key": "session-1",
        });
        let mut sink = |_: crate::StreamEvent| {};
        first_socket
            .complete(&first, &mut sink, classify)
            .await
            .unwrap();
        let history = first_socket.incremental_history().unwrap();
        let turn_state = first_socket.turn_state().map(str::to_owned);
        drop(first_socket);

        let second = serde_json::json!({
            "model": "gpt-5.6-sol",
            "input": [
                {"role": "user", "content": "task"},
                {
                    "type": "function_call",
                    "call_id": "call-1",
                    "name": "shell",
                    "arguments": "{\"command\":\"true\"}",
                },
                {
                    "type": "function_call_output",
                    "call_id": "call-1",
                    "output": "ok",
                }
            ],
            "tools": [{"type": "function", "name": "shell"}],
            "store": false,
            "stream": true,
            "prompt_cache_key": "session-1",
        });
        let mut second_socket = CodexTurnWebsocket::connect(
            &url,
            "test-token",
            "test-account",
            turn_state.as_deref(),
            &classify,
        )
        .await
        .unwrap();
        second_socket.restore_incremental_history(history);
        second_socket
            .complete(&second, &mut sink, classify)
            .await
            .unwrap();

        let frames = frames_rx.await.unwrap();
        server.await.unwrap();
        assert_eq!(frames[0].get("previous_response_id"), None);
        assert_eq!(frames[1]["previous_response_id"], "resp-1");
        assert_eq!(
            frames[1]["input"],
            serde_json::json!([{
                "type": "function_call_output",
                "call_id": "call-1",
                "output": "ok",
            }])
        );
    }

    #[test]
    fn to_ws_url_swaps_scheme_only() {
        assert_eq!(
            to_ws_url("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            "wss://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            to_ws_url("http://127.0.0.1:9999/responses").unwrap(),
            "ws://127.0.0.1:9999/responses"
        );
        assert!(to_ws_url("ftp://nope").is_err());
    }

    /// Shape a body exactly the way `codex_oauth.rs::complete_with` does before dispatching to
    /// either transport, then stamp the WS tag — this is what `run()` sends over the wire.
    fn luna_ws_frame(max_output_tokens: u32, opts: &CompletionOptions) -> serde_json::Value {
        let messages = vec![FMessage::user("hi")];
        let mut body = build_responses_request(
            "codex-oauth::gpt-5.6-luna",
            &messages,
            &[],
            opts,
            max_output_tokens,
        );
        body["store"] = serde_json::json!(false);
        if let Some(obj) = body.as_object_mut() {
            for k in CODEX_UNSUPPORTED_PARAMS {
                obj.remove(*k);
            }
        }
        to_ws_frame(&body)
    }

    #[test]
    fn ws_frame_has_response_create_tag_and_codex_shaping() {
        let opts = CompletionOptions {
            temperature: Some(0.3),
            ..Default::default()
        };
        let framed = luna_ws_frame(4096, &opts);
        assert_eq!(framed["type"], "response.create");
        assert_eq!(framed["store"], false);
        assert_eq!(framed["model"], "gpt-5.6-luna");
        assert!(framed.get("max_output_tokens").is_none());
        assert!(framed.get("temperature").is_none());

        let serialized = serde_json::to_string(&framed).unwrap();
        assert!(serialized.contains(r#""type":"response.create""#));
        assert!(serialized.contains(r#""store":false"#));
        assert!(!serialized.contains("max_output_tokens"));
    }

    fn rate_limits_frame(rate_limits: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "codex.rate_limits",
            "rate_limits": rate_limits,
            "credits": {"remaining": 100},
            "plan_type": "plus",
        })
    }

    #[test]
    fn rate_limits_frame_maps_both_windows() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        // `reset_at` (singular) is the live WS frame's spelling — codex's own parser and test
        // fixtures use it; `resets_at` only appears in CLI-written rollout files.
        let frame = rate_limits_frame(serde_json::json!({
            "primary": {"used_percent": 42.0, "window_minutes": 300, "reset_at": now + 3600},
            "secondary": {"used_percent": 10.0, "window_minutes": 10080, "reset_at": now + 86400},
        }));
        let hints = parse_rate_limits_frame(&frame);
        assert_eq!(hints.len(), 2);
        let five_hour = hints.iter().find(|h| h.window == "five_hour").unwrap();
        assert_eq!(five_hour.provider, "codex-oauth");
        assert_eq!(five_hour.fraction_used, Some(0.42));
        assert_eq!(five_hour.status, QuotaStatus::Ok);
        assert_eq!(
            five_hour.resets_at,
            Some(now + 3600),
            "the live frame's reset_at must land in the hint, not NULL"
        );
        let weekly = hints.iter().find(|h| h.window == "weekly").unwrap();
        assert_eq!(weekly.fraction_used, Some(0.10));
        assert_eq!(weekly.status, QuotaStatus::Ok);
        assert_eq!(weekly.resets_at, Some(now + 86400));
    }

    #[test]
    fn rate_limits_frame_accepts_rollout_style_resets_at_spelling() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let frame = rate_limits_frame(serde_json::json!({
            "primary": {"used_percent": 42.0, "window_minutes": 300, "resets_at": now + 3600},
        }));
        let hints = parse_rate_limits_frame(&frame);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].resets_at, Some(now + 3600), "fallback spelling");
    }

    #[test]
    fn rate_limits_frame_skips_window_missing_used_percent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let frame = rate_limits_frame(serde_json::json!({
            "primary": {"window_minutes": 300, "resets_at": now + 3600},
            "secondary": {"used_percent": 55.0, "window_minutes": 10080, "resets_at": now + 86400},
        }));
        let hints = parse_rate_limits_frame(&frame);
        assert_eq!(hints.len(), 1, "no hint for primary — missing used_percent");
        assert_eq!(hints[0].window, "weekly");
    }

    #[test]
    fn rate_limits_frame_marks_exhausted_window() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let frame = rate_limits_frame(serde_json::json!({
            "primary": {"used_percent": 100.0, "window_minutes": 300, "resets_at": now + 3600},
            "rate_limit_reached_type": "primary",
        }));
        let hints = parse_rate_limits_frame(&frame);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].status, QuotaStatus::Exhausted);
    }

    #[test]
    fn rate_limits_frame_empty_without_rate_limits_object() {
        let frame = serde_json::json!({"type": "codex.rate_limits"});
        assert!(parse_rate_limits_frame(&frame).is_empty());
    }

    #[test]
    fn error_frame_preserves_connection_limit_as_retryable() {
        let classify = |status: u16, _body: &str, _retry: Option<std::time::Duration>| {
            ProviderError::Request(format!("unexpected classify call ({status})"))
        };
        let frame = serde_json::json!({
            "type": "error",
            "status": 429,
            "error": {"code": "websocket_connection_limit_reached", "message": "too many sockets"},
        });
        let err = classify_error_frame(&frame, &classify);
        assert!(matches!(err, ProviderError::Unavailable(_)));
    }

    #[test]
    fn error_frame_defers_to_classifier_otherwise() {
        let classify = |status: u16, body: &str, _retry: Option<std::time::Duration>| {
            assert_eq!(status, 401);
            assert!(body.contains("token expired"));
            ProviderError::Auth("classified".to_string())
        };
        let frame = serde_json::json!({
            "type": "error",
            "status": 401,
            "error": {"message": "token expired"},
        });
        let err = classify_error_frame(&frame, &classify);
        assert!(matches!(err, ProviderError::Auth(_)));
    }

    #[test]
    fn statusless_backend_error_is_retryable_but_schema_error_is_not() {
        let classify = |status: u16, _body: &str, _retry: Option<std::time::Duration>| {
            ProviderError::Request(format!("unexpected classify call ({status})"))
        };
        let backend = serde_json::json!({
            "type": "error",
            "error": {
                "type": "server_error",
                "message": "An error occurred while processing your request. You can retry your request."
            }
        });
        assert!(matches!(
            classify_error_frame(&backend, &classify),
            ProviderError::Unavailable(_)
        ));

        let schema = serde_json::json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "invalid tool schema"
            }
        });
        assert!(matches!(
            classify_error_frame(&schema, &classify),
            ProviderError::Request(_)
        ));
    }
}
