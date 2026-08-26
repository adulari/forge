//! Durable model health, capability, context-window, and pricing state.

use super::*;

impl Store {
    // --- Model health / failover (docs/features/mesh-routing.md) ---

    /// Bench a model until `cooldown_until` (epoch secs), recording why. Upsert: a fresh failure
    /// or probe overwrites any prior bench.
    pub fn bench_model(&self, model: &str, cooldown_until: i64, reason: &str) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO model_health (model, cooldown_until, reason, updated_at)
             VALUES (?1, ?2, ?3, strftime('%s','now'))
             ON CONFLICT(model) DO UPDATE SET
               cooldown_until = excluded.cooldown_until,
               reason = excluded.reason,
               updated_at = excluded.updated_at",
            (model, cooldown_until, reason),
        )?;
        Ok(())
    }

    /// Bench a model for `cooldown` from now (convenience over [`bench_model`] that owns the
    /// clock, like [`spend_today_usd`](Self::spend_today_usd)).
    pub fn bench_for(
        &self,
        model: &str,
        cooldown: std::time::Duration,
        reason: &str,
    ) -> Result<()> {
        let until = chrono::Utc::now().timestamp() + cooldown.as_secs() as i64;
        self.bench_model(model, until, reason)
    }

    /// Exclude a model that failed *permanently* (no tool-calling support, unaffordable, malformed
    /// tool payload — see [`ProviderError::Capability`](forge_provider::ProviderError::Capability)).
    /// Modeled as a long bench window so it reuses the `model_health` table and naturally
    /// *re-probes* after the window elapses (a provider may add tool support later). The reason is
    /// prefixed `excluded:` so the UI / report can distinguish it from a transient bench.
    pub fn exclude_model(&self, model: &str, reason: &str) -> Result<()> {
        let until = chrono::Utc::now().timestamp() + CAPABILITY_EXCLUSION_SECS;
        self.bench_model(model, until, &format!("excluded: {reason}"))
    }

    /// Exclude every current and future model alias for `provider` after an authentication
    /// failure. Auth is credential-wide, so retrying a sibling model only creates failover churn;
    /// the provider bench naturally expires and is cleared by a successful probe after re-login.
    pub fn exclude_provider(&self, provider: &str, reason: &str) -> Result<()> {
        self.exclude_model(
            &forge_types::provider_bench_key(provider),
            &format!("provider auth failed: {reason}"),
        )
    }

    /// The non-excluded model whose bench expires soonest (the "least dead" model), as a
    /// last-resort fallback when every routable model is currently benched but none is a permanent
    /// capability exclusion. `None` when nothing is benched or all benches are permanent
    /// exclusions. Used by the core loop so a turn never hard-fails while a transient bench exists.
    pub fn soonest_unbenched(&self) -> Result<Option<String>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT model FROM model_health
                 WHERE reason NOT LIKE 'excluded:%'
                 ORDER BY cooldown_until ASC LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(row)
    }

    /// All transiently-benched (non-excluded) models, soonest-recovering first. The caller
    /// applies its own filter (e.g. drop providers with no key) before picking a last-resort
    /// model — `soonest_unbenched` can't, since the store has no notion of key presence.
    pub fn transient_benched_ordered(&self) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model FROM model_health
             WHERE reason NOT LIKE 'excluded:%'
             ORDER BY cooldown_until ASC",
        )?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    /// Currently-benched snapshot as of *now* (convenience over [`benched_models`]).
    pub fn current_benched(&self) -> Result<forge_types::ModelHealth> {
        self.benched_models(chrono::Utc::now().timestamp())
    }

    /// Currently-benched detailed report as of *now* (convenience over [`benched_report`]).
    pub fn current_benched_report(&self) -> Result<Vec<(String, i64, String)>> {
        self.benched_report(chrono::Utc::now().timestamp())
    }

    /// Clear any bench on a model (e.g. a healthy probe). No-op if it wasn't benched.
    pub fn clear_model_health(&self, model: &str) -> Result<()> {
        self.lock()?
            .execute("DELETE FROM model_health WHERE model = ?1", [model])?;
        Ok(())
    }

    /// Clear a provider-wide auth bench after any of its models passes a probe.
    pub fn clear_provider_health(&self, provider: &str) -> Result<()> {
        self.clear_model_health(&forge_types::provider_bench_key(provider))
    }

    /// Persist a model's fetched context window (tokens), from a provider's model API. Upsert so a
    /// later discovery refreshes it.
    pub fn set_model_context(&self, model: &str, window: u32) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO model_context (model, window, updated_at) VALUES (?1, ?2, strftime('%s','now'))
             ON CONFLICT(model) DO UPDATE SET window = excluded.window, updated_at = excluded.updated_at",
            (model, window),
        )?;
        Ok(())
    }

    /// A model's fetched context window (tokens), or `None` if we never stored one. The core
    /// prefers this over the family heuristic when bounding a turn's transcript.
    pub fn model_context(&self, model: &str) -> Result<Option<u32>> {
        let row = self
            .lock()?
            .query_row(
                "SELECT window FROM model_context WHERE model = ?1",
                [model],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok(row.map(|w| w.max(0) as u32))
    }

    /// Every known context-window size: `model -> tokens`. Fed into the mesh router so it can skip
    /// models whose window is smaller than the current transcript.
    pub fn all_model_contexts(&self) -> Result<std::collections::HashMap<String, u32>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT model, window FROM model_context")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (model, window) = row.map_err(|error| {
                StoreError::InvalidValue(format!(
                    "failed to decode model context row while loading routing windows: {error}"
                ))
            })?;
            map.insert(model, window.max(0) as u32);
        }
        Ok(map)
    }

    /// Persist a model's fetched USD price (per 1k tokens), from a provider's model API. Upsert so a
    /// later discovery refreshes it. `cache_read_per_1k` is the discounted prompt-cache-read rate
    /// (None if the provider didn't report one).
    pub fn set_model_pricing(
        &self,
        model: &str,
        input_per_1k: f64,
        output_per_1k: f64,
        cache_read_per_1k: Option<f64>,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO model_pricing (model, input_per_1k, output_per_1k, cache_read_per_1k, updated_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))
             ON CONFLICT(model) DO UPDATE SET input_per_1k = excluded.input_per_1k,
                 output_per_1k = excluded.output_per_1k, cache_read_per_1k = excluded.cache_read_per_1k,
                 updated_at = excluded.updated_at",
            (model, input_per_1k, output_per_1k, cache_read_per_1k),
        )?;
        Ok(())
    }

    /// Every fetched per-model price: `model -> (input_per_1k, output_per_1k, cache_read_per_1k)` in
    /// USD. Fed into the mesh's `Pricing` as overrides so gateway/credit spend is tracked, not $0.
    pub fn all_model_pricing(&self) -> Result<Vec<ModelPriceRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, input_per_1k, output_per_1k, cache_read_per_1k FROM model_pricing",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Clear every model bench (the `forge models --clear` rescan reset). Returns the number of
    /// benched rows removed so the caller can report it.
    pub fn clear_all_model_health(&self) -> Result<usize> {
        Ok(self.lock()?.execute("DELETE FROM model_health", [])?)
    }

    /// Snapshot of models still benched as of `now` (epoch secs) — cooldown not yet elapsed.
    pub fn benched_models(&self, now: i64) -> Result<forge_types::ModelHealth> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare_cached("SELECT model FROM model_health WHERE cooldown_until > ?1")?;
        let set = stmt
            .query_map([now], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<std::collections::HashSet<_>, _>>()?;
        Ok(forge_types::ModelHealth::new(set))
    }

    /// Detailed view of currently-benched models (model, cooldown_until, reason) for the CLI /
    /// startup hint, newest cooldown first.
    pub fn benched_report(&self, now: i64) -> Result<Vec<(String, i64, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, cooldown_until, reason FROM model_health
             WHERE cooldown_until > ?1 ORDER BY cooldown_until DESC",
        )?;
        let rows = stmt
            .query_map([now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
