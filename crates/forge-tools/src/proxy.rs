//! Proxy tools: see a phone's traffic, change it, and replay it.
//!
//! The mobile counterpart to [`crate::browser`]. Same two-tool split and the same reasoning:
//! `proxy` controls, `proxy_network` inspects, because inspection is a different job from control
//! and every tool schema costs prompt tokens in every turn, browser or not.
//!
//! Both declare [`SideEffect::Network`]. That understates it slightly — a running proxy is
//! reachable by anything on the LAN and sees plaintext of every request the device makes — which
//! is why it starts only on an explicit call and reports what it exposed.

use std::sync::Arc;

use async_trait::async_trait;
use forge_proxy::{Filter, InterceptRules, Proxy};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::{SideEffect, Tool, ToolError};

/// The proxy outlives a single tool call: start it, have someone use the phone, then ask what it
/// saw. A tool object is stateless and re-used, so the state lives here.
static PROXY: std::sync::OnceLock<Mutex<Option<Arc<Mutex<Proxy>>>>> = std::sync::OnceLock::new();

fn slot() -> &'static Mutex<Option<Arc<Mutex<Proxy>>>> {
    PROXY.get_or_init(|| Mutex::new(None))
}

async fn require_proxy() -> Result<Arc<Mutex<Proxy>>, ToolError> {
    slot().lock().await.clone().ok_or_else(|| {
        ToolError::Failed(
            "no proxy is running — call proxy with action \"start\" first".to_string(),
        )
    })
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn capture_dir() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".local/share/forge/proxy")
}

pub struct ProxyTool;

#[async_trait]
impl Tool for ProxyTool {
    fn name(&self) -> &str {
        "proxy"
    }

    fn description(&self) -> &str {
        "Intercept the network traffic of a PHONE or any other device, via mitmproxy. This is the \
         only way to see what a native mobile app sends: there is no DevTools to open and the app \
         will not tell you. \"start\" runs the proxy on this machine's LAN address and returns the \
         exact steps to point a device at it (proxy settings, and the CA certificate the device \
         must trust before HTTPS is readable). Then use `proxy_network` to read what it caught. \
         \"intercept\" blocks endpoints, rewrites request/response headers and bodies, or serves a \
         canned response, so you can ask what the app does when an API fails, returns something \
         else, or disappears — rules apply to the next request with no restart. \"replay\" \
         re-issues a captured request from this machine with your changes, turning a one-shot \
         mobile request into something you can iterate on. Note: certificate pinning defeats \
         interception for apps that use it, and on Android 7+ a user-installed CA is not trusted \
         by apps unless the app opts in. Actions: start, status, intercept, intercept_clear, \
         replay, stop."
    }

    fn side_effect(&self) -> SideEffect {
        SideEffect::Network
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["start", "status", "intercept", "intercept_clear", "replay", "stop"],
                    "description": "What to do."
                },
                "port": {"type": "integer", "description": "For start. Default 8080."},
                "request_id": {"type": "string", "description": "For replay — the id from proxy_network list."},
                "method": {"type": "string", "description": "For replay: override the HTTP method."},
                "headers": {
                    "type": "object",
                    "additionalProperties": {"type": "string"},
                    "description": "For replay: headers added to (or overriding) the captured ones."
                },
                "body": {"type": "string", "description": "For replay: override the request body."},
                "block": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "For intercept: URL substrings whose requests are refused."
                },
                "set_request_headers": {
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "For intercept: [{url_contains, headers:{name:value}}] applied to matching requests."
                },
                "set_response_headers": {
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "For intercept: [{url_contains, headers:{name:value}}] applied to matching responses."
                },
                "replace_request_body": {
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "For intercept: [{url_contains, body}] replacing matching request bodies."
                },
                "stub_response": {
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "For intercept: [{url_contains, status, body, headers}] served instead of the real response. The real one is still captured."
                }
            },
            "required": ["action"]
        })
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let action = str_arg(args, "action").unwrap_or_default();
        match action.as_str() {
            "start" => {
                let port = args.get("port").and_then(Value::as_u64).map(|p| p as u16);
                let proxy = Proxy::start(port, &capture_dir())
                    .await
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))?;
                let status = proxy.status();
                let host = forge_proxy::lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
                let steps = forge_proxy::setup_instructions(&host, status.port);
                *slot().lock().await = Some(Arc::new(Mutex::new(proxy)));
                Ok(format!(
                    "proxy listening on {} (all interfaces — anything on this LAN can use it \
                     while it runs)\n\n{steps}",
                    status.listening_on
                ))
            }
            "status" => {
                let proxy = require_proxy().await?;
                let status = proxy.lock().await.status();
                Ok(format!(
                    "listening on {} · {} flows captured · {}\ncapture: {}",
                    status.listening_on,
                    status.captured,
                    status.rules,
                    status.capture_path.display()
                ))
            }
            "intercept" => {
                let rules = parse_rules(args)?;
                let described = rules.describe();
                let proxy = require_proxy().await?;
                proxy
                    .lock()
                    .await
                    .set_rules(rules)
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))?;
                Ok(format!("interception active: {described}"))
            }
            "intercept_clear" => {
                let proxy = require_proxy().await?;
                proxy
                    .lock()
                    .await
                    .set_rules(InterceptRules::default())
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))?;
                Ok("interception cleared — observing only".to_string())
            }
            "replay" => {
                let id = str_arg(args, "request_id").ok_or_else(|| {
                    ToolError::Failed("replay needs request_id (from proxy_network list)".into())
                })?;
                let headers = args.get("headers").and_then(Value::as_object).map(|map| {
                    map.iter()
                        .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                        .collect::<std::collections::BTreeMap<_, _>>()
                });
                let proxy = require_proxy().await?;
                let guard = proxy.lock().await;
                guard
                    .replay(
                        &id,
                        str_arg(args, "method").as_deref(),
                        headers.as_ref(),
                        str_arg(args, "body").as_deref(),
                    )
                    .await
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))
            }
            "stop" => {
                let taken = slot().lock().await.take();
                match taken {
                    // Dropping the last handle kills mitmdump; the capture file stays so a HAR can
                    // still be exported afterwards.
                    Some(_) => Ok("proxy stopped (the capture is kept)".to_string()),
                    None => Ok("no proxy was running".to_string()),
                }
            }
            other => Err(ToolError::Failed(format!("unknown proxy action: {other}"))),
        }
    }
}

