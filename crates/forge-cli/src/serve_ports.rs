//! `GET /api/ports?session=` lists the TCP ports the session's processes are listening on: its
//! terminals and project actions, everything they spawned, and any process working inside the
//! session's worktree (an agent-started dev server has no terminal parent).
//!
//! It needs the `terminal` scope: process command lines can carry secrets.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::json;

use crate::serve::{err_response, json_response, DaemonState};

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(&format!("{base}/api/ports"), get(ports))
}

#[derive(Deserialize)]
struct PortsQuery {
    #[serde(default)]
    session: String,
}

async fn ports(State(state): State<Arc<DaemonState>>, Query(query): Query<PortsQuery>) -> Response {
    let Some(session) = state.registry.get(&query.session).await else {
        return err_response(StatusCode::NOT_FOUND, "session not found");
    };
    let roots = state.terminals.session_roots(&query.session).await;
    let workspace = session
        .worktree
        .clone()
        .unwrap_or_else(|| session.cwd.clone());
    let workspace = std::path::PathBuf::from(workspace)
        .canonicalize()
        .ok()
        .filter(|path| path.parent().is_some());
    let found =
        tokio::task::spawn_blocking(move || crate::ports::scan(&roots, workspace.as_deref()))
            .await
            .unwrap_or_default();
    json_response(&json!({ "ports": found }))
}
