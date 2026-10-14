//! Usage rows with prompt-cache attribution, and the per-model rollup behind `forge stats cache`.

use super::*;

/// What a main-loop request shared with the previous one to the same model (migration #41).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsagePrefix {
    /// `None` when the call was not fingerprinted (a CLI bridge reports cumulative usage).
    pub reused_msgs: Option<u32>,
    pub total_msgs: Option<u32>,
    pub tools_changed: Option<bool>,
    /// `append`, `harness_rewrite@<idx>(<kind>)`, `floor_advance`, `n/a`, ...
    pub label: String,
}

/// One model's cache behaviour over a window, from main-loop usage rows.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheModelStats {
    pub model: String,
    pub calls: u64,
    /// Calls that carry a prefix fingerprint; zero for bridges and pre-#41 rows.
    pub fingerprinted: u64,
    pub input_tokens: u64,
    /// `None` when no row for this model reports prompt-cache hits.
    pub cached_input_tokens: Option<u64>,
    /// Input tokens of the rows that do report caching, minus what they read from cache.
    pub uncached_input_tokens: u64,
    /// Miss labels with the `@<idx>(<kind>)` detail stripped, most frequent first.
    pub labels: Vec<(String, u64)>,
}

impl CacheModelStats {
    /// Cached share of the input of the rows that report caching.
    pub fn hit_ratio(&self) -> Option<f64> {
        let cached = self.cached_input_tokens?;
        let reported = cached + self.uncached_input_tokens;
        (reported > 0).then(|| cached as f64 / reported as f64)
    }
}