fn parse_rules(args: &Value) -> Result<InterceptRules, ToolError> {
    let field = |key: &str| args.get(key).cloned().unwrap_or(Value::Array(vec![]));
    let rules = InterceptRules {
        block: serde_json::from_value(field("block")).unwrap_or_default(),
        set_request_headers: serde_json::from_value(field("set_request_headers"))
            .unwrap_or_default(),
        replace_request_body: serde_json::from_value(field("replace_request_body"))
            .unwrap_or_default(),
        set_response_headers: serde_json::from_value(field("set_response_headers"))
            .unwrap_or_default(),
        stub_response: serde_json::from_value(field("stub_response")).unwrap_or_default(),
    };
    if rules.is_empty() {
        return Err(ToolError::Failed(
            "intercept needs at least one rule (block, set_request_headers, \
             replace_request_body, set_response_headers, or stub_response); use intercept_clear \
             to go back to observing"
                .into(),
        ));
    }
    Ok(rules)
}

pub struct ProxyNetworkTool;

#[async_trait]
impl Tool for ProxyNetworkTool {
    fn name(&self) -> &str {
        "proxy_network"
    }

    fn description(&self) -> &str {
        "Read the traffic the proxy captured from a phone or other device — every request it made, \
         with method, URL, status, full request and response headers, and bodies. \"list\" is the \
         index (filter by url_contains, method, status, host, or only requests that carried a \
         body — the fast path to \"what did the app POST\"); \"body\" opens one flow whole by its \
         id; \"har\" exports everything to a HAR file that opens in browser devtools, Charles, or \
         Proxyman; \"clear\" empties the capture so the next interaction starts clean. Requires \
         `proxy` with action \"start\" first."
    }

