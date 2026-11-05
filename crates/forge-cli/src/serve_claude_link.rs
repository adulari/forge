//! Carrying one conversation between Claude Code and Forge from a client.
//!
//! - `GET /api/sessions/{id}/claude`: is this Forge session linked to a Claude Code transcript.
//! - `POST /api/sessions/{id}/claude`: bring that transcript up to date (create it the first time)
//!   without launching anything, and say what to type to continue there.
//! - `POST /api/claude/sessions/{cc_id}/continue`: bring a Claude Code session into Forge: the
//!   linked Forge session gets only the new turns, an unlinked one is imported.
//!
//! These are the `forge switch` code paths ([`crate::cli::commands::switch`]) behind HTTP, so the
//! desktop, mobile and TUI all move a conversation exactly as the command line does.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde::Serialize;

use crate::cli::commands::import_sessions::{self, Agent, Selection};
use crate::cli::commands::switch::sync::{self, Mirrored};
use crate::cli::commands::switch::{self, cc_fs};
use crate::serve::{err_response, json_response, DaemonState};

/// One hand-off at a time, for the same reason imports are serialized: a retried request must
/// find the first one's link instead of racing it into a second transcript.
static MOVING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            &format!("{base}/api/sessions/{{id}}/claude"),
            get(link_status).post(export),
        )
        .route(
            &format!("{base}/api/claude/sessions/{{cc_id}}/continue"),
            post(continue_session),
        )
}

#[derive(Serialize)]
struct LinkBody {
    linked: bool,
    cc_session_id: Option<String>,
    cwd: String,
    /// Nothing on the Forge side is newer than what the Claude Code transcript already holds.
    synced: bool,
}

#[derive(Serialize)]
struct ExportBody {
    cc_session_id: String,
    cwd: String,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

#[derive(Serialize)]
struct ContinueBody {
    session_id: String,
    /// `imported` the first time, `synced` when the session was already linked.
    mode: &'static str,
    appended: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

fn internal(e: impl std::fmt::Display) -> Response {
    err_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}"))
}

/// Why a Forge session must not be touched right now, if it must not.
async fn forge_blocker(state: &DaemonState, id: &str) -> Option<String> {
    if let Some(handle) = state.registry.get(id).await {
        let snap = handle.snapshot_rx.borrow();
        let waiting = snap.snapshot.permission_prompt.is_some() || snap.snapshot.question.is_some();
        if snap.snapshot.busy || waiting {
            return Some("this session is busy: wait for the turn to finish".to_string());
        }
    }
    if state.store.is_session_local_read_only(id).unwrap_or(false) {
        return Some("this session is open in a terminal: close it there first".to_string());
    }
    None
}

async fn link_status(State(state): State<Arc<DaemonState>>, AxumPath(id): AxumPath<String>) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<LinkBody>> {
        let Some(meta) = store.session_meta(&id)? else {
            return Ok(None);
        };
        let link = store.cc_link_for_forge(&id)?;
        let synced = match &link {
            Some(l) => store.mirrorable_rows_after(&id, l.forge_synced_seq)? == 0,
            None => false,
        };
        Ok(Some(LinkBody {
            linked: link.is_some(),
            cc_session_id: link.map(|l| l.cc_session_id),
            cwd: meta.cwd,
            synced,
        }))
    })
    .await;
    match result {
        Ok(Ok(Some(body))) => json_response(&body),
        Ok(Ok(None)) => err_response(StatusCode::NOT_FOUND, "session not found"),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

async fn export(State(state): State<Arc<DaemonState>>, AxumPath(id): AxumPath<String>) -> Response {
    let _one_at_a_time = MOVING.lock().await;
    if let Some(why) = forge_blocker(&state, &id).await {
        return err_response(StatusCode::CONFLICT, &why);
    }
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<ExportBody, (StatusCode, String)> {
        let fail = |status, e: anyhow::Error| (status, format!("{e:#}"));
        let meta = store
            .session_meta(&id)
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "session not found".to_string()))?;
        let linked = store
            .cc_link_for_forge(&id)
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;
        if linked.is_none()
            && store
                .mirrorable_rows_after(&id, -1)
                .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?
                == 0
        {
            return Err((
                StatusCode::CONFLICT,
                "nothing to carry over yet: send a message first".to_string(),
            ));
        }
        if let Some(link) = &linked {
            let path = PathBuf::from(&link.cc_path);
            if let Some(why) = cc_fs::open_reason(&link.cc_session_id, &path, &meta.cwd) {
                return Err((
                    StatusCode::CONFLICT,
                    format!("Claude Code looks open on this session: {why}"),
                ));
            }
        }
        let report = sync::export_to_claude(&store, &id)
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let (launch, _note) = switch::claude_launch(&store, &id, &report.cc_session_id)
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(ExportBody {
            cc_session_id: report.cc_session_id,
            cwd: meta.cwd,
            command: launch.display(),
            warning: report.warning,
        })
    })
    .await;
    match result {
        Ok(Ok(body)) => json_response(&body),
        Ok(Err((status, msg))) => err_response(status, &msg),
        Err(e) => internal(e),
    }
}

