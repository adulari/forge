//! The daemon half of the control toolset (`forge_core::control`): the executor that runs a tool
//! call, the audit trail, and the routes the settings pages and the MCP bridges use.
//!
//! Routes (token-scoped under `{base}`):
//! - `POST /api/control/call`            `{tool, args, me}` run one tool, answer `{ok, text}`
//! - `GET  /api/control`                 the tool catalog with on/off state and the recent audit
//! - `POST /api/control/tools/{name}`    `{enabled}` switch one tool on or off
//!
//! The executor reuses the daemon's own routes through an in-process request instead of
//! duplicating their logic, so a session created by a model is exactly a session created from
//! the app. Permission gating already happened in the caller's session (or bridge); this side
//! refuses switched-off tools and writes one audit line per call.

use std::sync::{Arc, RwLock, Weak};

use axum::extract::{Json, Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_core::control::{self, ControlHost};
use serde::Deserialize;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use crate::serve::{err_response, json_response, DaemonState};

static STATE: RwLock<Option<Weak<DaemonState>>> = RwLock::new(None);

/// Remember the daemon's state so sessions it hosts (and the mention expander) can reach it
/// without an HTTP hop.
pub(crate) fn install(state: &Arc<DaemonState>) {
    *STATE.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(state));
}

pub(crate) fn daemon_state() -> Option<Arc<DaemonState>> {
    STATE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(Weak::upgrade)
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/control"), get(overview))
        .route(&format!("{base}/api/control/call"), post(call))
        .route(
            &format!("{base}/api/control/tools/{{name}}"),
            post(set_tool),
        )
}

const AUDIT_ROUTE_PREFIX: &str = "control/";
const MAX_RESULT_CHARS: usize = 12_000;

fn bounded(text: String) -> String {
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text;
    }
    let cut: String = text.chars().take(MAX_RESULT_CHARS).collect();
    format!("{cut}\n[result cut at {MAX_RESULT_CHARS} characters]")
}

fn audit(tool: &str, me: &str, args: &Value, outcome: &str, detail: &str) {
    let summary = args.to_string();
    let summary: String = summary.chars().take(300).collect();
    let detail = format!(
        "{summary} -> {}",
        detail.chars().take(160).collect::<String>()
    );
    crate::serve_access::access().record(crate::serve_access::AuditEntry {
        client: "control",
        label: "control tool",
        route: format!("{AUDIT_ROUTE_PREFIX}{tool}"),
        session: (!me.is_empty()).then(|| me.to_string()),
        decision: outcome,
        detail: Some(&detail),
        status: None,
        ip: None,
    });
}

/// The `limit` newest control audit lines, newest first.
pub(crate) fn recent_audit(limit: usize) -> Vec<Value> {
    let Some(path) = forge_config::data_dir().map(|d| d.join("remote-audit.jsonl")) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| {
            v["route"]
                .as_str()
                .is_some_and(|r| r.starts_with(AUDIT_ROUTE_PREFIX))
        })
        .take(limit)
        .collect()
}

/// An in-process request to one of the daemon's own routes.
async fn api(
    state: &Arc<DaemonState>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let uri = format!("{}{path}", state.base);
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    let body = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            axum::body::Body::from(body.to_string())
        }
        None => axum::body::Body::empty(),
    };
    let request = builder.body(body).map_err(|e| e.to_string())?;
    let response = crate::serve::daemon_routes(state.clone())
        .oneshot(request)
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 << 20)
        .await
        .map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    if status.is_success() {
        Ok(value)
    } else {
        Err(value["error"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("daemon answered {status}")))
    }
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

