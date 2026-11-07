//! Session retention: pruning never-used and stale sessions, and the user-initiated vacuum.

use super::*;

impl Store {
    /// Delete up to `max_sessions` sessions that have NEVER received a real (role='user') message —
    /// checked regardless of `active`, so a session whose sole user message was later soft-deleted
    /// by `/undo` or a checkpoint restore (which only flips `active`, it never removes the row) is
    /// still correctly recognized as having been used — and were created more than `horizon_secs`
    /// ago (oldest first) — separate from [`Store::prune`]'s much longer general retention horizon,
    /// since an empty session carries nothing worth keeping at all. A process that spawns a session
    /// and exits (or crashes) before the user ever sends a prompt — e.g. an `mcp agent` connection
    /// that's opened and torn down without being used — otherwise leaves a permanent, empty row that
    /// clutters `forge sessions` / the resume picker forever. Returns the number removed.
    ///
    /// A session someone has open is not unused just because nobody typed yet. A daemon-hosted
    /// session (`daemon_live`) or one a terminal beat for within the horizon is skipped, exactly
    /// as [`Store::prune`] skips them: this sweep runs once in EVERY Forge process, so without the
    /// check a freshly started `forge run` deleted a live, still-empty session out from under the
    /// chat or daemon holding it.
    pub fn prune_empty(&self, horizon_secs: i64, max_sessions: usize) -> Result<usize> {
        if max_sessions == 0 {
            return Ok(0);
        }
        let cutoff = chrono::Utc::now().timestamp() - horizon_secs;
        with_busy_retry(|| {
            let conn = self.lock()?;
            let ids: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM session s WHERE s.created_at < ?1 AND s.agent_active = 0 \
                     AND s.daemon_live = 0 \
                     AND (s.local_live = 0 OR COALESCE(s.local_last_seen, 0) < ?1) \
                     AND NOT EXISTS ( \
                       SELECT 1 FROM message m \
                       WHERE m.session_id = s.id AND m.role = 'user' \
                     ) ORDER BY s.created_at LIMIT ?2",
                )?;
                let v = stmt
                    .query_map((cutoff, max_sessions as i64), |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            };
            for id in &ids {
                // ON DELETE CASCADE clears the dependent rows (messages → usage/routing/tool_call).
                // The search index has no foreign key; it resolves rowids through `message`.
                Store::drop_recall_index(&conn, id)?;
                conn.execute("DELETE FROM session WHERE id = ?1", [id])?;
            }
            Ok(ids.len())
        })
    }

    /// Extra stale candidates `prune` reads so sessions skipped for a live worktree do not hide
    /// prunable ones behind them.
    const PRUNE_WORKTREE_SLACK: usize = 64;

    /// Delete up to `max_sessions` sessions whose `updated_at` is older than `horizon_secs` ago
    /// (oldest first), cascading to their messages/usage/routing/tool_calls/live_events/tasks. The
    /// retention unit is a whole stale session, never individual rows of a live one, so an active
    /// transcript is never partially pruned. Returns the number of sessions removed.
    pub fn prune(&self, horizon_secs: i64, max_sessions: usize) -> Result<usize> {
        if max_sessions == 0 {
            return Ok(0);
        }
        let cutoff = chrono::Utc::now().timestamp() - horizon_secs;
        with_busy_retry(|| {
            let conn = self.lock()?;
            let ids: Vec<String> = {
                let mut stmt = conn.prepare(
                    // `local_live` / `daemon_live` are as load-bearing here as `agent_active`.
                    // insert_message does NOT bump `updated_at` (only record_usage and the mode /
                    // view-snapshot writers do), so a session resumed in a terminal after a long
                    // idle still carries its OLD updated_at. Without these two predicates, any
                    // other Forge process calling create_session — which piggy-backs a prune —
                    // could cascade-delete a session the user is actively typing into, taking its
                    // whole transcript, usage and tool-call audit with it. ensure_session can
                    // restore the bare parent row afterwards but NOT the history.
                    // A `kill -9`'d `forge mcp agent` never clears its flag; once a session is a
                    // week past the horizon the flag is a leftover, not a live agent.
                    // Starred is the user saying "keep this". A session whose worktree is still on
                    // disk owns unmerged work; deleting its row strands the worktree and branch with
                    // nothing pointing at them. Both are skipped (the worktree check needs the
                    // filesystem, so it runs below over a few extra candidates).
                    "SELECT id, worktree_path FROM session WHERE updated_at < ?1 \
                     AND (agent_active = 0 OR updated_at < ?3) \
                     AND local_live = 0 AND daemon_live = 0 AND starred = 0 \
                     ORDER BY updated_at LIMIT ?2",
                )?;
                let v = stmt
                    .query_map(
                        (
                            cutoff,
                            (max_sessions + Self::PRUNE_WORKTREE_SLACK) as i64,
                            cutoff - 7 * 86_400,
                        ),
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v.into_iter()
                    .filter(|(_, worktree)| {
                        !worktree
                            .as_deref()
                            .is_some_and(|path| std::path::Path::new(path).is_dir())
                    })
                    .map(|(id, _)| id)
                    .take(max_sessions)
                    .collect()
            };
            for id in &ids {
                // ON DELETE CASCADE clears the dependent rows (messages → usage/routing/tool_call).
                // The search index has no foreign key; it resolves rowids through `message`.
                Store::drop_recall_index(&conn, id)?;
                conn.execute("DELETE FROM session WHERE id = ?1", [id])?;
            }
            Ok(ids.len())
        })
    }

    /// Reclaim free pages and checkpoint the WAL: `VACUUM` rebuilds the database file, then a
    /// truncating checkpoint shrinks the `-wal`. Both are safe in WAL mode with no open write
    /// transaction.
    ///
    /// MUST stay user-initiated. `VACUUM` rewrites the whole file — minutes of I/O and roughly the
    /// database's own size in temporary free space on a multi-GB store — while holding a write lock
    /// the entire time, which would stall every other Forge process (and `forge serve`'s live
    /// sessions) at an unpredictable moment. Its one caller is therefore the explicit
    /// `forge lattice prune --vacuum`, run by a user who has just deleted a lot of rows and knows to
    /// do it with no other Forge process running. Routine WAL growth needs no such stall: it is
    /// bounded automatically on every connection by [`WAL_SIZE_LIMIT_BYTES`].
    pub fn vacuum(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch("VACUUM")?;
        // Truncating WAL checkpoint to shrink the -wal file (no-op / harmless on in-memory DBs).
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        Ok(())
    }
}
