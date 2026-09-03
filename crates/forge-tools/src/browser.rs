//! Browser tools: drive a real Chrome, and read its network traffic.
//!
//! Two tools rather than a dozen. Every tool's schema is in the system prompt of every turn, so a
//! `browser_open`/`browser_click`/`browser_type`/`browser_eval`/… fan-out would tax sessions that
//! never touch a browser. `browser` takes an `action`; `browser_network` is separate because
//! inspection is a genuinely different job from control and is the reason this exists at all.
//!
//! Both declare [`SideEffect::Network`]: a browser reaches the network, and `eval` runs arbitrary
//! script in a page that may be logged into the user's accounts.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use forge_browser::{BrowserSession, Filter, LaunchConfig};
use forge_types::SideEffect;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::{str_arg, Tool, ToolError};

/// Where persistent browser profiles live. A profile keeps cookies and logins between turns and
/// between sessions, which is what makes a login flow investigable at all.
fn profile_root() -> std::path::PathBuf {
    std::env::var_os("FORGE_BROWSER_PROFILE_ROOT")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs_data().map(|dir| dir.join("forge").join("browser")))
        .unwrap_or_else(|| std::path::PathBuf::from(".forge/browser"))
}

fn dirs_data() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return Some(std::path::PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
}

/// Live sessions, keyed by profile name, shared by both tools.
///
/// The browser has to outlive a single tool call: opening a page, clicking through a login, and
/// then asking what the page fetched are three separate calls against one browser. A tool object
/// is stateless and re-used, so the state lives here.
static SESSIONS: std::sync::OnceLock<Mutex<HashMap<String, Arc<BrowserSession>>>> =
    std::sync::OnceLock::new();

fn sessions() -> &'static Mutex<HashMap<String, Arc<BrowserSession>>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn profile_name(args: &Value) -> String {
    args.get("profile")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("default")
        .to_string()
}

/// Fetch the live session for a profile, or explain that nothing is open.
async fn require_session(profile: &str) -> Result<Arc<BrowserSession>, ToolError> {
    let map = sessions().lock().await;
    map.get(profile).cloned().ok_or_else(|| {
        ToolError::Failed(format!(
            "no browser is open for profile '{profile}' — call browser with action \"open\" first"
        ))
    })
}

pub struct BrowserTool;

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Drive a real Chrome browser: open a window, navigate, click, type, run JavaScript, read \
         the rendered HTML, screenshot, or read cookies. The browser is a genuine windowed Chrome \
         with a PERSISTENT profile, so a login you perform once survives later turns — use it for \
         pages behind authentication, pages built by JavaScript, and any flow you need to observe \
         rather than guess at. Pair it with `browser_network` to see every request the page makes. \
         Set headless=true only when nobody needs to watch or interact; sites that fingerprint \
         automation commonly block headless. Actions: open, navigate, click, type, eval, html, \
         screenshot, cookies, close."
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
                    "enum": ["open", "navigate", "click", "type", "eval", "html",
                             "screenshot", "cookies", "close"],
                    "description": "What to do."
                },
                "url": {"type": "string", "description": "For open/navigate."},
                "selector": {"type": "string", "description": "CSS selector, for click/type."},
                "text": {"type": "string", "description": "Text to type, for type."},
                "expression": {"type": "string", "description": "JavaScript, for eval."},
                "headless": {
                    "type": "boolean",
                    "description": "Open without a window. Default false — a real window is the \
                                    point, and headless is widely blocked."
                },
                "profile": {
                    "type": "string",
                    "description": "Named persistent profile. Default \"default\". Use a separate \
                                    name to keep logins for different accounts apart."
                },
                "max_chars": {"type": "integer", "description": "Cap for html output."}
            },
            "required": ["action"]
        })
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let action = str_arg(args, "action")?;
        let profile = profile_name(args);

        if action == "open" {
            return open(args, &profile).await;
        }
        if action == "close" {
            let mut map = sessions().lock().await;
            return Ok(match map.remove(&profile) {
                Some(_) => format!("closed the browser for profile '{profile}'"),
                None => format!("no browser was open for profile '{profile}'"),
            });
        }

        let session = require_session(&profile).await?;
        match action {
            "navigate" => {
                let url = str_arg(args, "url")?;
                let landed = session.navigate(url).await.map_err(message)?;
                Ok(format!("navigated to {landed}"))
            }
            "click" => {
                let selector = str_arg(args, "selector")?;
                session.click(selector).await.map_err(message)?;
                Ok(format!("clicked {selector}"))
            }
            "type" => {
                let selector = str_arg(args, "selector")?;
                let text = str_arg(args, "text")?;
                session.type_text(selector, text).await.map_err(message)?;
                Ok(format!("typed into {selector}"))
            }
            "eval" => {
                let expression = str_arg(args, "expression")?;
                let value = session.eval(expression).await.map_err(message)?;
                Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
            }
            "html" => {
                let max = args
                    .get("max_chars")
                    .and_then(Value::as_u64)
                    .unwrap_or(20_000) as usize;
                let html = session.html().await.map_err(message)?;
                Ok(forge_browser::truncate(&html, max))
            }
            "screenshot" => {
                let data = session.screenshot().await.map_err(message)?;
                // A base64 PNG in the transcript is tens of thousands of useless tokens. Write it
                // where the caller can look at it and hand back the path.
                let path = std::env::temp_dir().join(format!(
                    "forge-browser-{}-{}.png",
                    profile,
                    std::process::id()
                ));
                let bytes = base64_decode(&data)
                    .ok_or_else(|| ToolError::Failed("screenshot was not valid base64".into()))?;
                std::fs::write(&path, bytes)
                    .map_err(|e| ToolError::Failed(format!("write screenshot: {e}")))?;
                Ok(format!("screenshot written to {}", path.display()))
            }
            "cookies" => {
                let cookies = session.cookies().await.map_err(message)?;
                Ok(serde_json::to_string_pretty(&cookies).unwrap_or_else(|_| cookies.to_string()))
            }
            other => Err(ToolError::Failed(format!(
                "unknown browser action '{other}'"
            ))),
        }
    }
}

