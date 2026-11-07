//! Custody of a goal's PR when the goal changes workers.
//!
//! A fresh-worker relay (`forge_store::Store::create_relay_successor`) hands the worktree and the
//! fleet slot to the successor in one transaction. PR Watch and the merge queue are keyed on
//! session id and keep no store state, so this module moves them the same way: a watch follows
//! the end of its session's hand-off chain, a head nobody hosts is hosted on demand or taken out
//! of the queue with a reason, and an archive gives the watch back instead of leaving it to block
//! the repo. Everything here is idempotent and re-derivable from the store's relay table, so a
//! daemon that died halfway heals on its next sweep.

use std::sync::Arc;

use crate::cli::commands::run::SessionDriverHandle;
use crate::serve::DaemonState;
use crate::serve_pr_watch::{registry, Mode, Registry, Watch};

impl Registry {
    /// Re-key an active watch (queue place included: `created_at` and the owner's rank travel in
    /// the row) from `from` to `to`. If `to` already has its own watch the old one is dropped.
    pub(crate) fn rekey(&self, from: &str, to: &str) -> bool {
        let mut guard = self.lock();
        if !guard.get(from).is_some_and(Watch::active) {
            return false;
        }
        let Some(mut watch) = guard.remove(from) else {
            return false;
        };
        if !guard.contains_key(to) {
            watch.session_id = to.to_string();
            guard.insert(to.to_string(), watch);
        }
        self.persist(&guard);
        true
    }

    /// Drop a session's watch.
    pub(crate) fn disarm(&self, session: &str) -> bool {
        let mut guard = self.lock();
        let removed = guard.remove(session).is_some();
        if removed {
            self.persist(&guard);
        }
        removed
    }
}

/// Move every active watch to the session that holds its goal now. Returns how many moved.
pub(crate) fn settle(state: &DaemonState) -> usize {
    let mut moved = 0;
    for watch in registry().all().into_iter().filter(Watch::active) {
        let Ok(tail) = state.store.relay_tail(&watch.session_id) else {
            continue;
        };
        if tail != watch.session_id && registry().rekey(&watch.session_id, &tail) {
            tracing::info!(from = %watch.session_id, to = %tail, "PR watch followed its goal to a fresh worker");
            moved += 1;
        }
    }
    if moved > 0 {
        state.registry.notify_fleet();
    }
    moved
}

/// A predecessor that handed its goal on stops being hosted: its driver would otherwise keep a
/// stale worktree in memory and could take a prompt aimed at the directory the successor owns.
pub(crate) async fn retire_predecessor(state: &Arc<DaemonState>, predecessor: &str) {
    if let Some(handle) = state.registry.remove(predecessor).await {
        state.terminals.close_session(predecessor).await;
        handle.shutdown();
        handle.join(crate::serve::ARCHIVE_JOIN_TIMEOUT).await;
    }
    let _ = state.store.set_session_daemon_live(predecessor, false);
}

/// After the daemon hosted `successor`: the watch follows, the predecessor goes.
pub(crate) async fn successor_hosted(state: &Arc<DaemonState>, successor: &str) {
    settle(state);
    if let Ok(Some(link)) = state.store.relay_predecessor(successor) {
        retire_predecessor(state, &link.predecessor_id).await;
    }
}

/// The queue head's session is not in the registry. Host it on demand; when that cannot be done
/// (archived, open in a terminal, gone) take its PR out of the queue with the reason so the PRs
/// behind it keep landing. `None` means this tick has nothing to drive.
pub(crate) async fn unhosted(
    state: &Arc<DaemonState>,
    watch: &Watch,
) -> Option<Arc<SessionDriverHandle>> {
    if watch.mode != Mode::Merge
        || watch.queue.is_none()
        || crate::serve_merge_queue::parked_unhosted(watch)
    {
        return None;
    }
    let id = watch.session_id.as_str();
    let why = if state.store.session_archived(id).unwrap_or(false) {
        "it was archived".to_string()
    } else if crate::terminal_owner::terminal_owns_session(&state.store, id).await {
        "it is open in a terminal".to_string()
    } else {
        let request = crate::serve::CreateSessionReq {
            resume: Some(id.to_string()),
            ..Default::default()
        };
        let response = crate::serve::create_session(
            axum::extract::State(state.clone()),
            Ok(axum::Json(request)),
        )
        .await;
        if response.status().is_success() {
            if let Some(handle) = state.registry.get(id).await {
                return Some(handle);
            }
        }
        format!("restoring it failed ({})", response.status())
    };
    registry().skip_unhosted(id, &why);
    state.registry.notify_fleet();
    None
}

impl Registry {
    fn skip_unhosted(&self, session: &str, why: &str) {
        let mut guard = self.lock();
        if let Some(watch) = guard.get_mut(session) {
            crate::serve_merge_queue::skip_unhosted(watch, why);
        }
        self.persist(&guard);
    }
}

/// An archive ends the session's claim on its PR: the watch goes to the session holding the goal
/// when the goal moved on, and is disarmed otherwise, so a retired head cannot hold the queue.
pub(crate) fn on_archive(state: &DaemonState, session: &str) {
    let tail = state
        .store
        .relay_tail(session)
        .unwrap_or_else(|_| session.to_string());
    let handed_back = tail != session && registry().rekey(session, &tail);
    if handed_back || registry().disarm(session) {
        state.registry.notify_fleet();
    }
}

/// Roll back merges a crashed update left in the worktrees of merge watches; a daemon that
/// restarted mid-update would otherwise answer "a merge is already in progress" for ever.
pub(crate) async fn recover_interrupted_updates(state: &Arc<DaemonState>) {
    for watch in registry().all().into_iter().filter(Watch::active) {
        if watch.mode != Mode::Merge {
            continue;
        }
        let Ok(root) = crate::serve_git::session_repo_root(state, &watch.session_id).await else {
            continue;
        };
        let session = watch.session_id.clone();
        let note = tokio::task::spawn_blocking(move || {
            crate::serve_merge_queue::recover_interrupted_update(&root)
        })
        .await
        .ok()
        .flatten();
        if let Some(note) = note {
            tracing::info!(%session, "{note}");
            registry().note_action(&session, &note);
        }
    }
}

impl Registry {
    fn note_action(&self, session: &str, note: &str) {
        let mut guard = self.lock();
        if let Some(watch) = guard.get_mut(session) {
            watch.last_action = Some(note.to_string());
        }
        self.persist(&guard);
    }
}
