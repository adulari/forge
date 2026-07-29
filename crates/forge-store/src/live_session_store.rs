//! Remote event replay and live-agent presence state.

use super::*;

impl Store {
    /// Write an event for an active MCP agent session. Retains at most [`LIVE_EVENT_KEEP`] plus
    /// [`LIVE_EVENT_PRUNE_EVERY`] - 1 events per session while amortizing pruning work.
    pub fn append_live_event(&self, session_id: &str, payload_json: &str) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO live_event (session_id, payload_json) VALUES (?1, ?2)",
            (session_id, payload_json),
        )?;
        // Keep one durable counter per session so separate Store handles and sessions cannot
        // delay each other's pruning schedule.
        let writes: i64 = tx.query_row(
            "UPDATE session
             SET live_event_writes = live_event_writes + 1
             WHERE id = ?1
             RETURNING live_event_writes",
            [session_id],
            |row| row.get(0),
        )?;
        if writes % LIVE_EVENT_PRUNE_EVERY as i64 == 0 {
            tx.execute(
                "DELETE FROM live_event WHERE session_id = ?1 AND id <= (
                    SELECT id FROM live_event WHERE session_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2
                 )",
                (session_id, LIVE_EVENT_KEEP),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Fetch all events for `session_id` with `id > after_id`, in order.
    pub fn live_events_after(&self, session_id: &str, after_id: i64) -> Result<Vec<(i64, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, payload_json FROM live_event WHERE session_id = ?1 AND id > ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map((session_id, after_id), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }

    /// Mark a session as having an active MCP agent.
    pub fn set_session_agent_active(&self, session_id: &str, active: bool) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET agent_active = ?1 WHERE id = ?2",
            (active as i64, session_id),
        )?;
        Ok(())
    }

    /// Mark only `session_id` inactive before an MCP agent starts. This removes a stale flag left
    /// by a killed predecessor without clearing agents currently running in other sessions.
    pub fn reset_session_agent_active(&self, session_id: &str) -> Result<()> {
        self.set_session_agent_active(session_id, false)
    }

    /// Session IDs with agent_active = 1.
    pub fn active_agent_session_ids(&self) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM session WHERE agent_active = 1 AND parent_session_id IS NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }
}