async fn open(args: &Value, profile: &str) -> Result<String, ToolError> {
    let headless = args
        .get("headless")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let dir = profile_root().join(profile);

    let mut map = sessions().lock().await;
    if !map.contains_key(profile) {
        // Prefer re-attaching: a browser the user already logged into by hand is worth far more
        // than a fresh one, and relaunching would discard the window they are looking at.
        let session = match BrowserSession::reattach(dir.clone()).await {
            Ok(session) => session,
            Err(_) => {
                let config = LaunchConfig::new(dir.clone()).headless(headless);
                BrowserSession::open(&config).await.map_err(message)?
            }
        };
        map.insert(profile.to_string(), Arc::new(session));
    }
    let session = map.get(profile).cloned().expect("just inserted");
    drop(map);

    let mut report = format!(
        "browser ready (profile '{profile}', {}, {})",
        if headless { "headless" } else { "windowed" },
        dir.display()
    );
    if let Some(url) = args.get("url").and_then(Value::as_str) {
        let landed = session.navigate(url).await.map_err(message)?;
        report.push_str(&format!("\nnavigated to {landed}"));
    }
    Ok(report)
}

pub struct BrowserNetworkTool;

#[async_trait]
impl Tool for BrowserNetworkTool {
    fn name(&self) -> &str {
        "browser_network"
    }

    fn description(&self) -> &str {
        "Read the browser's network traffic — the same data the DevTools Network tab shows, for \
         every request the page made: method, URL, status, request and response headers, request \
         body, timing, and failures. This sees traffic that injected JavaScript hooks cannot: \
         document loads, redirect chains and their Location headers, CORS preflights, sendBeacon, \
         EventSource, and anything issued before a hook could install. Use it to reverse-engineer \
         an API, find which request carries a token or cookie, or check what a page actually sent. \
         action \"list\" queries with filters, \"body\" fetches one response body by request id, \
         \"clear\" empties the log before a fresh interaction."
    }

    fn side_effect(&self) -> SideEffect {
        SideEffect::Network
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "body", "clear"]},
                "request_id": {"type": "string", "description": "For body — the id from list."},
                "url_contains": {"type": "string", "description": "Case-insensitive URL filter."},
                "method": {"type": "string", "description": "GET, POST, ..."},
                "resource_type": {
                    "type": "string",
                    "description": "Document, XHR, Fetch, Script, Preflight, ..."
                },
                "status_min": {"type": "integer"},
                "status_max": {"type": "integer"},
                "with_post_data": {
                    "type": "boolean",
                    "description": "Only requests that carried a body — the fast path to \
                                    \"what did this form submit\"."
                },
                "limit": {"type": "integer", "description": "Newest N matches. Default 50."},
                "profile": {"type": "string"}
            },
            "required": ["action"]
        })
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let action = str_arg(args, "action")?;
        let profile = profile_name(args);
        let session = require_session(&profile).await?;

        match action {
            "clear" => {
                session.clear_network();
                Ok("network log cleared".to_string())
            }
            "body" => {
                let id = str_arg(args, "request_id")?;
                session.response_body(id).await.map_err(message)
            }
            "list" => {
                let filter = build_filter(args);
                let hits = session.network(&filter);
                if hits.is_empty() {
                    return Ok(format!(
                        "no captured requests match (the log holds {} exchange(s))",
                        session.network_len()
                    ));
                }
                Ok(render(&hits, session.network_len()))
            }
            other => Err(ToolError::Failed(format!(
                "unknown browser_network action '{other}'"
            ))),
        }
    }
}

fn build_filter(args: &Value) -> Filter {
    let status_range = match (
        args.get("status_min").and_then(Value::as_u64),
        args.get("status_max").and_then(Value::as_u64),
    ) {
        (None, None) => None,
        (low, high) => Some((
            low.unwrap_or(0) as u16,
            high.unwrap_or(u16::MAX as u64) as u16,
        )),
    };
    Filter {
        url_contains: args
            .get("url_contains")
            .and_then(Value::as_str)
            .map(str::to_string),
        method: args
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string),
        resource_type: args
            .get("resource_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        status_range,
        with_post_data: args
            .get("with_post_data")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        limit: Some(args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize),
    }
}

