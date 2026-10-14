//! Durable host fingerprint for moved-session detection (docs/features/session-host-move.md).
//!
//! The payload is opaque JSON owned by `forge_core::host_move`; the store only guarantees that a
//! notice is decided once per move, by swapping the value atomically.

use rusqlite::OptionalExtension;

use crate::{Result, Store};

impl Store {
    /// The recorded host state JSON, `None` when the session never recorded one.
    pub fn session_host_state(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT host_state FROM session WHERE id = ?1",
                [session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Replace the host state only if it still equals `expected`. Returns whether this caller won:
    /// two processes resuming the same moved session must not both announce the move.
    pub fn swap_session_host_state(
        &self,
        session_id: &str,
        expected: Option<&str>,
        next: &str,
    ) -> Result<bool> {
        let changed = self.lock()?.execute(
            "UPDATE session SET host_state = ?3 WHERE id = ?1 AND host_state IS ?2",
            rusqlite::params![session_id, expected, next],
        )?;
        Ok(changed > 0)
    }

    /// Forget a recorded isolated worktree (used when it does not exist on this host).
    pub fn clear_session_worktree(&self, session_id: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET worktree_path = NULL WHERE id = ?1",
            [session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn host_state_swaps_only_from_the_expected_value() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "Default").unwrap();
        assert_eq!(store.session_host_state(&id).unwrap(), None);
        assert!(store.swap_session_host_state(&id, None, "a").unwrap());
        assert!(!store.swap_session_host_state(&id, None, "b").unwrap());
        assert!(store.swap_session_host_state(&id, Some("a"), "b").unwrap());
        assert_eq!(store.session_host_state(&id).unwrap().as_deref(), Some("b"));
    }

    #[test]
    fn cwd_and_worktree_can_be_relocated() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/old", "Default").unwrap();
        store.set_session_worktree(&id, "/old/wt").unwrap();
        store.set_session_cwd(&id, "/new").unwrap();
        store.clear_session_worktree(&id).unwrap();
        assert_eq!(store.session_cwd(&id).unwrap().as_deref(), Some("/new"));
        assert_eq!(store.session_worktree(&id).unwrap(), None);
    }

    #[test]
    fn handoff_carries_the_host_state_and_the_compaction_summary() {
        let source = Store::open_in_memory().unwrap();
        let id = source.create_session("/laptop/app", "Default").unwrap();
        for seq in 0..4 {
            source
                .add_message(&id, seq, forge_types::Role::User, &format!("m{seq}"), None)
                .unwrap();
        }
        source.compact_session_store(&id, "SUMMARY", 1).unwrap();
        source
            .swap_session_host_state(&id, None, "{\"h\":1}")
            .unwrap();
        let export = source.export_handoff_session(&id).unwrap();
        let json = serde_json::to_string(&export).unwrap();

        let dest = Store::open_in_memory().unwrap();
        let imported = dest
            .import_handoff_session(&serde_json::from_str(&json).unwrap(), "/server/wt")
            .unwrap();
        assert_eq!(
            dest.session_host_state(&imported.session_id)
                .unwrap()
                .as_deref(),
            Some("{\"h\":1}")
        );
        let messages = dest.load_messages(&imported.session_id).unwrap();
        assert!(messages[0].content.contains("SUMMARY"), "{messages:?}");
    }
}
