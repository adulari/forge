//! Durable model health, capability, context-window, and pricing state.

use super::*;

/// Marker embedded in every auth-caused health reason, so an auth exclusion can be told apart from
/// a capability exclusion or a transient bench without re-deriving provider keys.
const AUTH_REASON: &str = "auth failed";

/// How long an authentication exclusion (either scope) keeps a model or provider out of routing.
///
/// Deliberately much shorter than [`CAPABILITY_EXCLUSION_SECS`]: an incapable model is incapable
/// for as long as the provider says so, whereas an auth verdict is a GUESS about a credential made
/// from one failed call, and nothing in the runtime re-probes it. The two errors are not
/// symmetric. A genuinely dead credential costs one failed-over turn per window — cheap, because
/// failover is automatic and the turn still completes elsewhere. A wrong verdict costs the user
/// their best subscription for the whole window, with no signal at all. Thirty minutes bounds the
/// damage of being wrong while still suppressing per-turn churn against a truly bad credential.
const AUTH_EXCLUSION_SECS: i64 = 30 * 60;

/// Drop auth exclusions written under a classification rule this build no longer implements.
///
/// **Decision (do releases invalidate incompatible health rows? yes, for auth rows only.)** A
/// `model_health` row is a cached VERDICT, not an observation, and a release that changes how
/// verdicts are reached leaves rows behind that the new rules would never have produced — they go
/// on suppressing a provider for up to a day after the fix ships, which is precisely how a fixed
/// bug keeps costing the user. The rule is self-describing rather than version-stamped: an auth
/// exclusion whose window is longer than [`AUTH_EXCLUSION_SECS`] cannot have been written by this
/// build, so it came from the superseded 24 h provider-wide rule and is discarded. Idempotent, and
/// scoped to auth rows only — a capability exclusion or a rate-limit bench is an observation about
/// the model that this release did not change, and those must survive.
pub(super) fn retire_superseded_auth_exclusions(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM model_health
         WHERE reason LIKE ?1 AND cooldown_until - updated_at > ?2",
        (format!("%{AUTH_REASON}%"), AUTH_EXCLUSION_SECS),
    )?;
    Ok(())
}

impl Store {
    // --- Model health / failover (docs/features/mesh-routing.md) ---

    /// Bench a model until `cooldown_until` (epoch secs), recording why. Upsert: a fresh failure
    /// or probe overwrites any prior bench.
    pub fn bench_model(&self, model: &str, cooldown_until: i64, reason: &str) -> Result<()> {
        self.bench_model_at(
            model,
            cooldown_until,
            reason,
            chrono::Utc::now().timestamp(),
        )
    }

