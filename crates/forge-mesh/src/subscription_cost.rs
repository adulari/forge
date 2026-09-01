use std::collections::HashMap;

use forge_types::TaskTier;

use crate::catalog::{is_subscription, provider_of};
use crate::pricing::Pricing;

/// Per-tier scaling for the subscription burn-weight penalty.
pub(crate) const BURN_K_TRIVIAL: f64 = 1.0;
pub(crate) const BURN_K_STANDARD: f64 = 0.7;
pub(crate) const BURN_K_COMPLEX: f64 = 0.30;

fn burn_k(tier: TaskTier) -> f64 {
    match tier {
        TaskTier::Trivial => BURN_K_TRIVIAL,
        TaskTier::Standard => BURN_K_STANDARD,
        TaskTier::Complex => BURN_K_COMPLEX,
    }
}

fn pressure_multiplier(fraction: f64) -> f64 {
    (PRESSURE_MULTIPLIER_FLOOR + 1.5 * fraction).clamp(PRESSURE_MULTIPLIER_FLOOR, 2.0)
}

const PRESSURE_MULTIPLIER_FLOOR: f64 = 0.5;

/// Derive relative plan-burn weights from tracked prices for subscription providers whose model
/// families have no curated weight. The cheapest priced sibling is 1.0; providers with fewer than
/// two priced models are left neutral.
///
/// OpenCode Go's dollar-denominated allowance is shared by every model while their token prices
/// vary widely (<https://opencode.ai/docs/go/#usage-limits>, read 2026-09-01). Reference list price
/// is therefore the deterministic spend-rate proxy, without charging subscription turns to the
/// user's metered budget or hardcoding a model preference list.
pub(crate) fn price_derived_burn_weights(
    models: &[String],
    pricing: &Pricing,
) -> HashMap<String, f64> {
    let mut by_provider: HashMap<&str, Vec<(&String, f64)>> = HashMap::new();
    for model in models.iter().filter(|model| is_subscription(model)) {
        if let Some(cost) = pricing
            .reference_estimated_cost(model)
            .filter(|cost| *cost > 0.0)
        {
            by_provider
                .entry(provider_of(model))
                .or_default()
                .push((model, cost));
        }
    }
    by_provider
        .into_values()
        .filter(|priced| priced.len() > 1)
        .flat_map(|priced| {
            let cheapest = priced
                .iter()
                .map(|(_, cost)| *cost)
                .fold(f64::INFINITY, f64::min);
            priced
                .into_iter()
                .map(move |(model, cost)| (model.clone(), cost / cheapest))
        })
        .collect()
}

/// The relative burn weight routing uses for `id`, and where it came from. Precedence:
/// an explicit config override → the weight derived from tracked (fetched or bundled) prices →
/// the hardcoded family ladder in `capability::known_burn_weight`, which exists only for models
/// no price source covers. Prices go stale in a table but not in a fetch, so the fetched number
/// must win; the previous order (table first) kept GPT-5.6's pre-discount ladder in force for
/// months after Luna's price dropped ~4x.
pub(crate) fn burn_weight_for(
    id: &str,
    overrides: &HashMap<String, f64>,
    price_weights: &HashMap<String, f64>,
) -> (f64, BurnWeightSource) {
    if let Some(weight) = overrides.get(crate::capability::bare_model(id)).copied() {
        return (weight, BurnWeightSource::Override);
    }
    if let Some(weight) = price_weights.get(id).copied() {
        return (weight, BurnWeightSource::Price);
    }
    match crate::capability::known_burn_weight(id) {
        Some(weight) => (weight, BurnWeightSource::Table),
        None => (1.0, BurnWeightSource::Unknown),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BurnWeightSource {
    Override,
    Price,
    Table,
    Unknown,
}

/// Penalize subscription burn. A subscription pool is shared by every model on it and drained at
/// each model's own price, so a heavier sibling must be MEANINGFULLY better to be worth picking
/// even on a fresh window: the penalty carries the standing 0.5 floor at zero pressure and rises
/// to 2x as the window fills. Price-derived weights used to start at exactly zero and only rise
/// with pressure; on OpenCode Go's $12/5h pool that let Kimi K3 (~50x Muse Spark's price) win a
/// 0.14-point score gap on every turn until 64% of the pool was gone in two hours (2026-09-02).
pub(crate) fn subscription_burn_penalty(
    id: &str,
    tier: TaskTier,
    quota: &forge_types::SubscriptionQuota,
    overrides: &HashMap<String, f64>,
    price_weights: &HashMap<String, f64>,
) -> f64 {
    let (weight, _source) = burn_weight_for(id, overrides, price_weights);
    if weight <= 1.0 {
        return 0.0;
    }
    let fraction = quota.effective_fraction_for(provider_of(id));
    burn_k(tier) * weight.ln() * pressure_multiplier(fraction)
}
