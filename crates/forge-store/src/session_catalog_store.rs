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

    /// Search every top-level, used session by title, id, cwd, and active user/assistant content.
    ///
    /// The query is executed and ranked in SQLite and returns a bounded result set. This avoids
    /// the old client-side behavior where search only saw whichever 50-row history page happened
    /// to be loaded. Message search deliberately excludes tool/system rows: they are often huge,
    /// can contain secrets or generated noise, and are not part of the user-facing thread.
    pub fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<SessionSearchRow>> {
        let query = query.trim();
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let escaped = escape_like_pattern(query);
        let contains = format!("%{escaped}%");
        let prefix = format!("{escaped}%");
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "WITH message_matches AS (
                 SELECT m.session_id, MAX(m.seq) AS matched_seq
                 FROM message m
                 WHERE m.active = 1
                   AND m.role IN ('user', 'assistant')
                   AND m.content LIKE :contains ESCAPE '\\'
                 GROUP BY m.session_id
             )
             SELECT s.id, s.title, s.cwd, s.archived,
                    (SELECT COUNT(*) FROM message count_m
                     WHERE count_m.session_id = s.id AND count_m.active = 1),
                    s.total_cost_usd,
                    COALESCE((SELECT MAX(activity_m.created_at) FROM message activity_m
                              WHERE activity_m.session_id = s.id), s.created_at) AS last_activity,
                    CASE
                      WHEN COALESCE(s.title, '') LIKE :contains ESCAPE '\\' THEN 'title'
                      WHEN s.id LIKE :contains ESCAPE '\\' THEN 'id'
                      WHEN s.cwd LIKE :contains ESCAPE '\\' THEN 'cwd'
                      ELSE 'message'
                    END AS match_source,
                    matched.seq,
                    matched.role,
                    CASE WHEN matched.content IS NULL THEN NULL ELSE
                      substr(
                        matched.content,
                        max(1, instr(lower(matched.content), lower(:query)) - 80),
                        240
                      )
                    END AS match_excerpt
             FROM session s
             LEFT JOIN message_matches mm ON mm.session_id = s.id
             LEFT JOIN message matched
               ON matched.session_id = s.id AND matched.seq = mm.matched_seq
             WHERE s.parent_session_id IS NULL
               AND EXISTS (
                 SELECT 1 FROM message used
                 WHERE used.session_id = s.id AND used.role = 'user'
               )
               AND (
                 COALESCE(s.title, '') LIKE :contains ESCAPE '\\'
                 OR s.id LIKE :contains ESCAPE '\\'
                 OR s.cwd LIKE :contains ESCAPE '\\'
                 OR mm.matched_seq IS NOT NULL
               )
             ORDER BY
               CASE
                 WHEN lower(COALESCE(s.title, '')) = lower(:query) THEN 0
                 WHEN COALESCE(s.title, '') LIKE :prefix ESCAPE '\\' THEN 1
                 WHEN COALESCE(s.title, '') LIKE :contains ESCAPE '\\' THEN 2
                 WHEN s.id LIKE :prefix ESCAPE '\\' THEN 3
                 WHEN s.cwd LIKE :contains ESCAPE '\\' THEN 4
                 ELSE 5
               END,
               last_activity DESC,
               s.rowid DESC
             LIMIT :limit",
        )?;
        let rows = stmt.query_map(
            rusqlite::named_params! {
                ":query": query,
                ":contains": contains,
                ":prefix": prefix,
                ":limit": i64::try_from(limit).unwrap_or(i64::MAX),
            },
            |row| {
                Ok(SessionSearchRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    cwd: row.get(2)?,
                    archived: row.get::<_, i64>(3)? != 0,
                    message_count: row.get(4)?,
                    total_cost_usd: row.get(5)?,
                    last_activity: row.get(6)?,
                    match_source: row.get(7)?,
                    match_seq: row.get(8)?,
                    match_role: row.get(9)?,
                    match_excerpt: row.get(10)?,
                })
            },
        )?;
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

    /// Permanently delete a persisted session, its subagent descendants, and session-scoped
    /// artifacts. Queue history is retained but detached from the deleted session.
    ///
    /// The daemon owns the running/worktree safety checks; the store still refuses frozen
    /// Anywhere handoffs anywhere in the tree so a lifecycle command cannot invalidate an
    /// in-flight transfer.
    pub fn delete_session(&self, session_id: &str) -> Result<bool> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let session_ids = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE session_tree(id) AS (
                     SELECT id FROM session WHERE id = ?1
                     UNION ALL
                     SELECT child.id
                     FROM session child
                     JOIN session_tree parent ON child.parent_session_id = parent.id
                 )
                 SELECT id FROM session_tree",
            )?;
            let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if session_ids.is_empty() {
            return Ok(false);
        }
        for id in &session_ids {
            let blocked: bool = tx.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM anywhere_handoff_session_state WHERE session_id=?1
                 )",
                [id],
                |row| row.get(0),
            )?;
            if blocked {
                return Err(StoreError::InvalidValue(
                    "session tree is frozen by an Anywhere handoff".into(),
                ));
            }
        }
        for id in &session_ids {
            // These older operational tables intentionally have no foreign key. Do not leave
            // session-scoped credentials/telemetry behind after a permanent deletion.
            tx.execute(
                "DELETE FROM live_activity_token WHERE session_id = ?1",
                [id],
            )?;
            tx.execute("DELETE FROM mesh_outcome WHERE session_id = ?1", [id])?;
            // Queue rows are user-visible execution history, not owned session artifacts.
            tx.execute(
                "UPDATE queue_task SET session_id = NULL WHERE session_id = ?1",
                [id],
            )?;
        }
        for id in session_ids.iter().rev() {
            tx.execute("DELETE FROM session WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(true)
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

    /// The model a session is pinned to, if it was created with a pin.
    ///
    /// A pin is a standing instruction to use exactly one model, so it has to outlive the running
    /// driver: resuming a session with the pin dropped hands the turn back to mesh classification,
    /// which can answer on a different model than the session was pinned to.
    pub fn session_pinned_model(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT pinned_model FROM session WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Record (or clear, with `None`) the model a session is pinned to.
    pub fn set_session_pinned_model(&self, session_id: &str, model: Option<&str>) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET pinned_model = ?2 WHERE id = ?1",
            rusqlite::params![session_id, model],
        )?;
        Ok(())
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
