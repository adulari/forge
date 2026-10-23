//! This terminal's claim on the session it is driving, published into the shared store.

use std::sync::Arc;

/// Publishes (and, on drop, retracts) this terminal-local `forge` chat process's presence into
/// the shared store — the read-only fleet entry a `forge serve`/Anywhere client sees alongside its
/// own driven sessions (`forge_store::Store::local_live_sessions`, `SessionRow::read_only` in
/// `serve.rs`).
///
/// `store: None` (open failed, or genuinely unavailable) makes every method a silent no-op:
/// presence is a nice-to-have surfaced through remote clients, never something the interactive
/// loop should degrade or fail over.
pub(super) struct LocalPresenceGuard {
    pub(super) store: Option<Arc<forge_store::Store>>,
    /// The session id currently marked `local_live` in the store, if any. A plain `Mutex` (not
    /// async) because every access here is a quick compare-and-maybe-write, never held across an
    /// `.await`.
    current: std::sync::Mutex<Option<String>>,
    /// The loopback control endpoint this terminal published, when it opened one (`[remote]
    /// interactive_local_sessions`). Kept here rather than written once because
    /// `set_session_local_live` deliberately clears the column, so `/new` and `/resume` have to
    /// re-publish the SAME still-listening channel against the new session id — otherwise the
    /// session silently reverts to read-only in the apps the moment the user starts a new chat.
    control_url: std::sync::Mutex<Option<String>>,
}

impl LocalPresenceGuard {
    pub(super) fn new(store: Option<Arc<forge_store::Store>>) -> Self {
        Self {
            store,
            current: std::sync::Mutex::new(None),
            control_url: std::sync::Mutex::new(None),
        }
    }

    /// Record the loopback control endpoint for this terminal and publish it for the session
    /// currently marked live, making the session drivable (not merely visible) from the apps.
    pub(super) fn publish_control_url(&self, url: String) {
        let mut held = self
            .control_url
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *held = Some(url);
        let Some(store) = &self.store else { return };
        let current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = current.as_deref() {
            let _ = store.set_session_local_control_url(id, held.as_deref());
        }
    }

    /// Mark `session_id` as the locally-live session, clearing the previous one first (`/new` and
    /// `/resume` swap the id this same terminal is driving without the process restarting).
    /// Idempotent: retargeting to the id already current is a no-op.
    ///
    /// Returns whether the session's previous terminal exited without cleaning up (a stale
    /// claim), which claiming it overwrites: [`forge_core::Session::resume`] only reports that
    /// while the stale flag is still there, so a caller that claims BEFORE resuming has to carry
    /// the answer to the user itself.
    pub(super) fn retarget(&self, session_id: &str) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.as_deref() == Some(session_id) {
            return false;
        }
        if let Some(old) = current.take() {
            let _ = store.set_session_local_live(&old, false);
        }
        let prior_unclean = store.take_unclean_exit(session_id).unwrap_or(false);
        let _ = store.set_session_local_live(session_id, true);
        // A session opened here has left the daemon's fleet. Leaving its `daemon_live` flag up
        // made every later daemon restart try to take the session back from this terminal.
        let _ = store.set_session_daemon_live(session_id, false);
        *current = Some(session_id.to_string());
        self.publish_held_control_url(store, session_id);
        prior_unclean
    }

    /// Re-publish the control endpoint this terminal listens on for `session_id`.
    /// `set_session_local_live` cleared the endpoint column (it may have held a dead port from a
    /// previous process). This terminal's channel is still listening and still drives this very
    /// loop, so it is published again for the new id.
    fn publish_held_control_url(&self, store: &forge_store::Store, session_id: &str) {
        if let Some(url) = self
            .control_url
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref()
        {
            let _ = store.set_session_local_control_url(session_id, Some(url));
        }
    }

    /// Refresh the liveness heartbeat and busy flag for the currently-tracked session. If another
    /// process has cleared this terminal's claim in the meantime, put it back: this process is
    /// alive and driving the session, and a claim that stays cleared both hides it from the fleet
    /// and leaves nothing to stop the daemon resuming the session beside it.
    pub(super) fn touch(&self, busy: bool) {
        let Some(store) = &self.store else { return };
        let current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(id) = current.as_deref() else { return };
        if matches!(store.touch_session_local_presence(id, busy), Ok(false)) {
            let _ = store.set_session_local_live(id, true);
            self.publish_held_control_url(store, id);
        }
    }
}

