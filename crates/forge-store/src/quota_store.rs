//! Subscription quota snapshots, history, alias merging, and routing calibration.

use super::*;

impl Store {
    /// Record the latest subscription quota observation (quota-aware routing, L3). One row per
    /// bridge provider, upserted — the most recent `rate_limit_event` wins.
    ///
    /// Also appends the observation to the append-only `quota_history` table (mesh-routing.md)
    /// when a `fraction_used` is reported, so [`quota_history_since`](Self::quota_history_since) can
    /// later derive a consumption rate. This does NOT change `subscription_usage`'s upsert
    /// (latest-snapshot-only) semantics — it's a pure addition alongside it.
    pub fn record_quota(&self, hint: &forge_types::QuotaHint) -> Result<()> {
        let status = match hint.status {
            forge_types::QuotaStatus::Ok => "ok",
            forge_types::QuotaStatus::Warning => "warning",
            forge_types::QuotaStatus::Exhausted => "exhausted",
        };
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO subscription_usage (provider, window_kind, status, resets_at, fraction, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'))
                 ON CONFLICT(provider, window_kind) DO UPDATE SET
                   status = excluded.status,
                   resets_at = excluded.resets_at,
                   fraction = excluded.fraction,
                   updated_at = excluded.updated_at",
                (
                    hint.provider.as_str(),
                    hint.window.as_str(),
                    status,
                    hint.resets_at,
                    hint.fraction_used,
                ),
            )?;
            if let Some(fraction_used) = hint.fraction_used {
                tx.execute(
                    "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
                     VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
                    (hint.provider.as_str(), hint.window.as_str(), fraction_used, hint.resets_at),
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Append one privacy-preserving routed model-call outcome.  This is deliberately best-effort
    /// at call sites: a temporary SQLite lock must never turn an otherwise successful model turn
    /// into a user-visible failure.
    pub fn record_mesh_outcome(&self, record: &MeshOutcome) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO mesh_outcome
             (session_id, model, tier, started_at, completed_at, latency_ms, outcome, error_kind,
              failover_hop, tool_calls, verified_completion)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                &record.session_id,
                &record.model,
                record.tier.as_str(),
                record.started_at,
                record.completed_at,
                record.latency_ms.min(i64::MAX as u64) as i64,
                &record.outcome,
                &record.error_kind,
                record.failover_hop,
                record.tool_calls,
                record.verified_completion as i64,
            ),
        )?;
        Ok(())
    }