async fn session_id(state: &Arc<DaemonState>, address: &str) -> Result<String, String> {
    let store = state.store.clone();
    let address = address.to_string();
    tokio::task::spawn_blocking(move || {
        crate::mentions::resolve_session(&store, &address).map(|s| s.id)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn created(value: &Value) -> String {
    let id = value["id"].as_str().unwrap_or("?");
    let cwd = value["cwd"].as_str().unwrap_or("");
    let worktree = value["worktree"]
        .as_str()
        .map(|w| format!(", worktree {w}"))
        .unwrap_or_default();
    format!("created session {id} in {cwd}{worktree}")
}

/// Run one control tool. `Err` text is what the calling model reads.
pub(crate) async fn execute(
    state: &Arc<DaemonState>,
    me: &str,
    tool: &str,
    args: &Value,
) -> Result<String, String> {
    if forge_core::control::side_effect_of(tool).is_none() {
        return Err(format!("unknown control tool {tool}"));
    }
    if crate::control_settings::disabled()
        .iter()
        .any(|n| n == tool)
    {
        audit(tool, me, args, "refused", "switched off");
        return Err(format!(
            "{tool} is switched off in Settings > Control tools"
        ));
    }
    let outcome = run(state, me, tool, args).await.map(bounded);
    match &outcome {
        Ok(text) => audit(tool, me, args, "allow", text),
        Err(text) => audit(tool, me, args, "error", text),
    }
    outcome
}

async fn run(
    state: &Arc<DaemonState>,
    me: &str,
    tool: &str,
    args: &Value,
) -> Result<String, String> {
    match tool {
        "list_projects" => Ok(pretty(&api(state, "GET", "/api/projects", None).await?)),
        "create_session" | "create_worktree" => {
            let worktree = tool == "create_worktree"
                || args
                    .get("worktree")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            let mut body = json!({"worktree": worktree});
            for (from, to) in [
                ("project", "cwd"),
                ("prompt", "prompt"),
                ("title", "title"),
                ("model", "model"),
            ] {
                if let Some(value) = text_arg(args, from) {
                    body[to] = json!(value);
                }
            }
            Ok(created(
                &api(state, "POST", "/api/sessions", Some(body)).await?,
            ))
        }
        "read_session" => {
            let address = text_arg(args, "session").ok_or("read_session needs `session`")?;
            let store = state.store.clone();
            let address = address.to_string();
            tokio::task::spawn_blocking(move || {
                crate::mentions::read_session_digest(&store, &address)
            })
            .await
            .map_err(|e| e.to_string())?
        }
        "send_to_session" => {
            let target = text_arg(args, "session").ok_or("send_to_session needs `session`")?;
            let text = text_arg(args, "message").ok_or("send_to_session needs `message`")?;
            let reply = api(
                state,
                "POST",
                "/api/peers/message",
                Some(json!({
                    "target": target,
                    "text": text,
                    "mode": text_arg(args, "mode").unwrap_or("follow_up"),
                    "sender_id": (!me.is_empty()).then_some(me),
                })),
            )
            .await?;
            Ok(format!("message sent: {}", reply))
        }
        "read_terminal" => {
            let target = match text_arg(args, "session") {
                Some(address) => session_id(state, address).await?,
                None if !me.is_empty() => me.to_string(),
                None => return Err("read_terminal needs `session`".into()),
            };
            let lines = args
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(200)
                .clamp(1, 1000) as usize;
            match state
                .terminals
                .scrollback(&target, text_arg(args, "terminal"))
                .await
            {
                Some((terminal, bytes)) => {
                    let text = crate::mentions::terminal_text(&bytes, lines);
                    Ok(if text.is_empty() {
                        format!("terminal {terminal} has printed nothing yet")
                    } else {
                        format!("terminal {terminal}, last {lines} lines:\n{text}")
                    })
                }
                None => Err("that session has no open terminal".into()),
            }
        }
        "merge_worktree" => {
            let address = text_arg(args, "session").ok_or("merge_worktree needs `session`")?;
            let id = session_id(state, address).await?;
            let merged = api(
                state,
                "POST",
                &format!("/api/sessions/{id}/merge"),
                Some(json!({})),
            )
            .await?;
            Ok(format!(
                "merged branch {}",
                merged["branch"].as_str().unwrap_or("?")
            ))
        }
        "add_schedule" => {
            let mut body =
                json!({"task": text_arg(args, "task").ok_or("add_schedule needs `task`")?});
            for (from, to) in [
                ("project", "cwd"),
                ("every", "every"),
                ("at", "at"),
                ("cron", "cron"),
                ("model", "model"),
            ] {
                if let Some(value) = text_arg(args, from) {
                    body[to] = json!(value);
                }
            }
            let row = api(state, "POST", "/api/schedules", Some(body)).await?;
            Ok(format!(
                "schedule {} added",
                row["id"].as_str().unwrap_or("?")
            ))
        }
        "remove_schedule" => {
            let id = text_arg(args, "id").ok_or("remove_schedule needs `id`")?;
            api(
                state,
                "POST",
                &format!("/api/schedules/{id}/delete"),
                Some(json!({})),
            )
            .await?;
            Ok(format!("schedule {id} removed"))
        }
        "add_mcp_server" => {
            let mut body = json!({
                "name": text_arg(args, "name").ok_or("add_mcp_server needs `name`")?,
                "transport": text_arg(args, "transport").ok_or("add_mcp_server needs `transport`")?,
                "args": args.get("args").cloned().unwrap_or_else(|| json!([])),
            });
            for key in ["command", "url"] {
                if let Some(value) = text_arg(args, key) {
                    body[key] = json!(value);
                }
            }
            api(state, "POST", "/api/mcp", Some(body)).await?;
            Ok("MCP server added; probe it with probe_mcp_server".into())
        }
        "probe_mcp_server" => {
            let name = text_arg(args, "name").ok_or("probe_mcp_server needs `name`")?;
            Ok(pretty(
                &api(state, "POST", "/api/mcp/probe", Some(json!({"name": name}))).await?,
            ))
        }
        "get_usage" => Ok(pretty(&api(state, "GET", "/api/usage", None).await?)),
        "notify_user" => {
            let session = if me.is_empty() { "control" } else { me };
            let (text, ok) =
                forge_core::push_notify::run(args, session, forge_types::PermissionMode::Bypass)
                    .await;
            if ok {
                Ok(text)
            } else {
                Err(text)
            }
        }
        other => Err(format!("unknown control tool {other}")),
    }
}

/// The host a daemon-hosted session uses: straight into the executor, no HTTP hop.
pub(crate) struct DaemonControl;

#[async_trait::async_trait]
impl ControlHost for DaemonControl {
    fn disabled(&self) -> Vec<String> {
        crate::control_settings::disabled()
    }

    async fn call(&self, me: &str, tool: &str, args: &Value) -> Result<String, String> {
        let state = daemon_state().ok_or("the daemon is not running in this process")?;
        execute(&state, me, tool, args).await
    }
}

#[derive(Deserialize)]
struct CallRequest {
    tool: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    me: String,
}

async fn call(State(state): State<Arc<DaemonState>>, Json(req): Json<CallRequest>) -> Response {
    if let Err(error) = control::validate(&req.tool, &req.args) {
        return json_response(&json!({"ok": false, "text": error}));
    }
    match execute(&state, &req.me, &req.tool, &req.args).await {
        Ok(text) => json_response(&json!({"ok": true, "text": text})),
        Err(text) => json_response(&json!({"ok": false, "text": text})),
    }
}

async fn overview() -> Response {
    let disabled = crate::control_settings::disabled();
    json_response(&json!({
        "tools": control::catalog(&disabled),
        "audit": recent_audit(40),
    }))
}

#[derive(Deserialize)]
struct ToggleRequest {
    enabled: bool,
}

async fn set_tool(AxumPath(name): AxumPath<String>, Json(req): Json<ToggleRequest>) -> Response {
    match crate::control_settings::set_enabled(&name, req.enabled) {
        Ok(disabled) => json_response(&json!({"tools": control::catalog(&disabled)})),
        Err(error) => err_response(StatusCode::BAD_REQUEST, &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::tests::FORGE_DB_LOCK;
    use crate::serve::SessionRegistry;

    async fn daemon(dir: &std::path::Path) -> Arc<DaemonState> {
        std::env::set_var("FORGE_DB", dir.join("control.db"));
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        let store = Arc::new(crate::open_store().unwrap());
        let state = Arc::new(DaemonState {
            registry: Arc::new(SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store,
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        install(&state);
        state
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_model_can_create_a_session_read_it_and_message_it() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let state = daemon(dir.path()).await;

        let made = execute(
            &state,
            "",
            "create_session",
            &json!({"project": dir.path(), "prompt": "mock:write please", "title": "worker bee"}),
        )
        .await
        .unwrap();
        assert!(made.starts_with("created session "), "{made}");
        let id = made.split_whitespace().nth(2).unwrap().to_string();

        let mut digest = String::new();
        for _ in 0..100 {
            digest = execute(
                &state,
                "",
                "read_session",
                &json!({"session": "worker bee"}),
            )
            .await
            .unwrap_or_default();
            if digest.contains("[user] mock:write please") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(digest.contains("[user] mock:write please"), "{digest}");
        assert!(digest.contains(&id[..8]));

        let sent = execute(
            &state,
            "",
            "send_to_session",
            &json!({"session": id, "message": "status?"}),
        )
        .await
        .unwrap();
        assert!(sent.starts_with("message sent"), "{sent}");

        let err = execute(&state, "", "read_session", &json!({"session": "nobody"}))
            .await
            .unwrap_err();
        assert!(err.contains("nobody"), "{err}");
        assert!(execute(&state, "", "get_usage", &json!({})).await.is_ok());
        assert!(execute(&state, "", "list_projects", &json!({}))
            .await
            .is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_terminal_that_is_not_open_is_reported_and_a_switched_off_tool_is_refused() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let state = daemon(dir.path()).await;

        let none = execute(&state, "some-session", "read_terminal", &json!({}))
            .await
            .unwrap_err();
        assert!(none.contains("no open terminal"), "{none}");

        crate::control_settings::set_enabled("get_usage", false).unwrap();
        let off = execute(&state, "", "get_usage", &json!({}))
            .await
            .unwrap_err();
        assert!(off.contains("switched off"), "{off}");
        crate::control_settings::set_enabled("get_usage", true).unwrap();
        assert!(execute(&state, "", "get_usage", &json!({})).await.is_ok());
        assert!(execute(&state, "", "no_such_tool", &json!({}))
            .await
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_http_host_runs_a_tool_and_the_overview_lists_every_tool() {
        use forge_core::control::ControlHost;
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let state = daemon(dir.path()).await;
        let app = crate::serve::daemon_routes(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let host =
            crate::control_http::HttpControl::at(format!("http://127.0.0.1:{port}"), "tok".into());
        // The test router's base is "/tok", so the token segment is the base.
        let usage = host.call("", "get_usage", &json!({})).await;
        assert!(usage.is_ok(), "{usage:?}");
        let refused = host.call("", "read_session", &json!({})).await.unwrap_err();
        assert!(refused.contains("needs `session`"), "{refused}");

        let overview = reqwest::get(format!("http://127.0.0.1:{port}/tok/api/control"))
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(
            overview["tools"].as_array().unwrap().len(),
            control::catalog(&[]).len()
        );
    }
}
