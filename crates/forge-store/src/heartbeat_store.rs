//! Session-heartbeat persistence: recurring prompts that re-enter a LIVE session
//! (docs/features/session-heartbeats.md), distinct from `forge schedule`'s OS-timer registry of
//! fresh `forge run` processes. One row per heartbeat, session-scoped, cascading with its session.

use super::*;

fn map_heartbeat(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionHeartbeat> {
    Ok(SessionHeartbeat {
        id: r.get(0)?,
        session_id: r.get(1)?,
        owner: r.get(2)?,
        label: r.get(3)?,
        prompt: r.get(4)?,
        interval_secs: r.get(5)?,
        status: r.get(6)?,
        next_due_at: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

const HEARTBEAT_COLUMNS: &str = "id, session_id, owner, label, prompt, interval_secs, status, \
    next_due_at, created_at, updated_at";

impl Store {
    /// Create or replace the session's single user-owned heartbeat (`/heartbeat every`). Any
    /// existing user heartbeat on this session is deleted first — this IS the "creating another
    /// replaces it" semantics the command promises, not a failure path the caller must avoid.
    /// `id` is caller-generated ([`forge_types::new_id`]).
    pub fn set_user_heartbeat(
        &self,
        id: &str,
        session_id: &str,
        prompt: &str,
        interval_secs: i64,
        now: i64,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM session_heartbeat WHERE session_id = ?1 AND owner = 'user'",
            [session_id],
        )?;
        conn.execute(
            "INSERT INTO session_heartbeat \
                (id, session_id, owner, label, prompt, interval_secs, status, next_due_at, updated_at) \
             VALUES (?1, ?2, 'user', NULL, ?3, ?4, 'active', ?5, ?6)",
            (id, session_id, prompt, interval_secs, now + interval_secs, now),
        )?;
        Ok(())
    }

    /// The session's user-owned heartbeat, if any (`/heartbeat status`).
    pub fn user_heartbeat(&self, session_id: &str) -> Result<Option<SessionHeartbeat>> {
        let conn = self.lock()?;
        conn.query_row(
            &format!(
                "SELECT {HEARTBEAT_COLUMNS} FROM session_heartbeat \
                 WHERE session_id = ?1 AND owner = 'user'"
            ),
            [session_id],
            map_heartbeat,
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Delete the session's user-owned heartbeat (`/heartbeat clear`). `Ok(true)` if one existed.
    pub fn clear_user_heartbeat(&self, session_id: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM session_heartbeat WHERE session_id = ?1 AND owner = 'user'",
            [session_id],
        )?;
        Ok(n > 0)
    }

    /// Every heartbeat (user + agent) on a session, oldest first.
    pub fn list_heartbeats(&self, session_id: &str) -> Result<Vec<SessionHeartbeat>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {HEARTBEAT_COLUMNS} FROM session_heartbeat \
             WHERE session_id = ?1 ORDER BY created_at"
        ))?;
        let rows = stmt.query_map([session_id], map_heartbeat)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// Count of agent-owned heartbeats on a session. Cap enforcement (the small per-session
    /// limit) is a `manage_heartbeats` policy decision in forge-core, which checks this count
    /// against its configured max before calling [`Store::add_agent_heartbeat`] — this is the
    /// pure count it checks against, not the policy itself.
    pub fn agent_heartbeat_count(&self, session_id: &str) -> Result<i64> {
        self.lock()?
            .query_row(
                "SELECT COUNT(*) FROM session_heartbeat WHERE session_id = ?1 AND owner = 'agent'",
                [session_id],
                |r| r.get(0),
            )
            .map_err(StoreError::from)
    }

    /// Create a new agent-owned heartbeat, addressed later by `label`. The unique index on
    /// `(session_id, label) WHERE owner = 'agent'` rejects a duplicate label at the database
    /// boundary; the caller should check [`Store::list_heartbeats`] first to return a clean error
    /// instead of surfacing a raw constraint failure. `id` is caller-generated.
    pub fn add_agent_heartbeat(
        &self,
        id: &str,
        session_id: &str,
        label: &str,
        prompt: &str,
        interval_secs: i64,
        now: i64,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO session_heartbeat \
                (id, session_id, owner, label, prompt, interval_secs, status, next_due_at, updated_at) \
             VALUES (?1, ?2, 'agent', ?3, ?4, ?5, 'active', ?6, ?7)",
            (id, session_id, label, prompt, interval_secs, now + interval_secs, now),
        )?;
        Ok(())
    }

    /// Pause or resume a heartbeat by id. Resuming reschedules `next_due_at` from `now` — a
    /// heartbeat paused for a week does not fire immediately on resume, and does not owe the
    /// paused stretch as a backlog of missed ticks. `status` must be `"active"` or `"paused"`;
    /// `Ok(false)` if no row matched.
    pub fn set_heartbeat_status(&self, id: &str, status: &str, now: i64) -> Result<bool> {
        let n = if status == "active" {
            self.lock()?.execute(
                "UPDATE session_heartbeat \
                 SET status = 'active', next_due_at = ?1 + interval_secs, updated_at = ?1 \
                 WHERE id = ?2",
                (now, id),
            )?
        } else {
            self.lock()?.execute(
                "UPDATE session_heartbeat SET status = ?1, updated_at = ?2 WHERE id = ?3",
                (status, now, id),
            )?
        };
        Ok(n > 0)
    }

    /// Delete a heartbeat by exact id (either owner). `Ok(true)` if a row was removed.
    pub fn delete_heartbeat(&self, id: &str) -> Result<bool> {
        let n = self
            .lock()?
            .execute("DELETE FROM session_heartbeat WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    /// Delete an agent-owned heartbeat by its label (`manage_heartbeats` delete action). Never
    /// touches the user heartbeat — it has no label to match. `Ok(true)` if a row was removed.
    pub fn delete_agent_heartbeat_by_label(&self, session_id: &str, label: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM session_heartbeat \
             WHERE session_id = ?1 AND owner = 'agent' AND label = ?2",
            (session_id, label),
        )?;
        Ok(n > 0)
    }

    /// Claim every heartbeat on a session that is due at `now`, atomically rescheduling each
    /// claimed row's `next_due_at` to `now + interval_secs` as part of the same statement that
    /// returns it — the claim is durable BEFORE the caller ever builds/delivers the prompt, so a
    /// crash between claiming and delivery drops at most that one tick instead of risking a
    /// double-delivery on restart. Missed ticks coalesce for the same reason: a heartbeat overdue
    /// by any amount (one tick or fifty, e.g. after a long busy turn) is claimed exactly once
    /// here and reschedules from `now`, never from its stale `next_due_at` — one catch-up
    /// delivery, not a replayed backlog.
    pub fn claim_due_heartbeats(
        &self,
        session_id: &str,
        now: i64,
    ) -> Result<Vec<SessionHeartbeat>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "UPDATE session_heartbeat \
             SET next_due_at = ?1 + interval_secs, updated_at = ?1 \
             WHERE session_id = ?2 AND status = 'active' AND next_due_at <= ?1 \
             RETURNING {HEARTBEAT_COLUMNS}"
        ))?;
        let rows = stmt.query_map((now, session_id), map_heartbeat)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_session() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "Default").unwrap();
        (store, sid)
    }

    #[test]
    fn a_second_user_heartbeat_replaces_the_first() {
        let (store, sid) = store_with_session();
        store
            .set_user_heartbeat("hb1", &sid, "first prompt", 60, 1_000)
            .unwrap();
        store
            .set_user_heartbeat("hb2", &sid, "second prompt", 120, 1_000)
            .unwrap();

        let hb = store.user_heartbeat(&sid).unwrap().unwrap();
        assert_eq!(hb.id, "hb2");
        assert_eq!(hb.prompt, "second prompt");
        assert_eq!(hb.interval_secs, 120);
        // Exactly one row survives — the partial unique index never had to reject anything.
        assert_eq!(
            store
                .list_heartbeats(&sid)
                .unwrap()
                .iter()
                .filter(|h| h.owner == "user")
                .count(),
            1
        );
    }

    #[test]
    fn claim_due_heartbeats_coalesces_missed_ticks_into_one_delivery() {
        let (store, sid) = store_with_session();
        // Due at t=1000; the caller doesn't check in until t=10_000 (a long busy stretch covered
        // many missed ticks at a 60s interval).
        store
            .set_user_heartbeat("hb1", &sid, "ping", 60, 940)
            .unwrap();

        let claimed = store.claim_due_heartbeats(&sid, 10_000).unwrap();
        assert_eq!(claimed.len(), 1, "one delivery, not a replayed backlog");
        assert_eq!(claimed[0].id, "hb1");

        // Rescheduled from `now`, not from the stale original `next_due_at`.
        let hb = store.user_heartbeat(&sid).unwrap().unwrap();
        assert_eq!(hb.next_due_at, 10_060);

        // Not due again immediately.
        assert!(store.claim_due_heartbeats(&sid, 10_000).unwrap().is_empty());
    }

    #[test]
    fn user_and_agent_heartbeats_are_independent() {
        let (store, sid) = store_with_session();
        store
            .set_user_heartbeat("user1", &sid, "user ping", 60, 1_000)
            .unwrap();
        store
            .add_agent_heartbeat("agent1", &sid, "watch-ci", "check CI", 30, 1_000)
            .unwrap();

        let all = store.list_heartbeats(&sid).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(store.agent_heartbeat_count(&sid).unwrap(), 1);

        // Deleting/clearing one owner's heartbeat never touches the other's.
        assert!(store.clear_user_heartbeat(&sid).unwrap());
        assert_eq!(store.list_heartbeats(&sid).unwrap().len(), 1);
        assert_eq!(store.list_heartbeats(&sid).unwrap()[0].owner, "agent");

        assert!(store
            .delete_agent_heartbeat_by_label(&sid, "watch-ci")
            .unwrap());
        assert!(store.list_heartbeats(&sid).unwrap().is_empty());
    }

    #[test]
    fn duplicate_agent_label_on_the_same_session_is_rejected() {
        let (store, sid) = store_with_session();
        store
            .add_agent_heartbeat("agent1", &sid, "watch-ci", "check CI", 30, 1_000)
            .unwrap();
        let err =
            store.add_agent_heartbeat("agent2", &sid, "watch-ci", "check CI again", 30, 1_000);
        assert!(err.is_err(), "duplicate label must be rejected");
    }

    #[test]
    fn pause_then_resume_reschedules_from_resume_time_not_the_paused_stretch() {
        let (store, sid) = store_with_session();
        store
            .set_user_heartbeat("hb1", &sid, "ping", 60, 1_000)
            .unwrap();
        assert!(store.set_heartbeat_status("hb1", "paused", 1_010).unwrap());
        assert_eq!(
            store.user_heartbeat(&sid).unwrap().unwrap().status,
            "paused"
        );
        // Paused for a week; claiming must never fire a paused heartbeat.
        assert!(store
            .claim_due_heartbeats(&sid, 1_000_000)
            .unwrap()
            .is_empty());

        assert!(store
            .set_heartbeat_status("hb1", "active", 1_000_000)
            .unwrap());
        let hb = store.user_heartbeat(&sid).unwrap().unwrap();
        assert_eq!(hb.status, "active");
        assert_eq!(
            hb.next_due_at, 1_000_060,
            "resume reschedules from now, not backlog"
        );
    }
}