    /// Aggregate recent model outcomes for the Mesh.  The query uses only a bounded trailing
    /// sample per model, so a provider's long-past history cannot permanently dominate its current
    /// behaviour.  The router itself ignores groups below its minimum-sample gate.
    pub fn model_outcome_calibration(&self) -> Result<Vec<ModelOutcomeCalibration>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "WITH recent AS (
                 SELECT model, outcome, latency_ms,
                        ROW_NUMBER() OVER (PARTITION BY model ORDER BY completed_at DESC, id DESC) AS n
                 FROM mesh_outcome
             )
             SELECT model,
                    COUNT(*) AS samples,
                    AVG(CASE WHEN outcome = 'success' THEN 1.0 ELSE 0.0 END) AS success_rate,
                    AVG(latency_ms) AS mean_latency_ms
             FROM recent WHERE n <= 120 GROUP BY model",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ModelOutcomeCalibration {
                    model: row.get(0)?,
                    samples: row.get::<_, i64>(1)? as u32,
                    success_rate: row.get(2)?,
                    mean_latency_ms: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Persist one account-wide Codex quota observation for both supported Codex surfaces in one
    /// transaction. The observation timestamp controls alias precedence and history ordering.
    fn record_codex_quota_transaction(
        &self,
        hint: &forge_types::QuotaHint,
        observed_at: i64,
        mark_live_oauth: bool,
        plan: Option<&str>,
    ) -> Result<()> {
        let status = match hint.status {
            forge_types::QuotaStatus::Ok => "ok",
            forge_types::QuotaStatus::Warning => "warning",
            forge_types::QuotaStatus::Exhausted => "exhausted",
        };
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for provider in ["codex-oauth", "codex-cli"] {
                let changed = tx.execute(
                    "INSERT INTO subscription_usage (provider, window_kind, status, resets_at, fraction, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(provider, window_kind) DO UPDATE SET
                       status = excluded.status,
                       resets_at = excluded.resets_at,
                       fraction = excluded.fraction,
                       updated_at = excluded.updated_at
                     WHERE excluded.updated_at >= subscription_usage.updated_at",
                    (provider, hint.window.as_str(), status, hint.resets_at, hint.fraction_used, observed_at),
                )?;
                if changed > 0 {
                    if let Some(fraction_used) = hint.fraction_used {
                        tx.execute(
                            "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
                             SELECT ?1, ?2, ?3, ?4, ?5
                             WHERE NOT EXISTS (
                                 SELECT 1 FROM quota_history
                                 WHERE provider = ?1 AND window_kind = ?2 AND observed_at = ?5
                             )",
                            (provider, hint.window.as_str(), fraction_used, hint.resets_at, observed_at),
                        )?;
                    }
                }
            }
            if let Some(plan) = plan {
                for provider in ["codex-oauth", "codex-cli"] {
                    tx.execute(
                        "INSERT INTO subscription_plan_observation (provider, plan, observed_at)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT(provider) DO UPDATE SET plan = excluded.plan,
                           observed_at = excluded.observed_at
                         WHERE excluded.observed_at >= subscription_plan_observation.observed_at",
                        (provider, plan, observed_at),
                    )?;
                }
            }
            if mark_live_oauth {
                tx.execute(
                    "INSERT INTO codex_oauth_quota_observation (singleton, observed_at)
                     VALUES (1, ?1)
                     ON CONFLICT(singleton) DO UPDATE SET observed_at = excluded.observed_at",
                    [observed_at],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Persist one account-wide Codex quota observation for both supported Codex surfaces.
    ///
    /// The direct OAuth provider and Codex CLI bridge consume the same ChatGPT allowance.  They
    /// must therefore expose one identical snapshot to every Mesh/TUI/API consumer; recording a
    /// header reading under just the transport that happened to obtain it made the two surfaces
    /// disagree until their next unrelated completion.  This deliberately duplicates the same
    /// observation (rather than adding them) so alias-group reads retain their latest-wins
    /// semantics.
    pub fn record_codex_quota(&self, hint: &forge_types::QuotaHint) -> Result<()> {
        self.record_codex_quota_transaction(hint, chrono::Utc::now().timestamp(), false, None)
    }

    /// Persist a quota observation read directly from ChatGPT's Codex OAuth response headers.
    /// Unlike [`record_codex_quota`](Self::record_codex_quota), this advances the canonical source
    /// freshness marker and is therefore allowed to suppress another inexpensive probe briefly.
    pub fn record_live_codex_oauth_quota(&self, hint: &forge_types::QuotaHint) -> Result<()> {
        self.record_live_codex_account(hint, None)
    }

    /// Atomically persist a live Codex OAuth quota window, both shared aliases, the canonical
    /// freshness marker, and an optional backend-observed account plan.
    pub fn record_live_codex_account(
        &self,
        hint: &forge_types::QuotaHint,
        plan: Option<&str>,
    ) -> Result<()> {
        self.record_codex_quota_transaction(hint, chrono::Utc::now().timestamp(), true, plan)
    }

    /// Persist an official-Codex-CLI observation at its source observation time. Older rollout
    /// data cannot outrank a newer direct OAuth header reading.
    pub fn record_codex_quota_at(
        &self,
        hint: &forge_types::QuotaHint,
        observed_at: i64,
    ) -> Result<()> {
        self.record_codex_quota_transaction(hint, observed_at, false, None)
    }

    /// Age of the last direct OAuth header observation, independent from the shared CLI alias
    /// rows. `None` means no authoritative direct observation has ever been recorded.
    pub fn codex_oauth_quota_age_secs(&self) -> Result<Option<i64>> {
        let now = chrono::Utc::now().timestamp();
        self.lock()?
            .query_row(
                "SELECT ?1 - observed_at FROM codex_oauth_quota_observation WHERE singleton = 1",
                [now],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Append one observation to the quota usage history (mesh-routing.md). Called by
    /// [`record_quota`](Self::record_quota) for every hint that carries a `fraction_used`; exposed
    /// separately so callers/tests can seed history points directly (e.g. with a fixed
    /// `observed_at` via [`Self::record_quota_history_at`]).
    pub fn record_quota_history(
        &self,
        provider: &str,
        window: &str,
        fraction_used: f64,
        resets_at: Option<i64>,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
            (provider, window, fraction_used, resets_at),
        )?;
        Ok(())
    }

    /// [`record_quota`](Self::record_quota) with an explicit `updated_at` (epoch secs) — the
    /// OBSERVATION time, not the recording time. The seeding paths (codex rollout files, `forge
    /// mesh`) re-record old observations whenever their freshness gate reopens; stamping those
    /// with `now()` would let hours-old rollout data continually mask fresher `x-codex-*` header
    /// readings in the alias-group merge ([`quota_at`](Self::quota_at)'s latest-wins is only
    /// correct when `updated_at` means observation time).
    ///
    /// Guard: an incoming observation OLDER than the row's existing `updated_at` is a complete
    /// no-op (upsert rejected via the `ON CONFLICT ... WHERE`, history skipped) — a late-arriving
    /// stale observation can never regress a fresher reading, regardless of caller discipline.
    /// A duplicate history point (same provider/window/`observed_at`) is also skipped, so
    /// re-seeding the same rollout observation every few minutes doesn't grow `quota_history`.
    pub fn record_quota_at(&self, hint: &forge_types::QuotaHint, updated_at: i64) -> Result<()> {
        let status = match hint.status {
            forge_types::QuotaStatus::Ok => "ok",
            forge_types::QuotaStatus::Warning => "warning",
            forge_types::QuotaStatus::Exhausted => "exhausted",
        };
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let changed = tx.execute(
                "INSERT INTO subscription_usage (provider, window_kind, status, resets_at, fraction, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(provider, window_kind) DO UPDATE SET
                   status = excluded.status,
                   resets_at = excluded.resets_at,
                   fraction = excluded.fraction,
                   updated_at = excluded.updated_at
                 WHERE excluded.updated_at >= subscription_usage.updated_at",
                (
                    hint.provider.as_str(),
                    hint.window.as_str(),
                    status,
                    hint.resets_at,
                    hint.fraction_used,
                    updated_at,
                ),
            )?;
            if changed == 0 {
                return Ok(());
            }
            if let Some(fraction_used) = hint.fraction_used {
                tx.execute(
                    "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
                     SELECT ?1, ?2, ?3, ?4, ?5
                     WHERE NOT EXISTS (
                         SELECT 1 FROM quota_history
                         WHERE provider = ?1 AND window_kind = ?2 AND observed_at = ?5
                     )",
                    (
                        hint.provider.as_str(),
                        hint.window.as_str(),
                        fraction_used,
                        hint.resets_at,
                        updated_at,
                    ),
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// [`record_quota_history`](Self::record_quota_history) with an explicit `observed_at` (epoch
    /// secs) — a testable clock, mirroring [`quota_at`](Self::quota_at) for `current_quota`.
    pub fn record_quota_history_at(
        &self,
        provider: &str,
        window: &str,
        fraction_used: f64,
        resets_at: Option<i64>,
        observed_at: i64,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (provider, window, fraction_used, resets_at, observed_at),
        )?;
        Ok(())
    }

    /// History points for one provider+window, observed at or after `since` (epoch secs),
    /// oldest first — the input [`forge_types::compute_quota_pace`] needs to derive a rate.
    pub fn quota_history_since(
        &self,
        provider: &str,
        window: &str,
        since: i64,
    ) -> Result<Vec<forge_types::QuotaHistoryPoint>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT observed_at, fraction_used FROM quota_history
             WHERE provider = ?1 AND window_kind = ?2 AND observed_at >= ?3
             ORDER BY observed_at ASC",
        )?;
        let rows = stmt
            .query_map((provider, window, since), |row| {
                Ok(forge_types::QuotaHistoryPoint {
                    observed_at: row.get(0)?,
                    fraction_used: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Snapshot of currently-constraining subscription quotas (rows whose window hasn't reset),
    /// for the router. Only `Warning`/`Exhausted` providers are carried — `Ok` is the default.
    pub fn current_quota(&self) -> Result<forge_types::SubscriptionQuota> {
        self.quota_at(chrono::Utc::now().timestamp())
    }

    /// Seconds since the most recent quota update for `provider` (`None` if never recorded). Used
    /// to gate the on-demand claude rate-limit probe so it refreshes at most every few minutes.
    pub fn subscription_age_secs(&self, provider: &str) -> Option<i64> {
        let conn = self.lock().ok()?;
        let updated: Option<i64> = conn
            .query_row(
                "SELECT MAX(updated_at) FROM subscription_usage WHERE provider = ?1",
                [provider],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        updated.map(|u| chrono::Utc::now().timestamp() - u)
    }

    /// Persist the current plan observed from the Codex backend. This is deliberately a tiny
    /// account snapshot rather than configuration: callers may trust it only while it remains as
    /// fresh as the associated quota observation.
    pub fn record_subscription_plan(&self, provider: &str, plan: &str) -> Result<()> {
        self.record_subscription_plan_at(provider, plan, chrono::Utc::now().timestamp())
    }

    /// [`Self::record_subscription_plan`] at the actual observation time. Rollout-derived values
    /// must use this so an older Codex session line cannot make a stale plan look newly observed.
    pub fn record_subscription_plan_at(
        &self,
        provider: &str,
        plan: &str,
        observed_at: i64,
    ) -> Result<()> {
        let plan = plan.trim();
        if plan.is_empty() {
            return Ok(());
        }
        with_busy_retry(|| {
            let conn = self.lock()?;
            conn.execute(
                "INSERT INTO subscription_plan_observation (provider, plan, observed_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(provider) DO UPDATE SET plan = excluded.plan,
                 observed_at = excluded.observed_at
                 WHERE excluded.observed_at >= subscription_plan_observation.observed_at",
                (provider, plan, observed_at),
            )?;
            Ok(())
        })
    }

    /// A current server-observed plan, if one exists. Codex aliases share one account and use the
    /// same strict five-minute freshness limit as their quota; other providers have no live plan
    /// source today and therefore return no value unless a future caller writes one.
    pub fn fresh_subscription_plan(&self, provider: &str) -> Option<String> {
        self.fresh_subscription_plan_at(provider, chrono::Utc::now().timestamp())
    }

    /// [`Self::fresh_subscription_plan`] at an explicit clock value.
    pub fn fresh_subscription_plan_at(&self, provider: &str, now: i64) -> Option<String> {
        let members = quota_alias_members(provider);
        let placeholders = (0..members.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT plan, observed_at FROM subscription_plan_observation
             WHERE provider IN ({placeholders}) ORDER BY observed_at DESC LIMIT 1"
        );
        let conn = self.lock().ok()?;
        let mut stmt = conn.prepare(&sql).ok()?;
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(members.len());
        for member in &members {
            params.push(member);
        }
        let (plan, observed_at): (String, i64) = stmt
            .query_row(params.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .ok()
            .flatten()?;
        codex_quota_is_fresh(provider, observed_at, now).then_some(plan)
    }

    /// Per-provider, per-window fraction from `subscription_usage` (for display).
    /// Only returns non-stale rows (window hasn't reset yet or has no reset time). Alias-group
    /// members receive the same latest-per-window snapshot, exactly like [`Self::quota_at`], so
    /// `/usage`, `/mesh`, `forge mesh`, and routing cannot disagree about a shared Codex account.
    /// Returns `HashMap<provider, HashMap<window_kind, fraction>>`.
    pub fn bridge_fractions(
        &self,
    ) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, f64>>> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT provider, window_kind, fraction, updated_at FROM subscription_usage
         WHERE fraction IS NOT NULL AND (resets_at IS NULL OR resets_at > ?1)",
        )?;
        let rows: Vec<(String, String, f64, i64)> = stmt
            .query_map([now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .filter_map(std::result::Result::ok)
            .filter(|row| codex_quota_is_fresh(&row.0, row.3, now))
            .collect();

        let mut output_providers: std::collections::BTreeSet<&str> =
            std::collections::BTreeSet::new();
        for row in &rows {
            for member in quota_alias_members(&row.0) {
                output_providers.insert(member);
            }
        }

        let mut out = std::collections::HashMap::new();
        for provider in output_providers {
            let members = quota_alias_members(provider);
            let mut windows: std::collections::HashMap<String, &(String, String, f64, i64)> =
                std::collections::HashMap::new();
            for row in &rows {
                if !members.contains(&row.0.as_str()) {
                    continue;
                }
                windows
                    .entry(row.1.clone())
                    .and_modify(|existing| {
                        if row.3 > existing.3 {
                            *existing = row;
                        }
                    })
                    .or_insert(row);
            }
            if !windows.is_empty() {
                out.insert(
                    provider.to_string(),
                    windows
                        .into_iter()
                        .map(|(window, row)| (window, row.2))
                        .collect(),
                );
            }
        }
        Ok(out)
    }

    /// [`current_quota`](Self::current_quota) at an explicit `now` (epoch secs) — testable clock.
    ///
    /// `codex-cli` and `codex-oauth` bill the SAME ChatGPT account (mesh-routing.md), so their
    /// `subscription_usage`/`quota_history` rows are merged here at read time via
    /// [`QUOTA_ALIAS_GROUPS`] before the status/fraction/pace rollups run — see
    /// [`quota_alias_members`]. Non-grouped providers (e.g. `claude-cli`) are unaffected: a
    /// provider outside any group only ever merges with itself, which is a no-op.
    pub fn quota_at(&self, now: i64) -> Result<forge_types::SubscriptionQuota> {
        let conn = self.lock()?;

        struct UsageRow {
            provider: String,
            window: String,
            status: String,
            fraction: Option<f64>,
            resets_at: Option<i64>,
            updated_at: i64,
        }
        let raw_rows: Vec<UsageRow> = {
            let mut stmt = conn.prepare(
                "SELECT provider, window_kind, status, fraction, resets_at, updated_at
                 FROM subscription_usage
                 WHERE resets_at IS NULL OR resets_at > ?1",
            )?;
            let rows = stmt
                .query_map([now], |row| {
                    Ok(UsageRow {
                        provider: row.get(0)?,
                        window: row.get(1)?,
                        status: row.get(2)?,
                        fraction: row.get(3)?,
                        resets_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })?
                .filter_map(std::result::Result::ok)
                .filter(|row| codex_quota_is_fresh(&row.provider, row.updated_at, now))
                .collect();
            rows
        };

        // Every distinct provider seen, expanded to its full alias-group membership — so a group
        // member with zero rows of its own (e.g. codex-cli when only codex-oauth has reported
        // usage) still surfaces the group's shared reading.
        let mut output_providers: std::collections::BTreeSet<&str> =
            std::collections::BTreeSet::new();
        for row in &raw_rows {
            for m in quota_alias_members(&row.provider) {
                output_providers.insert(m);
            }
        }

        let mut map = std::collections::HashMap::new();
        let mut fractions = std::collections::HashMap::new();
        let mut paces = std::collections::HashMap::new();
        let since = now - forge_types::QUOTA_PACE_LOOKBACK_SECS;

        for provider in output_providers {
            let members = quota_alias_members(provider);

            // Merge per-window rows across every group member — these are server-authoritative
            // snapshots of the SAME account for a grouped provider, so the row with the latest
            // `updated_at` wins per window. NEVER summed (that would double-count headroom).
            let mut by_window: std::collections::HashMap<&str, &UsageRow> =
                std::collections::HashMap::new();
            for row in &raw_rows {
                if !members.contains(&row.provider.as_str()) {
                    continue;
                }
                by_window
                    .entry(row.window.as_str())
                    .and_modify(|existing| {
                        if row.updated_at > existing.updated_at {
                            *existing = row;
                        }
                    })
                    .or_insert(row);
            }
            if by_window.is_empty() {
                continue;
            }

            let worst_status = by_window
                .values()
                .map(|r| quota_status_from_str(&r.status))
                .max()
                .unwrap_or_default();
            if worst_status != forge_types::QuotaStatus::Ok {
                map.insert(provider.to_string(), worst_status);
            }

            // Strictest (max-fraction) window with a known fraction — also carried for still-Ok
            // providers so the router's graduated conservation can spread ahead of Warning. The
            // pace projection below must be derived for this SAME window, not just any window.
            if let Some(strictest) =
                by_window
                    .values()
                    .filter(|r| r.fraction.is_some())
                    .max_by(|a, b| {
                        a.fraction
                            .partial_cmp(&b.fraction)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            {
                let fraction = strictest.fraction.unwrap_or(0.0);
                fractions.insert(provider.to_string(), fraction);

                // Pace projection off that same strictest window's history, UNIONED across every
                // alias-group member (both surfaces may have recorded points for the same shared
                // account) — a subscription burning fast early in its window is otherwise
                // under-protected by `fractions` alone (mesh-routing.md).
                let history =
                    self.quota_history_union(&conn, &members, &strictest.window, since)?;
                if let Some(pace) =
                    forge_types::compute_quota_pace(&history, strictest.resets_at, now)
                {
                    paces.insert(provider.to_string(), pace);
                }
            }
        }

        Ok(forge_types::SubscriptionQuota::new(map)
            .with_fractions(fractions)
            .with_paces(paces))
    }

    /// History points for `window`, observed at or after `since`, unioned across every provider
    /// in `members` (ascending `observed_at`) — the shared-account merge [`quota_at`] needs so a
    /// grouped provider's pace reflects history recorded under either surface name.
    fn quota_history_union(
        &self,
        conn: &Connection,
        members: &[&str],
        window: &str,
        since: i64,
    ) -> Result<Vec<forge_types::QuotaHistoryPoint>> {
        let placeholders = (0..members.len())
            .map(|i| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT observed_at, fraction_used FROM quota_history
             WHERE window_kind = ?1 AND observed_at >= ?2 AND provider IN ({placeholders})
             ORDER BY observed_at ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&window, &since];
        for m in members {
            params.push(m);
        }
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(forge_types::QuotaHistoryPoint {
                    observed_at: row.get(0)?,
                    fraction_used: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
