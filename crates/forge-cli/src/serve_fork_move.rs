//! The source half of "Move": a fork whose child takes over the thread and the source retires.

use std::sync::Arc;

use crate::serve::DaemonState;

/// Stop and archive a source session whose thread now lives in a fork.
///
/// The fork child already runs in the source's directory and, for a worktree session, has adopted
/// the worktree record. So unlike a plain archive this clears the source's `worktree_path` and
/// never snapshots it: committing would race the child's first prompt in the same directory, and
/// leaving the record would let a later resume, merge or discard of the source remove the
/// worktree out from under the child.
pub(crate) async fn retire_moved_source(
    state: &Arc<DaemonState>,
    source: &str,
) -> Result<(), String> {
    if let Some(handle) = state.registry.remove(source).await {
        state.terminals.close_session(source).await;
        handle.shutdown();
        handle.join(crate::serve::ARCHIVE_JOIN_TIMEOUT).await;
    }
    let _ = state.store.set_session_daemon_live(source, false);
    state
        .store
        .clear_session_worktree(source)
        .map_err(|error| format!("releasing the source worktree failed: {error}"))?;
    state
        .store
        .archive_session(source)
        .map_err(|error| format!("archiving the moved session failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};

    use crate::serve::tests::FORGE_DB_LOCK;

    /// The fork child's driver opens the store from `FORGE_DB`, so the daemon state must share that file.
    fn state(db: &std::path::Path) -> Arc<DaemonState> {
        std::env::set_var("FORGE_DB", db);
        Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open(db).unwrap()),
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn move_carries_the_whole_thread_and_hands_the_worktree_to_the_child() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("fork-move.db"));
        let wt = dir.path().display().to_string();
        let src = state.store.create_session(&wt, "default").unwrap();
        state
            .store
            .add_message(&src, 0, forge_types::Role::User, "first", None)
            .unwrap();
        state
            .store
            .add_message(&src, 1, forge_types::Role::Assistant, "answer", None)
            .unwrap();
        state
            .store
            .add_message(&src, 2, forge_types::Role::User, "latest", None)
            .unwrap();
        state
            .store
            .add_message(&src, 3, forge_types::Role::Assistant, "latest answer", None)
            .unwrap();
        state.store.set_session_worktree(&src, &wt).unwrap();

        let req = serde_json::from_value(serde_json::json!({
            "at_seq": 2, "whole_thread": true, "move_source": true
        }))
        .unwrap();
        let response =
            crate::serve::fork_session(State(state.clone()), Path(src.clone()), axum::Json(req))
                .await;
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
        let child = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        assert_eq!(
            state.store.session_worktree(&child).unwrap().as_deref(),
            Some(wt.as_str()),
            "the child must own the worktree so it can be merged or discarded"
        );
        assert_eq!(state.store.session_worktree(&src).unwrap(), None);
        assert!(state.store.session_archived(&src).unwrap());
        assert_eq!(state.store.session_max_seq(&child).unwrap(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn plain_fork_keeps_the_source_and_its_worktree() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("fork-move.db"));
        let wt = dir.path().display().to_string();
        let src = state.store.create_session(&wt, "default").unwrap();
        state
            .store
            .add_message(&src, 0, forge_types::Role::User, "first", None)
            .unwrap();
        state.store.set_session_worktree(&src, &wt).unwrap();
        let req = serde_json::from_value(serde_json::json!({ "at_seq": 0 })).unwrap();
        let response =
            crate::serve::fork_session(State(state.clone()), Path(src.clone()), axum::Json(req))
                .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            state.store.session_worktree(&src).unwrap().as_deref(),
            Some(wt.as_str())
        );
        assert!(!state.store.session_archived(&src).unwrap());
    }
}
