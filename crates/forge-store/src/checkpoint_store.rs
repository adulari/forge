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
}
