//! Session lifecycle: star, mark done, restore from the archive, and the background sweep that
//! unloads idle sessions and archives stale ones.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Json, Path, State};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use forge_config::sessions::SessionsConfig;
use serde::Deserialize;

use crate::serve::{err_response, json_response, DaemonState};

const SWEEP_EVERY: Duration = Duration::from_secs(300);
const UNLOAD_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/sessions/{{id}}/star"), post(star))
        .route(&format!("{base}/api/sessions/{{id}}/done"), post(done))
        .route(
            &format!("{base}/api/sessions/{{id}}/restore"),
            post(restore),
        )
}

#[derive(Deserialize)]
struct StarReq {
    starred: bool,
}

#[derive(Deserialize)]
struct DoneReq {
    done: bool,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn store_result(
    state: &DaemonState,
    found: Result<bool, forge_store::StoreError>,
    body: serde_json::Value,
) -> Response {
    match found {
        Ok(true) => {
            state.registry.notify_fleet();
            json_response(&body)
        }
        Ok(false) => err_response(axum::http::StatusCode::NOT_FOUND, "no such session"),
        Err(error) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("updating session failed: {error}"),
        ),
    }
}

async fn star(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(req): Json<StarReq>,
) -> Response {
    let found = state.store.set_session_starred(&id, req.starred);
    store_result(
        &state,
        found,
        serde_json::json!({ "ok": true, "starred": req.starred }),
    )
}

async fn done(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(req): Json<DoneReq>,
) -> Response {
    let done_at = req.done.then(now_secs);
    let found = state.store.set_session_done_at(&id, done_at);
    store_result(
        &state,
        found,
        serde_json::json!({ "ok": true, "done_at": done_at }),
    )
}

/// `POST /api/sessions/{id}/archive` — stop the driver, snapshot a worktree's uncommitted
/// edits onto its branch (never silently lose work), and hide the session from lists.
pub(crate) async fn archive_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refusal) = crate::serve::reject_if_open_in_terminal(&state, &id, "archived").await {
        return refusal;
    }
    let handle = state.registry.remove(&id).await;
    let worktree = if let Some(handle) = &handle {
        handle.worktree.clone()
    } else {
        match state.store.session_worktree(&id) {
            Ok(worktree) => worktree,
            Err(error) => {
                return err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("reading session failed: {error}"),
                );
            }
        }
    };
    if handle.is_none() {
        match state.store.session_exists(&id) {
            Ok(true) => {}
            Ok(false) => {
                return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
            }
            Err(error) => {
                return err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("reading session failed: {error}"),
                );
            }
        }
    }
    state.terminals.close_session(&id).await;
    // Archiving is an explicit "done with this" — drop it from the fleet a restart would restore.
    let _ = state.store.set_session_daemon_live(&id, false);
    if let Some(handle) = handle {
        handle.shutdown();
        handle.join(crate::serve::ARCHIVE_JOIN_TIMEOUT).await;
    }
    if let Some(wt) = worktree {
        // Best-effort snapshot: uncommitted edits land on the session's branch so nothing is
        // lost; the worktree + branch stay in place for a manual review/merge.
        if let Err(error) = forge_core::worktree::commit_worktree(std::path::Path::new(&wt)) {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("snapshotting the session worktree failed: {error}"),
            );
        }
    }
    if let Err(error) = state.store.archive_session(&id) {
        return err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("archiving session failed: {error}"),
        );
    }
    json_response(&serde_json::json!({ "ok": true }))
}

/// Un-archive without resuming: the session returns to the lists and stays unloaded until opened.
async fn restore(State(state): State<Arc<DaemonState>>, Path(id): Path<String>) -> Response {
    match state.store.session_exists(&id) {
        Ok(true) => {}
        Ok(false) => return err_response(axum::http::StatusCode::NOT_FOUND, "no such session"),
        Err(error) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("reading session failed: {error}"),
            )
        }
    }
    let result = state.store.unarchive_session(&id).map(|()| true);
    store_result(&state, result, serde_json::json!({ "ok": true }))
}

pub(crate) fn idle_unload_due(
    last_activity: i64,
    created_at: i64,
    now: i64,
    idle_minutes: u32,
) -> bool {
    idle_minutes > 0 && now - last_activity.max(created_at) >= i64::from(idle_minutes) * 60
}

