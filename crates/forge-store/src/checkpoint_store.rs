//! Conversation rewind checkpoints and soft-deactivation.

use super::*;

impl Store {
    // --- Conversation checkpoints / undo (RFC session-management-and-commands, PR2) ---

    /// Soft-delete every message of a session with `seq >= from_seq` (an `/undo` / checkpoint
    /// rewind). The rows stay in the table (`active = 0`) for audit/redo; [`load_messages`]
    /// excludes them. Returns the number of messages deactivated.
    pub fn deactivate_messages_from(&self, session_id: &str, from_seq: i64) -> Result<usize> {
        Ok(self.lock()?.execute(
            "UPDATE message SET active = 0 WHERE session_id = ?1 AND seq >= ?2 AND active = 1",
            (session_id, from_seq),
        )?)
    }

    /// The lowest `seq` still active for a session, or `None` when nothing is active. A rewind
    /// target below this boundary on a compacted session lands inside the folded-away history,
    /// which has to be reactivated first ([`Session::rewind_to`]).
    pub fn min_active_seq(&self, session_id: &str) -> Result<Option<i64>> {
        Ok(self.lock()?.query_row(
            "SELECT MIN(seq) FROM message WHERE session_id = ?1 AND active = 1",
            [session_id],
            |row| row.get::<_, Option<i64>>(0),
        )?)
    }

    /// Save a checkpoint (rewind point) at `seq`. `label` NULL = an auto per-turn checkpoint.
    pub fn add_checkpoint(
        &self,
        session_id: &str,
        label: Option<&str>,
        seq: i64,
    ) -> Result<String> {
        let id = forge_types::new_id();
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO checkpoint (id, session_id, label, seq) VALUES (?1, ?2, ?3, ?4)",
                (&id, session_id, label, seq),
            )?;
            let payload = sync_json(serde_json::json!({
                "id": id,
                "session_id": session_id,
                "label": label,
                "seq": seq,
            }))?;
            append_sync_revision(
                &tx,
                "checkpoint",
                &id,
                SyncJournalOperation::Upsert,
                &payload,
            )?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(id)
    }

    /// A session's named checkpoints, newest first.
    pub fn list_checkpoints(&self, session_id: &str) -> Result<Vec<CheckpointRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, label, seq, created_at FROM checkpoint
             WHERE session_id = ?1 ORDER BY seq DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok(CheckpointRow {
                id: row.get(0)?,
                label: row.get(1)?,
                seq: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Append a user/assistant message and return `(id, seq)`: the seq the row REALLY landed on.
    /// The one asked for can be taken already (a side call's usage anchor row, a concurrent
    /// writer), and the row then lands at MAX(seq)+1. A caller that keys other state by the row's
    /// seq (the rewind checkpoint, the snapshot directory) must use the returned one.
    pub fn add_message_returning_seq(
        &self,
        session_id: &str,
        seq: i64,
        role: forge_types::Role,
        content: &str,
        model: Option<&str>,
    ) -> Result<(String, i64)> {
        self.insert_message(
            session_id,
            seq,
            role,
            content,
            model,
            &[],
            None,
            forge_types::Visibility::Llm,
            forge_types::MessageKind::Normal,
        )
    }

    /// Content of the active `role = 'user'` row at exactly `seq` (the prompt a rewind to that
    /// boundary hands back to the input box). `None` when the row is missing or not a user row.
    pub fn user_prompt_at(&self, session_id: &str, seq: i64) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT content FROM message
                 WHERE session_id = ?1 AND seq = ?2 AND role = 'user' AND active = 1",
                (session_id, seq),
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    /// DB seq of the newest active, person-typed user message (not a harness nudge, not a UI-only
    /// note) strictly below `before` (`None` = no upper bound). This is the turn anchor `/undo`
    /// chains through; it is read from the store because the in-memory transcript has no seqs.
    pub fn last_real_user_seq(&self, session_id: &str, before: Option<i64>) -> Result<Option<i64>> {
        Ok(self.lock()?.query_row(
            "SELECT MAX(seq) FROM message
             WHERE session_id = ?1 AND active = 1 AND role = 'user' AND nudge = 0
               AND visibility != 'ui' AND seq < ?2",
            (session_id, before.unwrap_or(i64::MAX)),
            |row| row.get::<_, Option<i64>>(0),
        )?)
    }

    /// `(seq, role, active, content)` for every row of a session, rewound and compacted ones
    /// included. Diagnostic/test read: the other loaders all hide what a rewind removed.
    #[doc(hidden)]
    pub fn message_state_rows(&self, session_id: &str) -> Result<Vec<(i64, String, bool, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT seq, role, active, content FROM message WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::{Role, Usage};

    fn store_with_session() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "default").unwrap();
        (store, id)
    }

    #[test]
    fn a_row_that_lands_past_the_requested_seq_reports_where_it_landed() {
        let (store, id) = store_with_session();
        store
            .add_message(&id, 0, Role::User, "first", None)
            .unwrap();
        // The recap's usage anchor takes the next seq behind the session's back.
        store
            .record_side_call_usage(&id, "recap", &Usage::default())
            .unwrap();
        let (_, landed) = store
            .add_message_returning_seq(&id, 1, Role::User, "second", None)
            .unwrap();
        assert_eq!(
            landed, 2,
            "seq 1 was taken by the anchor; the row lands at MAX+1"
        );
        assert_eq!(store.next_seq_for_session(&id).unwrap(), 3);
    }

    #[test]
    fn user_prompt_lookup_and_turn_anchor_ignore_nudges_ui_notes_and_rewound_rows() {
        let (store, id) = store_with_session();
        store
            .add_message(&id, 0, Role::User, "first", None)
            .unwrap();
        store
            .add_message(&id, 1, Role::Assistant, "ack", None)
            .unwrap();
        store.add_nudge_message(&id, 2, "keep going").unwrap();
        store
            .add_message(&id, 3, Role::User, "second", None)
            .unwrap();
        store.add_ui_note(&id, 4, Role::User, "a ui note").unwrap();

        assert_eq!(
            store.user_prompt_at(&id, 3).unwrap().as_deref(),
            Some("second")
        );
        assert_eq!(
            store.user_prompt_at(&id, 1).unwrap(),
            None,
            "not a user row"
        );
        assert_eq!(store.last_real_user_seq(&id, None).unwrap(), Some(3));
        assert_eq!(store.last_real_user_seq(&id, Some(3)).unwrap(), Some(0));
        assert_eq!(store.last_real_user_seq(&id, Some(0)).unwrap(), None);

        store.deactivate_messages_from(&id, 3).unwrap();
        assert_eq!(store.last_real_user_seq(&id, None).unwrap(), Some(0));
        assert_eq!(
            store.user_prompt_at(&id, 3).unwrap(),
            None,
            "rewound rows are gone"
        );
    }
}