    fn side_effect(&self) -> SideEffect {
        SideEffect::Network
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "body", "har", "clear"]},
                "request_id": {"type": "string", "description": "For body — the id from list (a prefix is fine)."},
                "path": {"type": "string", "description": "For har — where to write it. Default a temp path."},
                "url_contains": {"type": "string", "description": "Case-insensitive URL filter."},
                "method": {"type": "string", "description": "GET, POST, ..."},
                "status": {"type": "integer", "description": "Exact response status."},
                "host": {"type": "string", "description": "Case-insensitive host filter."},
                "with_body": {"type": "boolean", "description": "Only requests that carried a body."},
                "failed": {"type": "boolean", "description": "Only requests that failed or were blocked."},
                "limit": {"type": "integer", "description": "Max rows for list. Default 50, newest last."}
            },
            "required": ["action"]
        })
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let action = str_arg(args, "action").unwrap_or_default();
        let proxy = require_proxy().await?;
        let guard = proxy.lock().await;
        match action.as_str() {
            "list" => {
                let filter = Filter {
                    url_contains: str_arg(args, "url_contains"),
                    method: str_arg(args, "method"),
                    status: args.get("status").and_then(Value::as_u64).map(|s| s as u16),
                    host: str_arg(args, "host"),
                    with_body: args
                        .get("with_body")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    failed: args.get("failed").and_then(Value::as_bool).unwrap_or(false),
                };
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 500) as usize;
                let flows = guard
                    .flows(&filter, limit)
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))?;
                if flows.is_empty() {
                    return Ok(
                        "no matching flows. If the device is configured but nothing is \
                               arriving: the CA may not be installed, or the app may pin its \
                               certificate."
                            .to_string(),
                    );
                }
                Ok(flows
                    .iter()
                    .map(forge_proxy::Flow::summary)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "body" => {
                let id = str_arg(args, "request_id")
                    .ok_or_else(|| ToolError::Failed("body needs request_id".into()))?;
                let flow = guard
                    .flow(&id)
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))?;
                Ok(render_flow(&flow))
            }
            "har" => {
                let path = str_arg(args, "path")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::temp_dir().join("forge-proxy.har"));
                let count = guard
                    .har(&path)
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))?;
                Ok(format!("wrote {count} flows to {}", path.display()))
            }
            "clear" => {
                guard
                    .clear()
                    .map_err(|error| ToolError::Failed(format!("{error:#}")))?;
                Ok("capture cleared".to_string())
            }
            other => Err(ToolError::Failed(format!(
                "unknown proxy_network action: {other}"
            ))),
        }
    }
}

fn render_flow(flow: &forge_proxy::Flow) -> String {
    let headers = |map: &std::collections::BTreeMap<String, String>| {
        map.iter()
            .map(|(k, v)| format!("  {k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let clip = |clipped: bool, bytes: usize| {
        if clipped {
            format!(" (clipped from {bytes} bytes)")
        } else {
            String::new()
        }
    };
    format!(
        "{} {}\n\nREQUEST HEADERS\n{}\n\nREQUEST BODY{}\n{}\n\nRESPONSE {}\n{}\n\nRESPONSE BODY{}\n{}",
        flow.method,
        flow.url,
        headers(&flow.request_headers),
        clip(flow.request_body_clipped, flow.request_body_bytes),
        forge_proxy::truncate(&flow.request_body, forge_proxy::MAX_BODY_CHARS),
        flow.status
            .map(|s| s.to_string())
            .or_else(|| flow.error.clone())
            .unwrap_or_else(|| "—".into()),
        headers(&flow.response_headers),
        clip(flow.response_body_clipped, flow.response_body_bytes),
        forge_proxy::truncate(&flow.response_body, forge_proxy::MAX_BODY_CHARS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both tools must refuse clearly before a proxy exists. "no proxy is running" and the fix in
    /// one line beats a lookup failure the model has to interpret.
    #[tokio::test]
    async fn the_tools_say_what_to_do_before_a_proxy_is_started() {
        let error = ProxyNetworkTool
            .run(&json!({"action": "list"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("no proxy is running"), "{error}");
        assert!(error.contains("start"), "{error}");
    }

    /// An empty intercept call is almost always a mistake — the model meant to clear. Saying so,
    /// and naming the action that does it, beats silently installing nothing.
    #[test]
    fn an_intercept_with_no_rules_is_refused_and_points_at_intercept_clear() {
        let error = parse_rules(&json!({"action": "intercept"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("intercept_clear"), "{error}");
    }

    #[test]
    fn rules_parse_from_the_tool_schema_shape() {
        let rules = parse_rules(&json!({
            "block": ["/telemetry"],
            "stub_response": [{"url_contains": "/v1/me", "status": 402, "body": "{}"}],
            "set_request_headers": [{"url_contains": "/api", "headers": {"x-debug": "1"}}]
        }))
        .unwrap();
        assert_eq!(rules.block, vec!["/telemetry".to_string()]);
        assert_eq!(rules.stub_response[0].status, 402);
        assert_eq!(
            rules.set_request_headers[0].headers.get("x-debug"),
            Some(&"1".to_string())
        );
    }

    /// `status` defaults to 200 when omitted, so a stub written without one still serves a sane
    /// response rather than failing to parse.
    #[test]
    fn a_stub_without_a_status_defaults_to_200() {
        let rules =
            parse_rules(&json!({"stub_response": [{"url_contains": "/x", "body": "hi"}]})).unwrap();
        assert_eq!(rules.stub_response[0].status, 200);
    }
}
