//! Focused persistence operations.

use super::*;

impl Store {
    /// Record usage for a side call (compact, diagnose) that has no corresponding agent message.
    /// Inserts a synthetic inactive system message as the FK anchor, then the usage row, and
    /// bumps the session total so daily/monthly budget queries (which read `usage`) stay accurate.
    pub fn record_side_call_usage(
        &self,
        session_id: &str,
        label: &str,
        usage: &Usage,
    ) -> Result<()> {
        let msg_id = forge_types::new_id();
        let usage_id = forge_types::new_id();
        // IMMEDIATE: this SELECTs MAX(seq) then writes. A DEFERRED txn would take a read snapshot
        // first and, if another connection committed in between, fail the upgrade with
        // SQLITE_BUSY_SNAPSHOT (which busy_timeout does NOT cover) — silently losing the usage/cost
        // row. Taking the write lock up front avoids the snapshot conflict entirely.
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut s: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(seq), -1) + 1 FROM message WHERE session_id = ?1",
                    [session_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            loop {
                let r = tx.execute(
                    "INSERT INTO message (id, session_id, seq, role, content, active) \
                     VALUES (?1, ?2, ?3, 'system', ?4, 0)",
                    (msg_id.as_str(), session_id, s, label),
                );
                match r {
                    Ok(_) => break,
                    Err(ref e) if is_unique_violation(e) => s += 1,
                    Err(e) => return Err(StoreError::Sqlite(e)),
                }
            }
            tx.execute(
                "INSERT INTO usage \
                 (id, message_id, input_tokens, cached_input_tokens, output_tokens, cost_usd) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    &usage_id,
                    msg_id.as_str(),
                    usage.input_tokens as i64,
                    usage.cached_input_tokens as i64,
                    usage.output_tokens as i64,
                    usage.cost_usd,
                ),
            )?;
            tx.execute(
                "UPDATE session SET total_cost_usd = total_cost_usd + ?1, \
                 updated_at = strftime('%s','now') WHERE id = ?2",
                (usage.cost_usd, session_id),
            )?;
            let message_payload = sync_json(serde_json::json!({
                "id": msg_id,
                "session_id": session_id,
                "seq": s,
                "role": "system",
                "content": label,
                "model": null,
                "tool_calls": [],
                "tool_call_id": null,
                "visibility": "llm",
                "active": false,
            }))?;
            append_sync_revision(
                &tx,
                "message",
                &msg_id,
                SyncJournalOperation::Upsert,
                &message_payload,
            )?;
            let usage_payload = sync_json(serde_json::json!({
                "id": usage_id,
                "message_id": msg_id,
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cached_input_tokens": usage.cached_input_tokens,
                "cost_usd": usage.cost_usd,
            }))?;
            append_sync_revision(
                &tx,
                "usage",
                &usage_id,
                SyncJournalOperation::Upsert,
                &usage_payload,
            )?;
            append_session_snapshot(&tx, session_id)?;
            tx.commit()?;
            Ok(())
        })
    }
}