impl Store {
    /// [`Store::record_usage`] plus the prompt-cache attribution of the call, when known.
    pub fn record_usage_with_prefix(
        &self,
        session_id: &str,
        message_id: &str,
        usage: &Usage,
        model: Option<&str>,
        prefix: Option<&UsagePrefix>,
    ) -> Result<()> {
        let id = forge_types::new_id();
        let model = model.map(str::trim).filter(|m| !m.is_empty());
        let provider = model.and_then(|m| m.split_once("::")).map(|(ns, _)| ns);
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO usage
                 (id, message_id, provider, model, input_tokens, cached_input_tokens, output_tokens,
                  cost_usd, prefix_reused_msgs, prefix_total_msgs, tools_changed, cache_label,
                  cache_write_tokens, reasoning_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                (
                    &id,
                    message_id,
                    provider,
                    model,
                    usage.input_tokens as i64,
                    usage.cached_input_tokens.map(|c| c as i64),
                    usage.output_tokens as i64,
                    usage.cost_usd,
                    prefix.and_then(|p| p.reused_msgs).map(i64::from),
                    prefix.and_then(|p| p.total_msgs).map(i64::from),
                    prefix.and_then(|p| p.tools_changed),
                    prefix.map(|p| p.label.as_str()),
                    usage.cache_write_tokens.map(|c| c as i64),
                    usage.reasoning_tokens.map(|c| c as i64),
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

    /// Per-model cache rollup of main-loop calls (side calls anchor on system rows and are left
    /// out) created at or after `since` (epoch seconds), optionally for one session.
    pub fn cache_stats(
        &self,
        session_id: Option<&str>,
        since: i64,
    ) -> Result<Vec<CacheModelStats>> {
        let conn = self.lock()?;
        let mut totals = conn.prepare(
            "SELECT COALESCE(u.model, m.model, '?'), COUNT(*), COUNT(u.prefix_total_msgs),
                    COALESCE(SUM(u.input_tokens), 0), SUM(u.cached_input_tokens),
                    COALESCE(SUM(CASE WHEN u.cached_input_tokens IS NOT NULL
                                      THEN MAX(u.input_tokens - u.cached_input_tokens, 0) END), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.role = 'assistant' AND u.created_at >= ?1
               AND (?2 IS NULL OR m.session_id = ?2)
             GROUP BY 1 ORDER BY 4 DESC",
        )?;
        let mut stats: Vec<CacheModelStats> = totals
            .query_map((since, session_id), |r| {
                Ok(CacheModelStats {
                    model: r.get(0)?,
                    calls: r.get::<_, i64>(1)?.max(0) as u64,
                    fingerprinted: r.get::<_, i64>(2)?.max(0) as u64,
                    input_tokens: r.get::<_, i64>(3)?.max(0) as u64,
                    cached_input_tokens: r.get::<_, Option<i64>>(4)?.map(|c| c.max(0) as u64),
                    uncached_input_tokens: r.get::<_, i64>(5)?.max(0) as u64,
                    labels: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut labels = conn.prepare(
            "SELECT COALESCE(u.model, m.model, '?'),
                    CASE WHEN instr(u.cache_label, '@') > 0
                         THEN substr(u.cache_label, 1, instr(u.cache_label, '@') - 1)
                         ELSE u.cache_label END,
                    COUNT(*)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.role = 'assistant' AND u.created_at >= ?1 AND u.cache_label IS NOT NULL
               AND (?2 IS NULL OR m.session_id = ?2)
             GROUP BY 1, 2 ORDER BY 3 DESC",
        )?;
        let rows = labels.query_map((since, session_id), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?.max(0) as u64,
            ))
        })?;
        for row in rows {
            let (model, label, count) = row?;
            if let Some(entry) = stats.iter_mut().find(|s| s.model == model) {
                entry.labels.push((label, count));
            }
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, cached: Option<u64>) -> Usage {
        Usage {
            reported_cost_usd: None,
            input_tokens: input,
            output_tokens: 10,
            cached_input_tokens: cached,
            cost_usd: 0.0,
            cache_write_tokens: None,
            reasoning_tokens: None,
        }
    }

    fn assistant_row(store: &Store, session: &str, seq: i64) -> String {
        store
            .add_message(session, seq, forge_types::Role::Assistant, "a", None)
            .unwrap()
    }

    #[test]
    fn prefix_columns_round_trip_and_roll_up_per_model() {
        let store = Store::open_in_memory().unwrap();
        let session = store.create_session("/tmp", "Default").unwrap();
        let model = "bedrock::kimi";
        let rows = [
            (1000, Some(900), "append"),
            (1000, Some(100), "harness_rewrite@4(tool)"),
            (1000, Some(0), "harness_rewrite@7(tool)"),
            (1000, Some(950), "floor_advance"),
        ];
        for (seq, (input, cached, label)) in rows.into_iter().enumerate() {
            let msg = assistant_row(&store, &session, seq as i64);
            let prefix = UsagePrefix {
                reused_msgs: Some(3),
                total_msgs: Some(5),
                tools_changed: Some(false),
                label: label.to_string(),
            };
            store
                .record_usage_with_prefix(
                    &session,
                    &msg,
                    &usage(input, cached),
                    Some(model),
                    Some(&prefix),
                )
                .unwrap();
        }
        let msg = assistant_row(&store, &session, 10);
        store
            .record_usage(
                &session,
                &msg,
                &usage(500, None),
                Some("claude-cli::sonnet"),
            )
            .unwrap();

        let stats = store.cache_stats(Some(&session), 0).unwrap();
        let kimi = stats.iter().find(|s| s.model == model).unwrap();
        assert_eq!(kimi.calls, 4);
        assert_eq!(kimi.fingerprinted, 4);
        assert_eq!(kimi.cached_input_tokens, Some(1950));
        assert_eq!(kimi.uncached_input_tokens, 2050);
        assert_eq!(kimi.labels[0], ("harness_rewrite".to_string(), 2));
        assert!((kimi.hit_ratio().unwrap() - 0.4875).abs() < 1e-9);

        let bridge = stats
            .iter()
            .find(|s| s.model == "claude-cli::sonnet")
            .unwrap();
        assert_eq!(bridge.fingerprinted, 0);
        assert_eq!(bridge.cached_input_tokens, None);
        assert_eq!(bridge.hit_ratio(), None);
        assert!(bridge.labels.is_empty());
    }
}