async fn continue_session(
    State(state): State<Arc<DaemonState>>,
    AxumPath(cc_id): AxumPath<String>,
) -> Response {
    let _one_at_a_time = MOVING.lock().await;
    let store = state.store.clone();
    let linked_forge = store
        .cc_link_for_cc(&cc_id)
        .ok()
        .flatten()
        .map(|l| l.forge_session_id);
    if let Some(forge_id) = &linked_forge {
        if let Some(why) = forge_blocker(&state, forge_id).await {
            return err_response(StatusCode::CONFLICT, &why);
        }
    }
    let fallback = forge_config::home_dir();
    let result = tokio::task::spawn_blocking(move || -> Result<ContinueBody, (StatusCode, String)> {
        let fail = |status, e: anyhow::Error| (status, format!("{e:#}"));
        let candidates = import_sessions::scan(Some(&store));
        let found = candidates
            .iter()
            .find(|c| c.agent == Agent::Claude && c.id == cc_id)
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "that Claude Code session is not on this machine".to_string(),
                )
            })?;
        let mode = import_sessions::default_mode();
        let link = store
            .cc_link_for_cc(&cc_id)
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?
            .filter(|l| store.session_exists(&l.forge_session_id).unwrap_or(false));
        let Some(link) = link else {
            let pick = Selection {
                agent: Agent::Claude,
                id: cc_id.clone(),
            };
            let outcome = import_sessions::import_from(
                &store,
                &candidates,
                std::slice::from_ref(&pick),
                fallback.as_deref(),
                &mode,
            )
            .into_iter()
            .next()
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "nothing imported".to_string()))?;
            if let Some(error) = outcome.error {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, error));
            }
            let session_id = outcome.session_id.ok_or_else(|| {
                (StatusCode::INTERNAL_SERVER_ERROR, "import produced no session".to_string())
            })?;
            return Ok(ContinueBody {
                session_id,
                mode: "imported",
                appended: outcome.messages,
                warning: None,
            });
        };
        let own = PathBuf::from(&link.cc_path);
        let path = if own.is_file() { own } else { found.path.clone() };
        let report = sync::mirror_from_claude(&store, &path, &mode)
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let (mode, appended) = match report.kind {
            Mirrored::Imported | Mirrored::Reimported => ("imported", 0),
            Mirrored::Appended(n) => ("synced", n),
            Mirrored::Unchanged => ("synced", 0),
        };
        Ok(ContinueBody {
            session_id: report.forge_id,
            mode,
            appended,
            warning: report.warning,
        })
    })
    .await;
    match result {
        Ok(Ok(body)) => {
            // An idle driver holds the old history in memory; dropping it makes the next attach
            // load the turns that were just added.
            if body.mode == "synced" && body.appended > 0 {
                if let Some(handle) = state.registry.remove(&body.session_id).await {
                    handle.shutdown();
                }
            }
            json_response(&body)
        }
        Ok(Err((status, msg))) => err_response(status, &msg),
        Err(e) => internal(e),
    }
}

#[cfg(test)]
mod tests;
