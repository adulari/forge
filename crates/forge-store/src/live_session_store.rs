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

    /// Mark (or clear) whether an interactive terminal-local `forge` process — NOT `forge serve`,
    /// NOT an MCP agent — currently has this session open. Set on session start, cleared on clean
    /// exit; see [`Store::local_live_sessions`] for what a killed terminal (which never clears it)
    /// looks like on the read side.
    pub fn set_session_local_live(&self, session_id: &str, live: bool) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session
             SET local_live = ?2, local_busy = 0,
                 local_last_seen = strftime('%s','now')
             WHERE id = ?1",
            rusqlite::params![session_id, i64::from(live)],
        )?;
        Ok(())
    }

    /// Refresh the liveness heartbeat and busy flag for a locally-live session. Called at turn
    /// start/end and on a coarse idle tick so a killed terminal's row ages out instead of showing
    /// live forever. A no-op (0 rows touched) if the session isn't currently marked local-live —
    /// callers don't need to special-case that.
    pub fn touch_session_local_presence(&self, session_id: &str, busy: bool) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session
             SET local_busy = ?2, local_last_seen = strftime('%s','now')
             WHERE id = ?1 AND local_live = 1",
            rusqlite::params![session_id, i64::from(busy)],
        )?;
        Ok(())
    }

    /// Sessions currently open in an interactive terminal, most recently active first. This is the
    /// read-only counterpart to [`Store::daemon_live_sessions`]: same idea (a durable flag survives
    /// process restarts the in-memory registry can't), but for the plain `forge` chat loop instead
    /// of `forge serve`'s driver.
    ///
    /// A stale heartbeat (`local_last_seen` older than [`LOCAL_PRESENCE_STALE_SECS`]) is excluded —
    /// a crashed or `kill -9`'d terminal never runs its cleanup, so the read side ages it out rather
    /// than trusting every writer to always tidy up after itself.
    pub fn local_live_sessions(&self) -> Result<Vec<LocalPresenceRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.title, s.cwd, s.worktree_path, s.local_busy, s.created_at,
                    COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                             s.created_at) AS last_activity,
                    (SELECT r.chosen_model FROM routing_decision r
                     JOIN message m ON m.id = r.message_id
                     WHERE m.session_id = s.id ORDER BY m.seq DESC LIMIT 1) AS model
             FROM session s
             WHERE s.local_live = 1
               AND s.local_last_seen >= strftime('%s','now') - ?1
               AND s.archived = 0
               AND s.parent_session_id IS NULL
             ORDER BY last_activity DESC, s.rowid DESC",
        )?;
        let rows = stmt.query_map([LOCAL_PRESENCE_STALE_SECS], |row| {
            Ok(LocalPresenceRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                worktree_path: row.get(3)?,
                busy: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
                last_activity: row.get(6)?,
                model: row.get(7)?,
            })
        })?;
        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }

    /// Whether `session_id` is currently a non-stale terminal-local session — i.e. it would show
    /// up (read-only) in [`Store::local_live_sessions`]. Used to give a remote input attempt a
    /// clear "this session is read-only" refusal instead of a bare "not found", which would be
    /// confusing for an id a client just saw in the fleet list.
    pub fn is_session_local_read_only(&self, session_id: &str) -> Result<bool> {
        let live: i64 = self
            .lock()?
            .query_row(
                "SELECT 1 FROM session
             WHERE id = ?1 AND local_live = 1
               AND local_last_seen >= strftime('%s','now') - ?2
             LIMIT 1",
                rusqlite::params![session_id, LOCAL_PRESENCE_STALE_SECS],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(live == 1)
    }
}
