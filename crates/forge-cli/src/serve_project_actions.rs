//! Project actions over the daemon: `[[actions]]` in `.forge/config.toml` run as terminals tagged
//! by action id (see [`crate::serve_terminal`]).
//!
//! Routes live under `/api/terminals/` so they need the same `terminal` scope as a shell:
//! - `GET  /api/terminals/actions?session=`          the session's actions with run state
//! - `GET  /api/terminals/actions/running`           every running action on the host
//! - `POST /api/terminals/actions/run`  `{session, action}`
//! - `POST /api/terminals/actions/stop` `{session, action}`
//!
//! The request never carries a command or a directory: both come from the project's config, which
//! only a trusted project contributes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_config::project_actions::{actions_for, ProjectAction};
use serde::Deserialize;
use serde_json::json;

use crate::serve::{err_response, json_response, DaemonState};

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/terminals/actions"), get(list))
        .route(
            &format!("{base}/api/terminals/actions/running"),
            get(running),
        )
        .route(&format!("{base}/api/terminals/actions/run"), post(run))
        .route(&format!("{base}/api/terminals/actions/stop"), post(stop))
}

#[derive(Deserialize)]
struct SessionQuery {
    #[serde(default)]
    session: String,
}

#[derive(Deserialize)]
struct ActionBody {
    session: String,
    action: String,
}

/// Where a session's shells run: its worktree, else its directory.
async fn session_roots(state: &DaemonState, session: &str) -> Option<Vec<PathBuf>> {
    let handle = state.registry.get(session).await?;
    let mut roots = Vec::new();
    for dir in [
        handle.worktree.clone().unwrap_or_default(),
        handle.cwd.clone(),
    ] {
        let path = PathBuf::from(dir);
        if path.is_dir() && !roots.contains(&path) {
            roots.push(path);
        }
    }
    Some(roots)
}

/// The first of `roots` that declares actions, and the actions.
fn declared(roots: &[PathBuf]) -> (PathBuf, Vec<ProjectAction>) {
    for root in roots {
        let actions = actions_for(root);
        if !actions.is_empty() {
            return (root.clone(), actions);
        }
    }
    (roots.first().cloned().unwrap_or_default(), Vec::new())
}

/// True when a config declares actions that a missing trust decision is hiding.
fn hidden_by_trust(roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| {
        !forge_config::project_trust::is_trusted(root)
            && std::fs::read_to_string(root.join(".forge/config.toml"))
                .ok()
                .and_then(|text| text.parse::<toml::Table>().ok())
                .is_some_and(|table| table.contains_key("actions"))
    })
}

/// The action's working directory: inside the root, existing, symlinks resolved.
fn action_cwd(root: &Path, action: &ProjectAction) -> Result<PathBuf, String> {
    let base = root
        .canonicalize()
        .map_err(|e| format!("worktree unavailable: {e}"))?;
    let Some(sub) = action.cwd.as_deref() else {
        return Ok(base);
    };
    let joined = base
        .join(sub)
        .canonicalize()
        .map_err(|_| format!("the action's cwd \"{sub}\" does not exist"))?;
    if joined.starts_with(&base) && joined.is_dir() {
        Ok(joined)
    } else {
        Err(format!("the action's cwd \"{sub}\" is outside the project"))
    }
}

async fn list(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<SessionQuery>,
) -> Response {
    let Some(roots) = session_roots(&state, &query.session).await else {
        return err_response(StatusCode::NOT_FOUND, "session not found");
    };
    let runs = state.terminals.action_runs(Some(&query.session)).await;
    let (_, actions) = declared(&roots);
    let rows: Vec<_> = actions
        .iter()
        .map(|action| {
            let run = runs.iter().find(|run| run.action == action.id);
            json!({
                "id": action.id,
                "name": action.name,
                "command": action.command,
                "cwd": action.cwd,
                "icon": action.icon,
                "terminal_id": run.map(|run| run.terminal_id.clone()),
                "running": run.is_some_and(|run| run.running),
                "exit_code": run.and_then(|run| run.exit_code),
            })
        })
        .collect();
    json_response(&json!({
        "actions": rows,
        "untrusted": rows.is_empty() && hidden_by_trust(&roots),
    }))
}

async fn running(State(state): State<Arc<DaemonState>>) -> Response {
    let runs = state.terminals.action_runs(None).await;
    let rows: Vec<_> = runs
        .into_iter()
        .filter(|run| run.running)
        .map(|run| json!({"session": run.session, "action": run.action}))
        .collect();
    json_response(&json!({ "running": rows }))
}

async fn run(State(state): State<Arc<DaemonState>>, Json(body): Json<ActionBody>) -> Response {
    let Some(roots) = session_roots(&state, &body.session).await else {
        return err_response(StatusCode::NOT_FOUND, "session not found");
    };
    let (root, actions) = declared(&roots);
    let Some(action) = actions.iter().find(|action| action.id == body.action) else {
        return err_response(StatusCode::NOT_FOUND, "no such action in this project");
    };
    let cwd = match action_cwd(&root, action) {
        Ok(cwd) => cwd,
        Err(message) => return err_response(StatusCode::BAD_REQUEST, &message),
    };
    match state
        .terminals
        .start_action(
            &body.session,
            &action.id,
            cwd.to_string_lossy().into_owned(),
            &action.command,
            crate::host_caps::terminal_limit(),
        )
        .await
    {
        Ok(()) => json_response(&json!({"ok": true, "terminal_id": format!("act-{}", action.id)})),
        Err(message) => err_response(StatusCode::CONFLICT, &message),
    }
}

async fn stop(State(state): State<Arc<DaemonState>>, Json(body): Json<ActionBody>) -> Response {
    if state.registry.get(&body.session).await.is_none() {
        return err_response(StatusCode::NOT_FOUND, "session not found");
    }
    let stopped = state
        .terminals
        .stop_action(&body.session, &body.action)
        .await;
    json_response(&json!({"ok": true, "stopped": stopped}))
}
