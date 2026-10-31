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

type Refusal = (StatusCode, String);

/// Start `action_id` on the session from the project's config.
async fn start(state: &DaemonState, session: &str, action_id: &str) -> Result<String, Refusal> {
    let Some(roots) = session_roots(state, session).await else {
        return Err((StatusCode::NOT_FOUND, "session not found".into()));
    };
    let (root, actions) = declared(&roots);
    let Some(action) = actions.iter().find(|action| action.id == action_id) else {
        return Err((
            StatusCode::NOT_FOUND,
            "no such action in this project".into(),
        ));
    };
    let cwd = action_cwd(&root, action).map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    state
        .terminals
        .start_action(
            session,
            &action.id,
            cwd.to_string_lossy().into_owned(),
            &action.command,
            crate::host_caps::terminal_limit(),
        )
        .await
        .map_err(|message| (StatusCode::CONFLICT, message))?;
    Ok(format!("act-{}", action.id))
}

/// Stop the action if it is there and start it again.
pub(crate) async fn restart(
    state: &DaemonState,
    session: &str,
    action_id: &str,
) -> Result<(), Refusal> {
    state.terminals.stop_action(session, action_id).await;
    start(state, session, action_id).await.map(|_| ())
}

async fn run(State(state): State<Arc<DaemonState>>, Json(body): Json<ActionBody>) -> Response {
    match start(&state, &body.session, &body.action).await {
        Ok(terminal_id) => json_response(&json!({"ok": true, "terminal_id": terminal_id})),
        Err((status, message)) => err_response(status, &message),
    }
}

/// Stop needs no loaded session: an action must stay stoppable after its session was unloaded.
async fn stop(State(state): State<Arc<DaemonState>>, Json(body): Json<ActionBody>) -> Response {
    let stopped = state
        .terminals
        .stop_action(&body.session, &body.action)
        .await;
    json_response(&json!({"ok": true, "stopped": stopped}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_terminal::TerminalRegistry;

    fn state() -> Arc<DaemonState> {
        Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: "/tmp".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        })
    }

    #[tokio::test]
    async fn an_action_is_stoppable_without_a_loaded_session() {
        let state = state();
        let dir = tempfile::tempdir().unwrap();
        state
            .terminals
            .start_action(
                "unloaded",
                "dev",
                dir.path().display().to_string(),
                "sleep 30",
                0,
            )
            .await
            .unwrap();
        assert!(state.terminals.has_running_action("unloaded").await);
        let response = stop(
            State(state.clone()),
            Json(ActionBody {
                session: "unloaded".into(),
                action: "dev".into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!state.terminals.has_running_action("unloaded").await);
        assert!(state.terminals.action_runs(None).await.is_empty());
    }
}
