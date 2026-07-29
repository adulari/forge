//! Store operations for sessions, messages, usage, and spending.

use super::*;

impl Store {
    /// Create a new session row and return its id.
    pub fn create_session(&self, cwd: &str, mode: &str) -> Result<String> {
        let id = forge_types::new_id();
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO session (id, cwd, permission_mode, total_cost_usd) VALUES (?1, ?2, ?3, 0)",
                (&id, cwd, mode),
            )?;
            append_session_snapshot(&tx, &id)?;
            tx.commit()?;
            Ok(())
        })?;
        // Opportunistic, bounded retention sweep so the global append-only DB doesn't grow forever.
        // Best-effort: a prune failure must never block opening a session.
        let _ = self.prune(RETENTION_HORIZON_SECS, PRUNE_BATCH);
        let _ = self.prune_empty(EMPTY_SESSION_HORIZON_SECS, EMPTY_PRUNE_BATCH);
        Ok(id)
    }

    /// Restore a session parent row if a live driver outlasted retention. Existing session
    /// metadata is deliberately left untouched: this is a narrow durability guard for writes
    /// that reference `session_id`, not a session update operation.
    pub fn ensure_session(&self, id: &str, cwd: &str, mode: &str) -> Result<()> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let inserted = tx.execute(
                "INSERT INTO session (id, cwd, permission_mode, total_cost_usd) VALUES (?1, ?2, ?3, 0) \
                 ON CONFLICT(id) DO NOTHING",
                (id, cwd, mode),
            )?;
            if inserted == 1 {
                append_session_snapshot(&tx, id)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// The working directory recorded for a session at creation time, or `None` if no such
    /// session exists. Unlike `SessionRegistry::get` (the daemon's in-memory map of currently
    /// running drivers), this reads straight from the store — like [`Store::load_history_page`]
    /// it works for ANY persisted session, live or not, so a historical image can still be served
    /// after a daemon restart or once the session's driver has wound down.
    pub fn session_cwd(&self, id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        match conn.query_row("SELECT cwd FROM session WHERE id = ?1", [id], |r| r.get(0)) {
            Ok(cwd) => Ok(Some(cwd)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete up to `max_sessions` sessions that have NEVER received a real (role='user') message —
    /// checked regardless of `active`, so a session whose sole user message was later soft-deleted
    /// by `/undo` or a checkpoint restore (which only flips `active`, it never removes the row) is
    /// still correctly recognized as having been used — and were created more than `horizon_secs`
    /// ago (oldest first) — separate from [`Store::prune`]'s much longer general retention horizon,
    /// since an empty session carries nothing worth keeping at all. A process that spawns a session
    /// and exits (or crashes) before the user ever sends a prompt — e.g. an `mcp agent` connection
    /// that's opened and torn down without being used — otherwise leaves a permanent, empty row that
    /// clutters `forge sessions` / the resume picker forever. Returns the number removed.
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
                conn.execute("DELETE FROM session WHERE id = ?1", [id])?;
            }
            Ok(ids.len())
        })
    }

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
                    "SELECT id FROM session WHERE updated_at < ?1 AND agent_active = 0 \
                     ORDER BY updated_at LIMIT ?2",
                )?;
                let v = stmt
                    .query_map((cutoff, max_sessions as i64), |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            };
            for id in &ids {
                // ON DELETE CASCADE clears the dependent rows (messages → usage/routing/tool_call).
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

    /// Create a subagent child session linked to `parent_id` (RFC subagent-orchestration).
    pub fn create_child_session(&self, cwd: &str, mode: &str, parent_id: &str) -> Result<String> {
        let id = forge_types::new_id();
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO session (id, cwd, permission_mode, total_cost_usd, parent_session_id) \
                 VALUES (?1, ?2, ?3, 0, ?4)",
                (&id, cwd, mode, parent_id),
            )?;
            append_session_snapshot(&tx, &id)?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(id)
    }

    /// A session's stored permission mode (temper) string.
    pub fn session_mode(&self, session_id: &str) -> Result<String> {
        Ok(self.lock()?.query_row(
            "SELECT permission_mode FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Update a session's permission mode (temper) — persisted when the user switches it live.
    pub fn update_session_mode(&self, session_id: &str, mode: &str) -> Result<()> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if tx.execute(
                "UPDATE session SET permission_mode = ?2, updated_at = strftime('%s','now') WHERE id = ?1",
                (session_id, mode),
            )? == 1
            {
                append_session_snapshot(&tx, session_id)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// A session's persisted TUI view snapshot (opaque JSON), if one was saved. Used to restore the
    /// exact on-screen state (activity panel, viewer, scroll) when the session is resumed.
    pub fn session_view_snapshot(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self.lock()?.query_row(
            "SELECT view_snapshot FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Persist a session's TUI view snapshot (opaque JSON). Written at the end of each completed
    /// turn and on clean exit so a resume restores the screen as of the last prompt.
    pub fn update_session_view_snapshot(&self, session_id: &str, json: &str) -> Result<()> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if tx.execute(
                "UPDATE session SET view_snapshot = ?2, updated_at = strftime('%s','now') WHERE id = ?1",
                (session_id, json),
            )? == 1
            {
                append_session_snapshot(&tx, session_id)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Ids of the subagent child sessions spawned by `parent_id`, oldest first.
    pub fn child_sessions(&self, parent_id: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM session WHERE parent_session_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([parent_id], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// Name a session — for a subagent child, the resolved agent name (`title` doubles as the
    /// child's address for `send_to_agent`; top-level sessions title themselves elsewhere).
    pub fn set_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if tx.execute(
                "UPDATE session SET title = ?2 WHERE id = ?1",
                (session_id, title),
            )? == 1
            {
                append_session_snapshot(&tx, session_id)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// `(id, title)` of `parent_id`'s child sessions, oldest first — the address book
    /// `send_to_agent` resolves against (title = the agent name recorded at spawn).
    pub fn named_child_sessions(&self, parent_id: &str) -> Result<Vec<(String, Option<String>)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, title FROM session WHERE parent_session_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([parent_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// The next free message `seq` for a session (0 for an empty one) — lets a follow-up turn
    /// append to a child session without replaying its whole insert history.
    pub fn next_message_seq(&self, session_id: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            "SELECT COALESCE(MAX(seq) + 1, 0) FROM message WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Append a message to a session and return its id.
    pub fn add_message(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
    ) -> Result<String> {
        self.add_message_full(session_id, seq, role, content, model, &[], None)
    }

    /// Append a UI-only note: persisted (so it survives resume and shows in replay/scrollback)
    /// but tagged `visibility='ui'` so the context pipeline never sends it to a model.
    pub fn add_ui_note(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
    ) -> Result<String> {
        self.insert_message(
            session_id,
            seq,
            role,
            content,
            None,
            &[],
            None,
            Visibility::UiOnly,
        )
    }

    /// Append a message, including any tool-call linkage (assistant tool calls / tool
    /// result ids), so the transcript round-trips faithfully on resume.
    #[allow(clippy::too_many_arguments)]
    pub fn add_message_full(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
        tool_calls: &[ToolCall],
        tool_call_id: Option<&str>,
    ) -> Result<String> {
        self.insert_message(
            session_id,
            seq,
            role,
            content,
            model,
            tool_calls,
            tool_call_id,
            Visibility::Llm,
        )
    }

    /// Append provider-continuity transcript that must not appear as a user-facing reply (for
    /// example a provisional completion while Forge performs its verification continuation).
    #[allow(clippy::too_many_arguments)]
    pub fn add_llm_only_message_full(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
        tool_calls: &[ToolCall],
        tool_call_id: Option<&str>,
    ) -> Result<String> {
        self.insert_message(
            session_id,
            seq,
            role,
            content,
            model,
            tool_calls,
            tool_call_id,
            Visibility::LlmOnly,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_message(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
        tool_calls: &[ToolCall],
        tool_call_id: Option<&str>,
        visibility: Visibility,
    ) -> Result<String> {
        let id = forge_types::new_id();
        let tool_calls_json = if tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(tool_calls).unwrap_or_default())
        };
        // IMMEDIATE so the write lock is taken up front (no read-snapshot upgrade), bounded-retried
        // on transient busy, and self-healing on a seq collision: if `seq` is already taken (two
        // writers raced on `next_seq_for_session`), the UNIQUE(session_id, seq) index rejects it and
        // we re-allocate MAX(seq)+1 inside the same transaction rather than scrambling order.
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut s = seq;
            loop {
                let r = tx.execute(
                    "INSERT INTO message (id, session_id, seq, role, content, model, tool_calls_json, tool_call_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(session_id, seq) DO NOTHING",
                    (&id, session_id, s, role.as_str(), content, model, &tool_calls_json, tool_call_id, visibility.as_str()),
                );
                match r {
                    Ok(1) => break,
                    Ok(0) => {
                        s = tx.query_row(
                            "SELECT COALESCE(MAX(seq), -1) + 1 FROM message WHERE session_id = ?1",
                            [session_id],
                            |row| row.get(0),
                        )?;
                    }
                    Ok(_) => unreachable!("INSERT can affect at most one row"),
                    Err(e) => return Err(StoreError::Sqlite(e)),
                }
            }
            let payload = sync_json(serde_json::json!({
                "id": id,
                "session_id": session_id,
                "seq": s,
                "role": role.as_str(),
                "content": content,
                "model": model,
                "tool_calls": tool_calls,
                "tool_call_id": tool_call_id,
                "visibility": visibility.as_str(),
            }))?;
            append_sync_revision(&tx, "message", &id, SyncJournalOperation::Upsert, &payload)?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Record the Mesh's routing decision for a message.
    pub fn record_routing(
        &self,
        message_id: &str,
        tier: TaskTier,
        chosen_model: &str,
        rationale: &str,
    ) -> Result<()> {
        let id = forge_types::new_id();
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.prepare_cached(
                "INSERT INTO routing_decision (id, message_id, task_tier, chosen_model, rationale)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute((&id, message_id, tier.as_str(), chosen_model, rationale))?;
            let payload = sync_json(serde_json::json!({
                "id": id,
                "message_id": message_id,
                "task_tier": tier.as_str(),
                "chosen_model": chosen_model,
                "rationale": rationale,
            }))?;
            append_sync_revision(
                &tx,
                "routing_decision",
                &id,
                SyncJournalOperation::Upsert,
                &payload,
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Record token usage/cost for a message and bump the session's running total.
    /// Batched in one explicit transaction so the INSERT + UPDATE land in a single WAL commit.
    pub fn record_usage(&self, session_id: &str, message_id: &str, usage: &Usage) -> Result<()> {
        let id = forge_types::new_id();
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO usage
                 (id, message_id, input_tokens, cached_input_tokens, output_tokens, cost_usd)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    &id,
                    message_id,
                    usage.input_tokens as i64,
                    usage.cached_input_tokens as i64,
                    usage.output_tokens as i64,
                    usage.cost_usd,
                ),
            )?;
            tx.execute(
                "UPDATE session SET total_cost_usd = total_cost_usd + ?1,
                 updated_at = strftime('%s','now') WHERE id = ?2",
                (usage.cost_usd, session_id),
            )?;
            let payload = sync_json(serde_json::json!({
                "id": id,
                "message_id": message_id,
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cached_input_tokens": usage.cached_input_tokens,
                "cost_usd": usage.cost_usd,
            }))?;
            append_sync_revision(&tx, "usage", &id, SyncJournalOperation::Upsert, &payload)?;
            append_session_snapshot(&tx, session_id)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Record a tool call and its permission outcome.
    pub fn record_tool_call(
        &self,
        message_id: &str,
        tool_name: &str,
        args_json: &str,
        result: &str,
        permission: &str,
        status: &str,
    ) -> Result<()> {
        // Extracted from the UNCAPPED args string, before the cap below can truncate the tail
        // and clip a late `path` key out of the JSON.
        let path = extract_path_arg(args_json);
        // Cap oversized args/results (full file writes/reads etc.) so the append-only global DB
        // can't grow without bound; the head is preserved with a truncation marker for audit/replay.
        let args_json = cap_result_json(args_json);
        let result = cap_result_json(result);
        let id = forge_types::new_id();
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.prepare_cached(
                "INSERT INTO tool_call (id, message_id, tool_name, args_json, result_json, permission, status, path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute((&id, message_id, tool_name, args_json.as_ref(), result.as_ref(), permission, status, path.as_deref()))?;
            let payload = sync_json(serde_json::json!({
                "id": id,
                "message_id": message_id,
                "tool_name": tool_name,
                "args_json": args_json.as_ref(),
                "result_json": result.as_ref(),
                "permission": permission,
                "status": status,
                "path": path,
            }))?;
            append_sync_revision(
                &tx,
                "tool_call",
                &id,
                SyncJournalOperation::Upsert,
                &payload,
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Current running cost of a session (the per-session meter — unchanged).
    pub fn session_cost(&self, session_id: &str) -> Result<f64> {
        Ok(self.lock()?.query_row(
            "SELECT total_cost_usd FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Provider-consumed usage across the complete session ledger.
    ///
    /// Unlike [`Store::session_tokens`], this includes inactive synthetic side calls (recap,
    /// suggestion, memory, compaction, diagnosis) and calls belonging to later-deactivated turns.
    /// Those calls still consumed provider quota and remain part of honest billing/benchmark
    /// accounting even though they no longer contribute to the active transcript.
    pub fn session_token_usage(&self, session_id: &str) -> Result<Usage> {
        let conn = self.lock()?;
        let (input, cached, output, cost): (i64, i64, i64, f64) = conn.query_row(
            "SELECT COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.cached_input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0),
                    COALESCE(SUM(u.cost_usd), 0.0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        Ok(Usage {
            input_tokens: input.max(0) as u64,
            cached_input_tokens: cached.max(0) as u64,
            output_tokens: output.max(0) as u64,
            cost_usd: cost.max(0.0),
        })
    }

    /// `(input_tokens, output_tokens)` summed across a session's `usage` rows — the live token
    /// counter (tui-token-counter.md).
    pub fn session_tokens(&self, session_id: &str) -> Result<(u64, u64)> {
        let conn = self.lock()?;
        let (i, o): (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1 AND m.active = 1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((i.max(0) as u64, o.max(0) as u64))
    }

    /// Provider-reported prompt tokens served from cache across active calls in a session.
    /// This is a subset of `input_tokens`, not an additional token charge.
    pub fn session_cached_input_tokens(&self, session_id: &str) -> Result<u64> {
        let conn = self.lock()?;
        let cached: i64 = conn.query_row(
            "SELECT COALESCE(SUM(u.cached_input_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1 AND m.active = 1",
            [session_id],
            |r| r.get(0),
        )?;
        Ok(cached.max(0) as u64)
    }

    /// Number of provider calls (model steps) recorded in a session — one `usage` row per call.
    /// The Lattice benchmark uses this as the "steps" metric: fewer tool-exploration round-trips
    /// means fewer steps and fewer tokens.
    pub fn session_step_count(&self, session_id: &str) -> Result<u64> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1 AND m.active = 1",
            [session_id],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u64)
    }

    /// Models the Mesh routed to within a session (chosen_model per routing_decision), oldest
    /// first. Used to verify subagents route independently of the parent.
    pub fn session_models(&self, session_id: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT r.chosen_model FROM routing_decision r \
             JOIN message m ON m.id = r.message_id \
             WHERE m.session_id = ?1 ORDER BY m.seq",
        )?;
        let rows = stmt.query_map([session_id], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// Per-provider usage since a rolling epoch timestamp.
    pub fn usage_by_provider_since(&self, since_epoch: i64) -> Result<Vec<ProviderUsage>> {
        self.usage_query("WHERE u.created_at >= ?1", [since_epoch])
    }

    pub fn usage_by_provider_for_session(&self, session_id: &str) -> Result<Vec<ProviderUsage>> {
        let conn = self.lock()?;
        // Provider derived from `message.model` (see `usage_query`) — `usage.provider` is NULL.
        let sql = format!(
            "SELECT {USAGE_PROVIDER_EXPR} AS prov, COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.cached_input_tokens), 0), COALESCE(SUM(u.output_tokens), 0), COALESCE(SUM(u.cost_usd), 0.0) \
             FROM usage u JOIN message m ON m.id = u.message_id WHERE m.session_id = ?1 GROUP BY prov \
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens + u.output_tokens) DESC"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(ProviderUsage {
                provider: r.get(0)?,
                input_tokens: r.get::<_, i64>(1)? as u64,
                cached_input_tokens: r.get::<_, i64>(2)? as u64,
                output_tokens: r.get::<_, i64>(3)? as u64,
                cost_usd: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn usage_query<P: rusqlite::Params>(
        &self,
        predicate: &str,
        params: P,
    ) -> Result<Vec<ProviderUsage>> {
        let conn = self.lock()?;
        // Derive the provider from the routed model on the linked message: the `usage.provider`
        // column is never populated at insert, so grouping on it collapsed every row into one NULL
        // bucket (and `r.get::<String>` then failed on NULL → the whole page read as "no usage").
        // `message.model` IS populated (e.g. `codex-oauth::gpt-5.6-terra`); the namespace before
        // `::` is the provider. GROUP BY the alias `prov`, never a bare `provider` (that binds to
        // the still-NULL column, not this expression).
        let sql = format!("SELECT {USAGE_PROVIDER_EXPR} AS prov, COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.cached_input_tokens), 0), COALESCE(SUM(u.output_tokens), 0), COALESCE(SUM(u.cost_usd), 0.0) FROM usage u JOIN message m ON m.id = u.message_id {predicate} GROUP BY prov ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens + u.output_tokens) DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params, |r| {
            Ok(ProviderUsage {
                provider: r.get(0)?,
                input_tokens: r.get::<_, i64>(1)? as u64,
                cached_input_tokens: r.get::<_, i64>(2)? as u64,
                output_tokens: r.get::<_, i64>(3)? as u64,
                cost_usd: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn subscription_windows(&self) -> Result<Vec<SubscriptionWindow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT provider, window_kind, status, resets_at, fraction FROM subscription_usage WHERE resets_at IS NULL OR resets_at > ?1")?;
        let rows = stmt.query_map([chrono::Utc::now().timestamp()], |r| {
            Ok(SubscriptionWindow {
                provider: r.get(0)?,
                window_kind: r.get(1)?,
                status: r.get(2)?,
                resets_at: r.get(3)?,
                fraction: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// This is the authoritative budget figure (FR-5): it aggregates `usage.cost_usd` across
    /// every session, not one session's running total.
    pub fn spend_between(&self, start: i64, end: i64) -> Result<f64> {
        Ok(self.lock()?.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage \
             WHERE created_at >= ?1 AND created_at < ?2",
            (start, end),
            |row| row.get(0),
        )?)
    }
}
