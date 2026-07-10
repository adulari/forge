//! WebSocket transport for `codex-oauth::` "v1"-tagged models — currently just
//! `gpt-5.6-luna` — which the ChatGPT backend serves ONLY over
//! `wss://chatgpt.com/backend-api/codex/responses`; the plain HTTPS POST path in
//! `codex_oauth.rs` 404s them ("Model not found"). Every other `codex-oauth::` model keeps
//! using that battle-tested HTTP path UNCHANGED — this module is a narrow, additive transport
//! gated by [`CODEX_WEBSOCKET_MODELS`], not a rewrite.
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

use crate::oauth_responses::{apply_sse_event, ResponseAccumulator, CONNECT_TIMEOUT, IDLE_TIMEOUT};
use crate::{EventSink, ModelResponse, ProviderError};

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
    classify(status, &error_obj.to_string(), None)
}

/// Build [`QuotaHint`]s from a `{"type":"codex.rate_limits","rate_limits":{...},...}` frame. The
/// `rate_limits` shape mirrors the HTTP path's `x-codex-*`/rollout `rate_limits` object (see
/// `cli_provider::codex_quota_from_rollout`): primary/secondary each with `used_percent`,
/// `window_minutes` (300→"five_hour", 10080→"weekly"), `resets_at`. Any window missing
/// `used_percent` is skipped — no hint rather than a wrong one.
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
        let resets = w.get("resets_at").and_then(|v| v.as_i64());
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
        let frame = rate_limits_frame(serde_json::json!({
            "primary": {"used_percent": 42.0, "window_minutes": 300, "resets_at": now + 3600},
            "secondary": {"used_percent": 10.0, "window_minutes": 10080, "resets_at": now + 86400},
        }));
        let hints = parse_rate_limits_frame(&frame);
        assert_eq!(hints.len(), 2);
        let five_hour = hints.iter().find(|h| h.window == "five_hour").unwrap();
        assert_eq!(five_hour.provider, "codex-oauth");
        assert_eq!(five_hour.fraction_used, Some(0.42));
        assert_eq!(five_hour.status, QuotaStatus::Ok);
        let weekly = hints.iter().find(|h| h.window == "weekly").unwrap();
        assert_eq!(weekly.fraction_used, Some(0.10));
        assert_eq!(weekly.status, QuotaStatus::Ok);
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
}