pub(crate) fn start(state: &Arc<DaemonState>, config: SessionsConfig) {
    if config.idle_unload_minutes == 0 && config.auto_archive_days == 0 {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            sweep(&state, &config, now_secs()).await;
        }
    });
}

pub(crate) async fn sweep(state: &Arc<DaemonState>, config: &SessionsConfig, now: i64) {
    for handle in state.registry.all().await {
        let snapshot = handle.snapshot_rx.borrow().snapshot.clone();
        if snapshot.busy || snapshot.permission_prompt.is_some() || snapshot.question.is_some() {
            continue;
        }
        let last = handle
            .last_activity
            .load(std::sync::atomic::Ordering::Relaxed);
        if crate::serve_pr_watch::is_watching(&handle.session_id)
            || !idle_unload_due(last, handle.created_at, now, config.idle_unload_minutes)
        {
            continue;
        }
        let Some(handle) = state.registry.remove(&handle.session_id).await else {
            continue;
        };
        let _ = state
            .store
            .set_session_daemon_live(&handle.session_id, false);
        handle.shutdown();
        handle.join(UNLOAD_JOIN_TIMEOUT).await;
        tracing::info!(session = %handle.session_id, "unloaded idle session");
    }
    if config.auto_archive_days == 0 {
        return;
    }
    let cutoff = now - i64::from(config.auto_archive_days) * 86_400;
    let store = state.store.clone();
    let stale = tokio::task::spawn_blocking(move || store.sessions_idle_since(cutoff))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let mut archived = false;
    for (id, worktree) in stale {
        if worktree.is_some_and(|path| std::path::Path::new(&path).exists())
            || state.registry.get(&id).await.is_some()
            || crate::terminal_owner::terminal_owns_session(&state.store, &id).await
        {
            continue;
        }
        archived |= state.store.archive_session(&id).is_ok();
    }
    if archived {
        state.registry.notify_fleet();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unload_waits_for_the_threshold_and_can_be_disabled() {
        assert!(!idle_unload_due(1000, 0, 1000 + 59 * 60, 60));
        assert!(idle_unload_due(1000, 0, 1000 + 60 * 60, 60));
        assert!(!idle_unload_due(0, 0, 10_000_000, 0));
        assert!(!idle_unload_due(0, 9_999_999, 10_000_000, 60));
    }

    fn state() -> Arc<DaemonState> {
        Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
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

    fn used(state: &DaemonState) -> String {
        let id = state.store.create_session("/w", "default").unwrap();
        state
            .store
            .add_message(&id, 0, forge_types::Role::User, "hi", None)
            .unwrap();
        id
    }

    #[tokio::test]
    async fn star_done_and_restore_persist_and_unknown_ids_404() {
        let state = state();
        let id = used(&state);
        let response = star(
            State(state.clone()),
            Path(id.clone()),
            Json(StarReq { starred: true }),
        )
        .await;
        assert_eq!(response.status(), 200);
        let response = done(
            State(state.clone()),
            Path(id.clone()),
            Json(DoneReq { done: true }),
        )
        .await;
        assert_eq!(response.status(), 200);
        let lifecycle = state
            .store
            .session_lifecycle(std::slice::from_ref(&id))
            .unwrap()[&id];
        assert!(lifecycle.0 && lifecycle.1.is_some());
        state.store.archive_session(&id).unwrap();
        let response = restore(State(state.clone()), Path(id.clone())).await;
        assert_eq!(response.status(), 200);
        assert!(!state.store.session_archived(&id).unwrap());
        let response = star(
            State(state),
            Path("nope".into()),
            Json(StarReq { starred: true }),
        )
        .await;
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn the_sweep_archives_only_stale_unstarred_sessions() {
        let state = state();
        let stale = used(&state);
        let starred = used(&state);
        state.store.set_session_starred(&starred, true).unwrap();
        let config = SessionsConfig::default();
        sweep(&state, &config, now_secs()).await;
        assert!(!state.store.session_archived(&stale).unwrap());
        sweep(&state, &config, now_secs() + 4 * 86_400).await;
        assert!(state.store.session_archived(&stale).unwrap());
        assert!(!state.store.session_archived(&starred).unwrap());
        let off = SessionsConfig {
            auto_archive_days: 0,
            ..config
        };
        let fresh = used(&state);
        sweep(&state, &off, now_secs() + 40 * 86_400).await;
        assert!(!state.store.session_archived(&fresh).unwrap());
    }
}
