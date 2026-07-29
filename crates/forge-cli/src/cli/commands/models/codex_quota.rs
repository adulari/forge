//! Where the Codex subscription quota reading comes from.
//!
//! Codex quota moves whenever the shared ChatGPT account is used outside Forge, and there are two
//! places to learn about it: a direct OAuth header observation, which describes the account Forge
//! is about to use, and the local CLI's rollout log, which can be fresh yet belong to a different
//! account or carry an obsolete plan. This module owns that preference order and the staleness
//! rules around it — the CLI is a no-cost fallback, never a reason to skip a due OAuth probe.

use forge_store::Store;

use super::CODEX_QUOTA_MAX_AGE_SECS;

/// Choose the source before reading a CLI rollout. A direct OAuth-header observation represents
/// the account Forge is about to use; a local CLI session can be fresh yet belong to a different
/// account or retain an obsolete plan. The CLI is therefore a no-cost fallback, never a reason to
/// skip a due lightweight OAuth probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexQuotaSource {
    FreshOAuth,
    ProbeOAuth,
    CliFallback,
}

fn codex_quota_source(has_oauth: bool, oauth_age_secs: Option<i64>) -> CodexQuotaSource {
    if has_oauth {
        if oauth_age_secs.is_some_and(|age| age <= CODEX_QUOTA_MAX_AGE_SECS) {
            CodexQuotaSource::FreshOAuth
        } else {
            CodexQuotaSource::ProbeOAuth
        }
    } else {
        CodexQuotaSource::CliFallback
    }
}

fn codex_quota_is_stale(store: &Store) -> bool {
    !["codex-oauth", "codex-cli"]
        .into_iter()
        .filter_map(|provider| store.subscription_age_secs(provider))
        .any(|age| age <= CODEX_QUOTA_MAX_AGE_SECS)
}

fn seed_codex_rollout_quota(store: &Store, stats: &crate::bridge_stats::BridgeStats) -> bool {
    let mut observed = false;
    for (window, percent, observed_at) in [
        ("five_hour", stats.codex_5h_pct, stats.codex_5h_observed_at),
        (
            "weekly",
            stats.codex_weekly_pct,
            stats.codex_weekly_observed_at,
        ),
    ] {
        let (Some(percent), Some(observed_at)) = (percent, observed_at) else {
            continue;
        };
        if chrono::Utc::now().timestamp().saturating_sub(observed_at) > CODEX_QUOTA_MAX_AGE_SECS {
            continue;
        }
        let fraction = (percent / 100.0).clamp(0.0, 1.0);
        let status = if fraction >= 0.98 {
            forge_types::QuotaStatus::Exhausted
        } else if fraction >= 0.80 {
            forge_types::QuotaStatus::Warning
        } else {
            forge_types::QuotaStatus::Ok
        };
        let _ = store.record_codex_quota_at(
            &forge_types::QuotaHint {
                provider: "codex-cli".to_string(),
                window: window.to_string(),
                status,
                resets_at: None,
                fraction_used: Some(fraction),
            },
            observed_at,
        );
        observed = true;
    }
    if let Some(plan) = stats.codex_plan.as_deref() {
        let observed_at = stats
            .codex_plan_observed_at
            .unwrap_or_else(|| chrono::Utc::now().timestamp());
        if chrono::Utc::now().timestamp().saturating_sub(observed_at) <= CODEX_QUOTA_MAX_AGE_SECS {
            let _ = store.record_subscription_plan_at("codex-cli", plan, observed_at);
            let _ = store.record_subscription_plan_at("codex-oauth", plan, observed_at);
            observed = true;
        }
    }
    observed
}

