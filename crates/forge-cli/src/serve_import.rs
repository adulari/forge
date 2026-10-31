//! `GET /api/import/scan` and `POST /api/import`: the conversations Claude Code and Codex left on
//! the machine the daemon runs on, and importing a chosen few of them as Forge sessions.
//!
//! A request names a conversation by agent and id, never by path: the daemon resolves it against
//! its own scan, so the routes cannot be turned into a read of an arbitrary file.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::cli::commands::import_sessions::{self, Candidate, Outcome, Selection};
use crate::serve::{err_response, json_response, DaemonState};

/// One import at a time: a retry after a client timeout must wait for the first request, then find
/// its conversations already imported instead of importing them again alongside it.
static IMPORTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// More than this in one request is a mistake, not a selection.
const MAX_PICKS: usize = 500;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/import/scan"), get(scan))
        .route(&format!("{base}/api/import"), post(import))
}

#[derive(Serialize)]
struct ScanBody {
    items: Vec<Candidate>,
    /// Where a conversation whose own directory is gone is opened unless the person picks another.
    fallback_cwd: String,
}

#[derive(Deserialize)]
struct ImportBody {
    items: Vec<Selection>,
    #[serde(default)]
    fallback_cwd: Option<String>,
}

#[derive(Serialize)]
struct ImportReply {
    results: Vec<Outcome>,
    imported: usize,
    skipped: usize,
    failed: usize,
}

fn home_or(state: &DaemonState) -> String {
    forge_config::home_dir().map_or_else(|| state.default_cwd.clone(), |h| h.display().to_string())
}

async fn scan(State(state): State<Arc<DaemonState>>) -> Response {
    let store = state.store.clone();
    let fallback_cwd = home_or(&state);
    match tokio::task::spawn_blocking(move || import_sessions::scan(Some(&store))).await {
        Ok(items) => json_response(&ScanBody {
            items,
            fallback_cwd,
        }),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn import(State(state): State<Arc<DaemonState>>, Json(body): Json<ImportBody>) -> Response {
    if body.items.is_empty() || body.items.len() > MAX_PICKS {
        return err_response(
            StatusCode::BAD_REQUEST,
            &format!("select between 1 and {MAX_PICKS} conversations"),
        );
    }
    let fallback = body
        .fallback_cwd
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(PathBuf::from);
    if let Some(dir) = &fallback {
        if !dir.is_dir() {
            return err_response(
                StatusCode::BAD_REQUEST,
                &format!("{} is not a directory on this machine", dir.display()),
            );
        }
    }
    let _single_flight = IMPORTING.lock().await;
    let store = state.store.clone();
    let mode = import_sessions::default_mode();
    let results = tokio::task::spawn_blocking(move || {
        import_sessions::import_selected(&store, &body.items, fallback.as_deref(), &mode)
    })
    .await;
    match results {
        Ok(results) => {
            let failed = results.iter().filter(|r| r.error.is_some()).count();
            let skipped = results.iter().filter(|r| r.skipped).count();
            json_response(&ImportReply {
                imported: results.len() - failed - skipped,
                skipped,
                failed,
                results,
            })
        }
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}