impl Drop for LocalPresenceGuard {
    fn drop(&mut self) {
        let Some(store) = &self.store else { return };
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = current.take() {
            let _ = store.set_session_local_live(&id, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store on disk so a second connection can age a heartbeat, as a killed process leaves it.
    fn file_store() -> (tempfile::TempDir, Arc<forge_store::Store>) {
        let dir = tempfile::tempdir().unwrap();
        let store = forge_store::Store::open(dir.path().join("presence.db")).unwrap();
        (dir, Arc::new(store))
    }

    fn silent_for(dir: &tempfile::TempDir, id: &str, secs: i64) {
        rusqlite::Connection::open(dir.path().join("presence.db"))
            .unwrap()
            .execute(
                "UPDATE session SET local_last_seen = strftime('%s','now') - ?2 WHERE id = ?1",
                rusqlite::params![id, secs],
            )
            .unwrap();
    }

    #[test]
    fn a_session_opened_in_a_terminal_leaves_the_daemon_fleet() {
        let (_dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        store.set_session_daemon_live(&id, true).unwrap();

        let guard = LocalPresenceGuard::new(Some(store.clone()));
        assert!(!guard.retarget(&id));

        assert!(store.local_claim(&id).unwrap().is_some());
        assert!(
            !store.daemon_live_sessions().unwrap().contains(&id),
            "a flag left up makes every daemon restart try to take the session back"
        );
        drop(guard);
        assert!(
            store.local_claim(&id).unwrap().is_none(),
            "retracted on exit"
        );
    }

    /// Claiming a session before resuming it consumes the stale flag `Session::resume` reads, so
    /// the claim has to hand the answer back for the chat to show.
    #[test]
    fn claiming_reports_a_predecessor_that_never_cleaned_up() {
        let (dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        store.set_session_local_live(&id, true).unwrap();
        silent_for(&dir, &id, 600);

        let guard = LocalPresenceGuard::new(Some(store.clone()));
        assert!(
            guard.retarget(&id),
            "a killed terminal's claim is an unclean exit"
        );
        assert!(!store.take_unclean_exit(&id).unwrap(), "reported once");
        assert!(guard.current.lock().unwrap().is_some());

        // Retargeting the id already held reports nothing and changes nothing.
        assert!(!guard.retarget(&id));
    }

    #[test]
    fn a_claim_another_process_cleared_is_put_back_by_the_next_heartbeat() {
        let (_dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        let guard = LocalPresenceGuard::new(Some(store.clone()));
        guard.retarget(&id);
        guard.publish_control_url("http://127.0.0.1:5051/tok".to_string());

        // What a daemon resuming the session did before the claim was honoured: clear it.
        store.set_session_local_live(&id, false).unwrap();
        assert!(store.local_claim(&id).unwrap().is_none());

        guard.touch(true);
        let claim = store.local_claim(&id).unwrap().expect("claim restored");
        assert!(claim.is_fresh());
        assert_eq!(
            claim.control_url.as_deref(),
            Some("http://127.0.0.1:5051/tok")
        );
    }

    #[test]
    fn without_a_store_every_operation_is_a_quiet_no_op() {
        let guard = LocalPresenceGuard::new(None);
        assert!(!guard.retarget("anything"));
        guard.touch(false);
        guard.publish_control_url("http://127.0.0.1:1/tok".to_string());
    }
}