/// Render matches compactly. Headers and bodies are the expensive part, so the list stays a
/// one-line-per-request index and the caller asks for what it wants by id.
fn render(hits: &[forge_browser::Exchange], total: usize) -> String {
    let mut out = format!("{} of {total} captured exchange(s):\n", hits.len());
    for exchange in hits {
        let status = match (&exchange.status, &exchange.failure) {
            (_, Some(failure)) => failure.clone(),
            (Some(status), _) => status.to_string(),
            (None, _) => "pending".to_string(),
        };
        out.push_str(&format!(
            "\n[{}] {} {} → {}",
            exchange.id, exchange.method, exchange.url, status
        ));
        if let Some(kind) = &exchange.resource_type {
            out.push_str(&format!(" ({kind})"));
        }
        if let Some(ms) = exchange.duration_ms {
            out.push_str(&format!(" {ms:.0}ms"));
        }
        if let Some(target) = exchange.redirect_target() {
            out.push_str(&format!("\n    → redirects to {target}"));
        }
        if let Some(body) = &exchange.post_data {
            out.push_str(&format!(
                "\n    request body: {}",
                forge_browser::truncate(body, 500)
            ));
        }
    }
    out.push_str("\n\nUse action \"body\" with a request id for a response body.");
    out
}

fn message(error: forge_browser::Error) -> ToolError {
    ToolError::Failed(format!("{error:#}"))
}

/// Minimal base64 decode for screenshot payloads — avoids a dependency for one call site.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = index as u8;
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte == b'=' || byte == b'\n' || byte == b'\r' {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 255 {
            return None;
        }
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_filter_can_be_open_ended_on_either_side() {
        // "everything that failed" is status_min=400 with no max; "everything before a redirect"
        // is status_max=299 with no min. Requiring both would make the common asks awkward.
        let only_min = build_filter(&json!({"action": "list", "status_min": 400}));
        assert_eq!(only_min.status_range, Some((400, u16::MAX)));

        let only_max = build_filter(&json!({"action": "list", "status_max": 299}));
        assert_eq!(only_max.status_range, Some((0, 299)));

        let neither = build_filter(&json!({"action": "list"}));
        assert_eq!(neither.status_range, None, "no range means no filtering");
    }

    #[test]
    fn list_defaults_to_the_newest_fifty() {
        let filter = build_filter(&json!({"action": "list"}));
        assert_eq!(filter.limit, Some(50));
        assert!(!filter.with_post_data);
    }

    #[test]
    fn the_rendered_index_leads_with_what_identifies_a_request() {
        let exchange = forge_browser::Exchange {
            id: "R7".into(),
            method: "POST".into(),
            url: "https://accounts.spotify.com/login/password".into(),
            resource_type: Some("XHR".into()),
            status: Some(401),
            status_text: None,
            mime_type: None,
            request_headers: vec![],
            response_headers: vec![],
            post_data: Some("username=a&password=b".into()),
            remote_address: None,
            duration_ms: Some(184.0),
            encoded_bytes: None,
            failure: None,
            finished: true,
            ..Default::default()
        };
        let rendered = render(std::slice::from_ref(&exchange), 12);
        assert!(rendered.contains("[R7] POST"), "{rendered}");
        assert!(rendered.contains("→ 401"), "{rendered}");
        assert!(rendered.contains("184ms"), "{rendered}");
        assert!(
            rendered.contains("username=a&password=b"),
            "the request body is the answer being looked for: {rendered}"
        );
        assert!(rendered.contains("1 of 12"), "{rendered}");
    }

    #[test]
    fn a_failure_replaces_the_status_rather_than_showing_a_blank() {
        let exchange = forge_browser::Exchange {
            id: "R1".into(),
            method: "GET".into(),
            url: "https://blocked.test/x".into(),
            resource_type: None,
            status: None,
            status_text: None,
            mime_type: None,
            request_headers: vec![],
            response_headers: vec![],
            post_data: None,
            remote_address: None,
            duration_ms: None,
            encoded_bytes: None,
            failure: Some("net::ERR_BLOCKED_BY_CLIENT".into()),
            finished: true,
            ..Default::default()
        };
        let rendered = render(std::slice::from_ref(&exchange), 1);
        assert!(rendered.contains("ERR_BLOCKED_BY_CLIENT"), "{rendered}");
    }

    #[test]
    fn base64_round_trips_a_png_header() {
        // The screenshot path writes real bytes to disk; a decoder that silently returns garbage
        // would produce an unopenable file and a tool result that claims success.
        let decoded = base64_decode("iVBORw0KGgo=").expect("valid base64");
        assert_eq!(&decoded[..4], &[0x89, b'P', b'N', b'G']);
        assert!(base64_decode("not valid!!").is_none());
    }

    #[test]
    fn both_tools_are_gated_as_network_egress() {
        assert_eq!(BrowserTool.side_effect(), SideEffect::Network);
        assert_eq!(BrowserNetworkTool.side_effect(), SideEffect::Network);
    }
}
