//! Persisting a provider failure into the shared model-health store.

use super::compaction_policy::{model_health_reason, record_auth_failure_in};

/// Store-level body of [`Session::record_model_failure`], free so a detached side-call task that
/// owns only an `Arc<Store>` records failures through the SAME classifier as the main path.
pub(crate) fn record_model_failure_in(
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
    if err.is_context_overflow() {
        return;
    }
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