/// Refresh the shared Codex quota before a routing decision. OAuth is preferred whenever its own
/// direct observation is stale and uses one tiny shared-meter `gpt-5.6-luna` request. The local
/// CLI rollout is a no-cost fallback when that probe is unavailable. Failure is deliberately
/// non-fatal: the store filters expired data, letting normal provider failover handle an
/// unavailable account rather than routing on a known-stale pressure reading.
pub(crate) async fn refresh_codex_quota(store: &Store) {
    // OAuth header data is the canonical account observation: it comes directly from the
    // ChatGPT Codex backend, includes the provider's exact plan spelling, and naturally omits a
    // disabled window (for example an account with no five-hour allowance).  Prefer it over the
    // bridge rollout file, which is only a best-effort fallback and can be stale while a user is
    // actively consuming Codex outside Forge.
    match codex_quota_source(
        forge_provider::has_codex_oauth_session(),
        store.codex_oauth_quota_age_secs().ok().flatten(),
    ) {
        CodexQuotaSource::FreshOAuth => return,
        CodexQuotaSource::ProbeOAuth => match forge_provider::probe_codex_quota().await {
            Ok(hints) if !hints.is_empty() => {
                for hint in &hints {
                    let _ = store.record_live_codex_oauth_quota(hint);
                }
                if let Some(plan) = forge_provider::fresh_live_codex_plan() {
                    // The plan is account-wide just like the windows.  Keep both aliases in sync
                    // so `forge mesh`, `/mesh`, `/usage`, API and bridge selection agree.
                    let _ = store.record_subscription_plan("codex-oauth", &plan);
                    let _ = store.record_subscription_plan("codex-cli", &plan);
                }
                return;
            }
            // A successful response without usable quota headers is not a zero-percent reading.
            // Fall through to the CLI source instead of fabricating an allowance or freshness.
            Ok(_) | Err(_) => {}
        },
        CodexQuotaSource::CliFallback => {}
    }

    if !codex_quota_is_stale(store) {
        return;
    }

    // No OAuth session or an unavailable OAuth probe: retain the CLI bridge as a no-cost fallback.
    // Its values receive their original observation timestamps, so an old rollout cannot overwrite
    // a later direct-OAuth observation.
    let stats = tokio::task::spawn_blocking(crate::bridge_stats::fetch)
        .await
        .unwrap_or_default();
    let _ = seed_codex_rollout_quota(store, &stats);
    if !codex_quota_is_stale(store) {
        return;
    }
    for hint in crate::bridge_stats::probe_codex_limits().await {
        let _ = store.record_codex_quota(&hint);
    }
    if let Some(plan) = forge_provider::fresh_live_codex_plan() {
        let _ = store.record_subscription_plan("codex-oauth", &plan);
        let _ = store.record_subscription_plan("codex-cli", &plan);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollout_seed_preserves_source_timestamps_for_quota_and_plan() {
        let store = Store::open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let stats = crate::bridge_stats::BridgeStats {
            codex_5h_pct: Some(35.0),
            codex_5h_observed_at: Some(now - 30),
            codex_plan: Some("pro".to_string()),
            codex_plan_observed_at: Some(now - 20),
            ..Default::default()
        };

        assert!(seed_codex_rollout_quota(&store, &stats));
        assert_eq!(store.subscription_age_secs("codex-cli"), Some(30));
        assert_eq!(
            store
                .fresh_subscription_plan_at("codex-oauth", now)
                .as_deref(),
            Some("pro")
        );
        assert_eq!(
            store.fresh_subscription_plan_at("codex-oauth", now + CODEX_QUOTA_MAX_AGE_SECS + 1),
            None
        );
    }

    #[test]
    fn oauth_quota_probe_is_preferred_over_a_fresh_cli_rollout() {
        // The CLI rollout may be fresh but represent a different/stale local Codex session. When
        // direct OAuth has no fresh observation, its tiny authoritative probe must win; CLI is
        // only the fallback if the OAuth probe cannot produce quota headers.
        assert_eq!(codex_quota_source(true, None), CodexQuotaSource::ProbeOAuth);
        assert_eq!(
            codex_quota_source(true, Some(CODEX_QUOTA_MAX_AGE_SECS)),
            CodexQuotaSource::FreshOAuth
        );
        assert_eq!(
            codex_quota_source(false, None),
            CodexQuotaSource::CliFallback
        );
    }
}
