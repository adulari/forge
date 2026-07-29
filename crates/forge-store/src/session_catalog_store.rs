//! Session discovery, resume metadata, archive state, and prefix resolution.

use super::*;

impl Store {
    /// Number of messages in a session.
    pub fn message_count(&self, session_id: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            // `active = 1` only — soft-deleted (undone/compacted) rows must not inflate the count
            // shown in the session picker / `forge sessions`, which `load_messages` also excludes.
            "SELECT COUNT(*) FROM message WHERE session_id = ?1 AND active = 1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// The id of the most-recent top-level session (excludes subagent children), or `None` if
    /// there are no sessions yet.
    pub fn most_recent_session_id(&self) -> Result<Option<String>> {
        let conn = self.lock()?;
        // Order by LAST ACTIVITY (newest message), not creation time, so `--continue` reattaches
        // the session the user actually used most recently — not whichever was created last.
        let result = conn
            .query_row(
                "SELECT s.id FROM session s WHERE s.parent_session_id IS NULL \
                 AND s.archived = 0 \
                 AND EXISTS (SELECT 1 FROM message m WHERE m.session_id = s.id AND m.role = 'user') \
                 ORDER BY COALESCE( \
                   (SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id), \
                   s.created_at) DESC, s.rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(result)
    }

    /// Past sessions, **most-recently-used first** (by newest message, falling back to creation
    /// time), so the picker lists the sessions you're likely to resume at the top. Excludes
    /// subagent child sessions (`parent_session_id IS NOT NULL`) so the picker and the
    /// `forge sessions` command only surface top-level sessions. Also excludes sessions that
    /// never received a real (role='user') message — checked regardless of `active`, so a
    /// session whose sole user message was later soft-deleted by `/undo` or a checkpoint restore
    /// still counts as used — a session row is created eagerly at process start (before
    /// [`Store::prune_empty`] has a chance to sweep it, and for a session still in its first
    /// few minutes of life), so without this filter a process that opens a session and
    /// exits/crashes before any prompt is sent — including one stuck in a spawn loop, the
    /// original trigger for this — fills the picker with blank, useless entries.
    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.cwd, s.permission_mode, s.created_at, s.total_cost_usd,
                    (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id AND m.active = 1),
                    (SELECT content FROM message m WHERE m.session_id = s.id
                       AND m.role = 'user' AND m.active = 1 ORDER BY m.seq LIMIT 1),
                    COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                             s.created_at) AS last_activity,
                    s.title, s.worktree_path
             FROM session s WHERE s.parent_session_id IS NULL \
             AND s.archived = 0 \
             AND EXISTS ( \
               SELECT 1 FROM message m \
               WHERE m.session_id = s.id AND m.role = 'user' \
             ) \
             ORDER BY last_activity DESC, s.rowid DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                cwd: row.get(1)?,
                permission_mode: row.get(2)?,
                created_at: row.get(3)?,
                total_cost_usd: row.get(4)?,
                message_count: row.get(5)?,
                preview: row.get(6)?,
                last_activity: row.get(7)?,
                title: row.get(8)?,
                worktree_path: row.get(9)?,
                archived: false, // filtered to archived = 0 above
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Like [`Store::list_sessions`] but INCLUDES archived sessions (flagged via
    /// [`SessionSummary::archived`]) instead of hiding them. Used by `forge serve`'s
    /// past-sessions browser (`GET /api/sessions/past`) so a session the user explicitly
    /// archived is still browsable and resumable — just visibly marked — rather than only
    /// surfacing sessions orphaned by a daemon restart. Same MRU ordering, same exclusion of
    /// subagent children and sessions that never received a real user message.
    pub fn list_sessions_for_resume(&self) -> Result<Vec<SessionSummary>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.cwd, s.permission_mode, s.created_at, s.total_cost_usd,
                    (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id AND m.active = 1),
                    (SELECT content FROM message m WHERE m.session_id = s.id
                       AND m.role = 'user' AND m.active = 1 ORDER BY m.seq LIMIT 1),
                    COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                             s.created_at) AS last_activity,
                    s.title, s.worktree_path, s.archived
             FROM session s WHERE s.parent_session_id IS NULL \
             AND EXISTS ( \
               SELECT 1 FROM message m \
               WHERE m.session_id = s.id AND m.role = 'user' \
             ) \
             ORDER BY last_activity DESC, s.rowid DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                cwd: row.get(1)?,
                permission_mode: row.get(2)?,
                created_at: row.get(3)?,
                total_cost_usd: row.get(4)?,
                message_count: row.get(5)?,
                preview: row.get(6)?,
                last_activity: row.get(7)?,
                title: row.get(8)?,
                worktree_path: row.get(9)?,
                archived: row.get::<_, i64>(10)? != 0,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Archive a session (`forge serve`): hidden from [`Store::list_sessions`] and the daemon's
    /// session list, but its full history stays intact (nothing is deleted).
    pub fn archive_session(&self, session_id: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET archived = 1 WHERE id = ?1",
            [session_id],
        )?;
        Ok(())
    }

    /// Whether a session is archived. `Ok(false)` for unknown ids (nothing to un-hide).
    pub fn session_archived(&self, session_id: &str) -> Result<bool> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM session WHERE id = ?1 AND archived = 1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// Un-archive a session: reverses [`Store::archive_session`]. `forge serve` calls this when
    /// resuming a session from the past-sessions browser — resurrecting an archived session is
    /// an explicit choice to bring it back, so it should reappear in [`Store::list_sessions`]
    /// and the fleet list once it stops running again, rather than immediately re-hiding itself.
    pub fn unarchive_session(&self, session_id: &str) -> Result<()> {
        let conn = self.lock()?;
        let blocked: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM anywhere_handoff_session_state WHERE session_id=?1)",
            [session_id],
            |row| row.get(0),
        )?;
        if blocked {
            return Err(StoreError::InvalidValue(
                "session is frozen by an Anywhere handoff".into(),
            ));
        }
        conn.execute(
            "UPDATE session SET archived = 0 WHERE id = ?1",
            [session_id],
        )?;
        Ok(())
    }

    /// Record the isolated worktree a daemon session runs in (`forge serve` with `worktree:true`).
    pub fn set_session_worktree(&self, session_id: &str, path: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET worktree_path = ?2 WHERE id = ?1",
            (session_id, path),
        )?;
        Ok(())
    }

    /// The isolated worktree recorded for a session, if any.
    pub fn session_worktree(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT worktree_path FROM session WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// A session's stored title, if any.
    pub fn session_title(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT title FROM session WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Full session ids whose id starts with `prefix` (git-style abbreviation). `prefix` is
    /// matched literally: any `%`/`_`/`\` it contains is escaped so it can't act as a SQL LIKE
    /// wildcard and broaden the match beyond a literal prefix.
    pub fn matching_session_ids(&self, prefix: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let escaped = escape_like_pattern(prefix);
        let mut stmt =
            conn.prepare("SELECT id FROM session WHERE id LIKE ?1 || '%' ESCAPE '\\'")?;
        let rows = stmt.query_map([escaped], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Whether a session with this id exists.
    pub fn session_exists(&self, session_id: &str) -> Result<bool> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }
}
