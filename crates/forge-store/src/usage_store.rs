//! Focused persistence operations.

use super::*;

/// One session's provider calls summed per model, split by whether the call recorded real spend.
/// The bench reads this to price subscription/free calls at list rates ("notional" cost) without
/// touching the calls that were really billed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelUsageRow {
    pub model: Option<String>,
    /// The calls in this row recorded `cost_usd = 0` (subscription, free route or unpriced model).
    pub zero_cost: bool,
    pub calls: u64,
    pub usage: Usage,
}

impl Store {
    /// The latest observation of every subscription window, for the usage surfaces. Unlike
    /// [`Self::subscription_windows`] this keeps an observation past its routing freshness gate
    /// (Codex's is five minutes) so the user still sees the last reading with its age; only a
    /// window whose reset has passed is dropped, because its reading no longer describes anything.
    pub fn subscription_windows_for_display(&self, now: i64) -> Result<Vec<SubscriptionWindow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT provider, window_kind, status, resets_at, fraction, updated_at
             FROM subscription_usage WHERE resets_at IS NULL OR resets_at > ?1
             ORDER BY provider, window_kind",
        )?;
        let rows = stmt.query_map([now], |row| {
            Ok(SubscriptionWindow {
                provider: row.get(0)?,
                window_kind: row.get(1)?,
                status: row.get(2)?,
                resets_at: row.get(3)?,
                fraction: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Provider-consumed usage across the complete session ledger.
    ///
    /// Unlike [`Store::session_tokens`], this includes inactive synthetic side calls (recap,
    /// suggestion, memory, compaction, diagnosis) and calls belonging to later-deactivated turns.
    /// Those calls still consumed provider quota and remain part of honest billing/benchmark
    /// accounting even though they no longer contribute to the active transcript.
    pub fn session_token_usage(&self, session_id: &str) -> Result<Usage> {
        let conn = self.lock()?;
        // `COUNT(col)` skips NULLs, so `reporting == 0` means not one call in the session came
        // from a provider that reports cache hits — the session total is unknown, not zero. When
        // only some calls reported, the sum covers exactly those and the rest stay uncounted.
        type Sums = (
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            f64,
        );
        let (input, cached, cache_write, reasoning, output, cost): Sums = conn.query_row(
            "SELECT SUM(u.input_tokens), SUM(u.cached_input_tokens), SUM(u.cache_write_tokens),
                    SUM(u.reasoning_tokens), SUM(u.output_tokens), COALESCE(SUM(u.cost_usd), 0.0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1",
            [session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        // SQL `SUM` over only-NULL values is NULL, so an optional total stays "not reported" unless
        // at least one call in the session reported it.
        let count = |v: Option<i64>| v.map(|n| n.max(0) as u64);
        Ok(Usage {
            reported_cost_usd: None,
            input_tokens: count(input).unwrap_or(0),
            cached_input_tokens: count(cached),
            output_tokens: count(output).unwrap_or(0),
            cost_usd: cost.max(0.0),
            cache_write_tokens: count(cache_write),
            reasoning_tokens: count(reasoning),
        })
    }

    /// Every provider call in a session (side calls and deactivated turns included, as in
    /// [`Store::session_token_usage`]) grouped by model and by zero/non-zero recorded cost. A
    /// usage row counts as the model round trips it stands for (`usage.api_calls`, else one).
    pub fn session_usage_by_model(&self, session_id: &str) -> Result<Vec<ModelUsageRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT COALESCE(u.model, m.model) AS mdl, u.cost_usd = 0 AS zero,
                    SUM(COALESCE(u.api_calls, 1)),
                    SUM(u.input_tokens), SUM(u.cached_input_tokens), SUM(u.cache_write_tokens),
                    SUM(u.reasoning_tokens), SUM(u.output_tokens), COALESCE(SUM(u.cost_usd), 0.0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1
             GROUP BY mdl, zero
             ORDER BY mdl, zero",
        )?;
        let count = |v: Option<i64>| v.map(|n| n.max(0) as u64);
        let rows = stmt
            .query_map([session_id], |r| {
                Ok(ModelUsageRow {
                    model: r.get(0)?,
                    zero_cost: r.get(1)?,
                    calls: r.get::<_, i64>(2)?.max(0) as u64,
                    usage: Usage {
                        reported_cost_usd: None,
                        input_tokens: count(r.get(3)?).unwrap_or(0),
                        cached_input_tokens: count(r.get(4)?),
                        cache_write_tokens: count(r.get(5)?),
                        reasoning_tokens: count(r.get(6)?),
                        output_tokens: count(r.get(7)?).unwrap_or(0),
                        cost_usd: r.get::<_, f64>(8)?.max(0.0),
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Record that the usage row(s) of `message_id` stand for `calls` model round trips (a
    /// CLI-bridge turn runs its whole agent loop inside one provider call). A no-op when the
    /// message has no usage row.
    pub fn set_usage_api_calls(&self, message_id: &str, calls: u64) -> Result<()> {
        self.lock()?.execute(
            "UPDATE usage SET api_calls = ?1 WHERE message_id = ?2",
            (calls as i64, message_id),
        )?;
        Ok(())
    }

    /// Number of tool calls recorded in a session, across every turn.
    pub fn session_tool_call_count(&self, session_id: &str) -> Result<u64> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM tool_call t JOIN message m ON m.id = t.message_id
             WHERE m.session_id = ?1",
            [session_id],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u64)
    }

    /// Record usage for a side call (compact, diagnose) that has no corresponding agent message.
    /// Inserts a synthetic inactive system message as the FK anchor, then the usage row, and
    /// bumps the session total so daily/monthly budget queries (which read `usage`) stay accurate.
    pub fn record_side_call_usage(
        &self,
        session_id: &str,
        label: &str,
        usage: &Usage,
    ) -> Result<()> {
        self.record_side_call_usage_for(session_id, label, None, usage)
    }

    /// [`Store::record_side_call_usage`], naming the model that answered. Without it a side call's
    /// row cannot say which summarizer returned nothing. The anchor message carries the model too:
    /// per-provider usage is grouped on `message.model`, and a NULL there made a side call count
    /// against whichever provider happened to own the neighbouring turn.
    pub fn record_side_call_usage_for(
        &self,
        session_id: &str,
        label: &str,
        model: Option<&str>,
        usage: &Usage,
    ) -> Result<()> {
        let attributed = self.attributed_model(session_id, model);
        let model = attributed.as_deref().or(model);
        let provider = model.and_then(|m| m.split_once("::")).map(|(p, _)| p);
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
                    "INSERT INTO message (id, session_id, seq, role, content, model, active) \
                     VALUES (?1, ?2, ?3, 'system', ?4, ?5, 0)",
                    (msg_id.as_str(), session_id, s, label, model),
                );
                match r {
                    Ok(_) => break,
                    Err(ref e) if is_unique_violation(e) => s += 1,
                    Err(e) => return Err(StoreError::Sqlite(e)),
                }
            }
            tx.execute(
                "INSERT INTO usage \
                 (id, message_id, provider, model, input_tokens, cached_input_tokens, \
                  output_tokens, cost_usd, cache_write_tokens, reasoning_tokens) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                (
                    &usage_id,
                    msg_id.as_str(),
                    provider,
                    model,
                    usage.input_tokens as i64,
                    usage.cached_input_tokens.map(|c| c as i64),
                    usage.output_tokens as i64,
                    usage.cost_usd,
                    usage.cache_write_tokens.map(|c| c as i64),
                    usage.reasoning_tokens.map(|c| c as i64),
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
                "model": model,
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
                "provider": provider,
                "model": model,
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cached_input_tokens": usage.cached_input_tokens,
                "cost_usd": usage.cost_usd,
                "created_at": tx.query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |r| r.get::<_, i64>(0))?,
                "cache_write_tokens": usage.cache_write_tokens,
                "reasoning_tokens": usage.reasoning_tokens,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_detail_columns_round_trip_and_group_by_model_and_zero_cost() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let billed = store
            .add_message(&sid, 0, Role::Assistant, "a", Some("bedrock::kimi-k3"))
            .unwrap();
        store
            .record_usage(
                &sid,
                &billed,
                &Usage {
                    reported_cost_usd: None,
                    input_tokens: 1_000,
                    output_tokens: 100,
                    cached_input_tokens: Some(600),
                    cost_usd: 0.25,
                    cache_write_tokens: Some(300),
                    reasoning_tokens: Some(40),
                },
                Some("bedrock::kimi-k3"),
            )
            .unwrap();
        for seq in 1..=2 {
            let free = store
                .add_message(&sid, seq, Role::Assistant, "b", Some("claude-cli::sonnet"))
                .unwrap();
            store
                .record_usage(
                    &sid,
                    &free,
                    &Usage {
                        input_tokens: 500,
                        output_tokens: 50,
                        ..Default::default()
                    },
                    Some("claude-cli::sonnet"),
                )
                .unwrap();
            store
                .record_tool_call(&free, "read_file", "{}", "ok", "allowed", "ok")
                .unwrap();
        }

        let rows = store.session_usage_by_model(&sid).unwrap();
        assert_eq!(rows.len(), 2);
        let bedrock = rows
            .iter()
            .find(|r| r.model.as_deref() == Some("bedrock::kimi-k3"))
            .unwrap();
        assert!(!bedrock.zero_cost);
        assert_eq!(bedrock.calls, 1);
        assert_eq!(bedrock.usage.cache_write_tokens, Some(300));
        assert_eq!(bedrock.usage.reasoning_tokens, Some(40));
        let bridge = rows
            .iter()
            .find(|r| r.model.as_deref() == Some("claude-cli::sonnet"))
            .unwrap();
        assert!(bridge.zero_cost);
        assert_eq!((bridge.calls, bridge.usage.input_tokens), (2, 1_000));
        assert_eq!(
            bridge.usage.cache_write_tokens, None,
            "never reported → unknown"
        );
        assert_eq!(store.session_tool_call_count(&sid).unwrap(), 2);

        let total = store.session_token_usage(&sid).unwrap();
        assert_eq!(total.cache_write_tokens, Some(300));
        assert_eq!(total.reasoning_tokens, Some(40));
        assert_eq!(total.input_tokens, 2_000);
    }

    #[test]
    fn migration_42_adds_nullable_token_detail_columns_to_an_existing_store() {
        let path = std::env::temp_dir().join(format!("forge-m42-{}.db", forge_types::new_id()));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(schema::SCHEMA).unwrap();
            conn.execute_batch(
                "ALTER TABLE usage DROP COLUMN cache_write_tokens;
                 ALTER TABLE usage DROP COLUMN reasoning_tokens;",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 40i64).unwrap();
        }
        for pass in ["first open (migrates)", "second open (idempotent)"] {
            let store = Store::open(&path).unwrap_or_else(|e| panic!("{pass}: {e:?}"));
            let conn = store.lock().unwrap();
            let columns: Vec<(String, i64)> = conn
                .prepare(
                    "SELECT name, \"notnull\" FROM pragma_table_info('usage')
                     WHERE name IN ('cache_write_tokens', 'reasoning_tokens') ORDER BY name",
                )
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(
                columns,
                vec![
                    ("cache_write_tokens".to_string(), 0),
                    ("reasoning_tokens".to_string(), 0)
                ],
                "{pass}"
            );
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn a_bridge_row_counts_as_the_model_calls_it_stands_for() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 10,
            ..Default::default()
        };
        let looped = store
            .add_message(&sid, 0, Role::Assistant, "a", Some("claude-cli::sonnet"))
            .unwrap();
        store
            .record_usage(&sid, &looped, &usage, Some("claude-cli::sonnet"))
            .unwrap();
        let plain = store
            .add_message(&sid, 1, Role::Assistant, "b", Some("claude-cli::sonnet"))
            .unwrap();
        store
            .record_usage(&sid, &plain, &usage, Some("claude-cli::sonnet"))
            .unwrap();
        let calls = || store.session_usage_by_model(&sid).unwrap()[0].calls;
        assert_eq!(
            calls(),
            2,
            "one call per row until the bridge says otherwise"
        );
        store.set_usage_api_calls(&looped, 7).unwrap();
        assert_eq!(calls(), 8, "7 round trips in one row + 1 in the other");
        store.set_usage_api_calls("no-such-message", 3).unwrap();
        assert_eq!(calls(), 8);
    }

    #[test]
    fn migration_46_adds_a_nullable_api_calls_column_to_an_existing_store() {
        let path = std::env::temp_dir().join(format!("forge-m46-{}.db", forge_types::new_id()));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(schema::SCHEMA).unwrap();
            conn.execute_batch("ALTER TABLE usage DROP COLUMN api_calls;")
                .unwrap();
            conn.pragma_update(None, "user_version", 45i64).unwrap();
        }
        for pass in ["first open (migrates)", "second open (idempotent)"] {
            let store = Store::open(&path).unwrap_or_else(|e| panic!("{pass}: {e:?}"));
            let conn = store.lock().unwrap();
            let notnull: i64 = conn
                .query_row(
                    "SELECT \"notnull\" FROM pragma_table_info('usage') WHERE name = 'api_calls'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or_else(|e| panic!("{pass}: column missing: {e}"));
            assert_eq!(notnull, 0, "{pass}");
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
