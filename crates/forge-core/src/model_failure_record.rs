//! Persisting a provider failure into the shared model-health store.

use super::compaction_policy::{model_health_reason, record_auth_failure_in};

/// The class recorded with a failed attempt in the mesh outcome ledger.
pub(crate) fn error_kind(error: &forge_provider::ProviderError) -> &'static str {
    if error.is_auth() {
        "auth"
    } else if error.is_rate_limited() {
        "rate_limited"
    } else if error.is_context_overflow() {
        "context_overflow"
    } else if error.is_permanent() {
        "permanent"
    } else {
        "transient"
    }
}

/// Store-level body of [`Session::record_model_failure`], free so a detached side-call task that
/// owns only an `Arc<Store>` records failures through the SAME classifier as the main path.
pub fn record_model_failure_in(
    store: &forge_store::Store,
    model: &str,
    err: &forge_provider::ProviderError,
    default_cooldown: std::time::Duration,
) {
    // An over-window request is a statement about the payload, not about the model: the same
    // model answers fine the moment we send less. Benching for it sidelines a healthy model for
    // a full cooldown and, because the auxiliary chains are trivial-tier, walks the identical
    // oversized payload into the next cheap model and benches that one too — so the damage
    // outlives the request that caused it and degrades routing for ordinary turns afterwards.
    // A token throttle is the same kind of statement about the payload: a request that exceeds the
    // account's tokens-per-minute quota is throttled whatever model it goes to, while a short
    // prompt on the same model is fine.
    if err.is_context_overflow()
        || err.is_token_throttle()
        || crate::zero_retention::is_refusal(err)
    {
        return;
    }
    record_key_verdict(store, model, err);
    let detail = err.to_string();
    let reason = model_health_reason(err, &detail);
    if err.is_auth() {
        // The store prefixes the class itself ("excluded: auth failed: …"), so what goes in
        // here must be the EVIDENCE — the provider's / CLI's own text — not the class label
        // again. A row reading "auth failed: auth failed" told the mobile app that Opus was
        // benched for auth while `claude --model opus -p` answered fine on the same login,
        // and left nothing to diagnose it with (2026-09-02).
        let detail = detail.trim();
        let detail: String = detail.chars().take(240).collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        record_auth_failure_in(
            store,
            model,
            if detail.is_empty() {
                err.reason()
            } else {
                &detail
            },
            now,
        );
    } else if err.is_permanent() {
        let _ = store.exclude_model(model, reason);
    } else {
        let _ = store.bench_for(model, err.cooldown(default_cooldown), reason);
    }
}

/// A key the provider itself refused gets a persisted verdict for the providers screens. Only
/// keyed API providers: a logged-out CLI bridge is also `Auth`, but there is no key to renew.
fn record_key_verdict(
    store: &forge_store::Store,
    model: &str,
    err: &forge_provider::ProviderError,
) {
    let Some((provider, _)) = model.split_once("::") else {
        return;
    };
    if !forge_config::known_key_providers().any(|p| p == provider) {
        return;
    }
    let Some((kind, reason)) = err.key_failure() else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    if let Ok(forge_store::VerdictChange::New | forge_store::VerdictChange::Changed) =
        store.set_provider_verdict(provider, kind.as_str(), &reason, now)
    {
        tracing::warn!("{provider}: {} ({reason})", kind.headline());
    }
}

#[cfg(test)]
mod key_verdict_tests {
    use super::*;

    #[test]
    fn a_refused_key_sets_a_verdict_and_a_completed_call_clears_it() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let err = forge_provider::ProviderError::Auth("401 This API key has expired".into());
        record_model_failure_in(
            &store,
            "groq::llama",
            &err,
            std::time::Duration::from_secs(60),
        );
        let verdicts = store.provider_verdicts().unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].status, "expired");

        store.clear_health_after_success("groq::llama").unwrap();
        assert!(store.provider_verdicts().unwrap().is_empty());
    }

    #[test]
    fn a_logged_out_bridge_is_not_a_key_verdict() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let err = forge_provider::ProviderError::Auth("Not logged in".into());
        record_model_failure_in(
            &store,
            "claude-cli::opus",
            &err,
            std::time::Duration::from_secs(60),
        );
        assert!(store.provider_verdicts().unwrap().is_empty());
    }
}