    /// [`bench_model`](Self::bench_model) with an explicit `updated_at`, like
    /// [`record_quota_at`](Self::record_quota_at). `updated_at` is load-bearing, not bookkeeping:
    /// [`provider_auth_failed_before`](Self::provider_auth_failed_before) reads it to tell a repeat
    /// auth failure from a burst of concurrent ones.
    pub fn bench_model_at(
        &self,
        model: &str,
        cooldown_until: i64,
        reason: &str,
        updated_at: i64,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO model_health (model, cooldown_until, reason, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(model) DO UPDATE SET
               cooldown_until = excluded.cooldown_until,
               reason = excluded.reason,
               updated_at = excluded.updated_at",
            (model, cooldown_until, reason, updated_at),
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

    /// Exclude a single model after an authentication failure.
    ///
    /// The default scope for an auth failure: one failed turn is not evidence that a whole
    /// subscription's credential is dead. Uses the short [`AUTH_EXCLUSION_SECS`] window rather than
    /// the 24 h capability window — see [`exclude_provider`](Self::exclude_provider).
    pub fn exclude_model_auth(&self, model: &str, reason: &str) -> Result<()> {
        let until = chrono::Utc::now().timestamp() + AUTH_EXCLUSION_SECS;
        self.bench_model(model, until, &format!("excluded: {AUTH_REASON}: {reason}"))
    }

    /// Exclude every current and future model alias for `provider` after an authentication
    /// failure. Auth is credential-wide, so retrying a sibling model only creates failover churn.
    ///
    /// Reserved for CORROBORATED evidence — a dedicated liveness probe failing, or a repeat auth
    /// failure separated in time from the first (see `record_model_failure` in forge-core). A
    /// single failed turn writes a model-scope [`exclude_model_auth`](Self::exclude_model_auth)
    /// instead: a bridge that exits abnormally for an unrelated reason still prints something
    /// auth-shaped, and benching an entire healthy subscription on that costs far more than one
    /// extra failed-over turn.
    pub fn exclude_provider(&self, provider: &str, reason: &str) -> Result<()> {
        let until = chrono::Utc::now().timestamp() + AUTH_EXCLUSION_SECS;
        self.bench_model(
            &forge_types::provider_bench_key(provider),
            until,
            &format!("excluded: provider {AUTH_REASON}: {reason}"),
        )
    }

    /// Whether an auth exclusion for `provider` (at either scope) was already recorded at or before
    /// `cutoff` and is still in force.
    ///
    /// The corroboration test for escalating to provider scope. `cutoff` is deliberately in the
    /// past: concurrent turns against one provider fail together within the same second, so
    /// "another auth failure exists" alone would let a single burst escalate anyway. Requiring the
    /// earlier failure to be measurably older distinguishes a repeat from a burst.
    pub fn provider_auth_failed_before(&self, provider: &str, cutoff: i64) -> Result<bool> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model FROM model_health
             WHERE cooldown_until > ?1 AND updated_at <= ?2 AND reason LIKE ?3",
        )?;
        let provider_key = forge_types::provider_bench_key(provider);
        // Provider matching happens in Rust, not in a LIKE pattern: provider names legitimately
        // contain `_` (`opencode_go`), which is a LIKE wildcard.
        let mut rows = stmt.query(rusqlite::params![now, cutoff, format!("%{AUTH_REASON}%")])?;
        while let Some(row) = rows.next()? {
            let model: String = row.get(0)?;
            if model == provider_key
                || model
                    .split_once("::")
                    .is_some_and(|(prefix, _)| prefix == provider)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Every provider currently excluded provider-wide: `(provider, cooldown_until, reason)`.
    ///
    /// Provider rows are keyed `__forge_provider__::<name>`, so any diagnostic that filters model
    /// health by a bare provider prefix silently omits them — which is exactly how a benched
    /// subscription stayed invisible for a day. Surfaces use this instead of re-deriving the key.
    pub fn excluded_providers(&self, now: i64) -> Result<Vec<(String, i64, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, cooldown_until, reason FROM model_health
             WHERE cooldown_until > ?1 AND model LIKE ?2
             ORDER BY cooldown_until DESC",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![now, format!("{}%", forge_types::PROVIDER_BENCH_PREFIX)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(model, until, reason)| {
                let provider = model
                    .strip_prefix(forge_types::PROVIDER_BENCH_PREFIX)
                    .unwrap_or(&model)
                    .to_string();
                (provider, until, reason)
            })
            .collect())
    }

    /// Currently provider-wide-excluded providers as of *now* (convenience over
    /// [`excluded_providers`](Self::excluded_providers)).
    pub fn current_excluded_providers(&self) -> Result<Vec<(String, i64, String)>> {
        self.excluded_providers(chrono::Utc::now().timestamp())
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

    /// Retire every health record invalidated by `model` having just answered successfully.
    ///
    /// A completed turn is strictly stronger evidence than the liveness ping `forge models
    /// --probe` sends, and it is free. Without this the only things that ever cleared a
    /// provider-wide exclusion were that undiscoverable probe command and `forge auth`, so a wrong
    /// verdict outlived every piece of evidence against it. Clearing the provider scope too is the
    /// point: a pinned `forge run --model <provider>::<model>` now retires the exclusion the user
    /// just disproved by hand.
    pub fn clear_health_after_success(&self, model: &str) -> Result<()> {
        self.clear_model_health(model)?;
        if let Some((provider, _)) = model.split_once("::") {
            self.clear_provider_health(provider)?;
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    /// The naming trap. Provider rows are keyed `__forge_provider__::<name>`, so a diagnostic that
    /// filters model health on a bare provider prefix reports the subscription as perfectly healthy
    /// while every one of its aliases is out of routing. `current_excluded_providers` is the query
    /// those surfaces must use instead.
    #[test]
    fn a_provider_exclusion_is_invisible_to_a_bare_provider_prefix_filter() {
        let store = Store::open_in_memory().unwrap();
        store.exclude_provider("claude-cli", "auth failed").unwrap();

        let by_prefix: Vec<_> = store
            .current_benched_report()
            .unwrap()
            .into_iter()
            .filter(|(model, _, _)| model.starts_with("claude-cli"))
            .collect();
        assert!(
            by_prefix.is_empty(),
            "the trap itself: a `claude-cli%` filter finds nothing"
        );

        let excluded = store.current_excluded_providers().unwrap();
        assert_eq!(excluded.len(), 1);
        assert_eq!(
            excluded[0].0, "claude-cli",
            "reported without its key prefix"
        );
        assert!(excluded[0].2.contains("auth failed"));
        assert!(
            excluded[0].1 - now() <= AUTH_EXCLUSION_SECS,
            "and with an expiry the user can be told"
        );
    }

    #[test]
    fn an_auth_exclusion_is_far_shorter_than_a_capability_exclusion() {
        let store = Store::open_in_memory().unwrap();
        store.exclude_provider("claude-cli", "auth failed").unwrap();
        store.exclude_model("x::y", "no tool calling").unwrap();
        let report = store.current_benched_report().unwrap();
        let until = |needle: &str| {
            report
                .iter()
                .find(|(m, _, _)| m.contains(needle))
                .map(|(_, until, _)| *until)
                .unwrap()
        };
        assert!(until("claude-cli") - now() <= AUTH_EXCLUSION_SECS);
        assert!(until("x::y") - now() > AUTH_EXCLUSION_SECS * 4);
    }

    #[test]
    fn corroboration_requires_a_prior_failure_older_than_the_burst_window() {
        let store = Store::open_in_memory().unwrap();
        store
            .exclude_model_auth("claude-cli::opus", "auth failed")
            .unwrap();
        // Same instant: a burst of concurrent turns, not a repeat.
        assert!(!store
            .provider_auth_failed_before("claude-cli", now() - 120)
            .unwrap());
        // Aged past the window: a genuine repeat.
        let (model, until, reason) = store
            .current_benched_report()
            .unwrap()
            .into_iter()
            .find(|(m, _, _)| m == "claude-cli::opus")
            .unwrap();
        store
            .bench_model_at(&model, until, &reason, now() - 600)
            .unwrap();
        assert!(store
            .provider_auth_failed_before("claude-cli", now() - 120)
            .unwrap());
        // A different provider's failure is not corroboration.
        assert!(!store
            .provider_auth_failed_before("codex-cli", now() - 120)
            .unwrap());
    }

    #[test]
    fn a_rate_limit_bench_is_never_mistaken_for_auth_corroboration() {
        let store = Store::open_in_memory().unwrap();
        store
            .bench_model_at("claude-cli::opus", now() + 600, "rate-limited", now() - 600)
            .unwrap();
        assert!(!store
            .provider_auth_failed_before("claude-cli", now() - 120)
            .unwrap());
    }

    #[test]
    fn a_successful_turn_retires_the_model_bench_and_its_provider_exclusion() {
        let store = Store::open_in_memory().unwrap();
        store.exclude_provider("claude-cli", "auth failed").unwrap();
        store
            .exclude_model_auth("claude-cli::opus", "auth failed")
            .unwrap();
        store
            .clear_health_after_success("claude-cli::opus")
            .unwrap();
        assert!(store.current_benched_report().unwrap().is_empty());
        assert!(store.current_excluded_providers().unwrap().is_empty());
    }

    /// A row written under the superseded 24 h provider-wide rule must not outlive the release that
    /// fixed the rule; a capability exclusion or a rate-limit bench must survive untouched.
    #[test]
    fn opening_a_store_retires_auth_rows_written_under_the_old_rule() {
        let store = Store::open_in_memory().unwrap();
        let day = 24 * 60 * 60;
        store
            .bench_model_at(
                "__forge_provider__::claude-cli",
                now() + day,
                "excluded: provider auth failed: auth failed",
                now(),
            )
            .unwrap();
        store
            .bench_model_at(
                "openrouter::some/model",
                now() + day,
                "excluded: unsupported (no tool calling / unaffordable)",
                now(),
            )
            .unwrap();
        store
            .bench_model_at("groq::x", now() + 600, "rate-limited", now())
            .unwrap();

        retire_superseded_auth_exclusions(&store.lock().unwrap()).unwrap();

        let remaining: Vec<_> = store
            .current_benched_report()
            .unwrap()
            .into_iter()
            .map(|(m, _, _)| m)
            .collect();
        assert_eq!(remaining.len(), 2, "got {remaining:?}");
        assert!(!remaining.iter().any(|m| m.contains("claude-cli")));
        assert!(remaining.iter().any(|m| m == "openrouter::some/model"));
        assert!(remaining.iter().any(|m| m == "groq::x"));
    }

    /// A CURRENT-rule auth exclusion is not swept away by the same pass — otherwise every store
    /// open would silently unbench a provider that just failed.
    #[test]
    fn retiring_old_rows_leaves_a_current_rule_auth_exclusion_in_place() {
        let store = Store::open_in_memory().unwrap();
        store.exclude_provider("claude-cli", "auth failed").unwrap();
        retire_superseded_auth_exclusions(&store.lock().unwrap()).unwrap();
        assert_eq!(store.current_excluded_providers().unwrap().len(), 1);
    }
}
