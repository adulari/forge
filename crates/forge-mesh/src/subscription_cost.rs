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
        if crate::capability::known_burn_weight(model).is_some() {
            continue;
        }
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

/// Penalize subscription burn while preserving capability-first behavior on a fresh window.
/// Curated family weights retain their standing 0.5 multiplier; price-derived ratios start at
/// exactly zero and rise with observed pressure because their much larger spread would otherwise
/// overturn genuine capability differences before conservation is needed.
pub(crate) fn subscription_burn_penalty(
    id: &str,
    tier: TaskTier,
    quota: &forge_types::SubscriptionQuota,
    overrides: &HashMap<String, f64>,
    price_weights: &HashMap<String, f64>,
) -> f64 {
    let (weight, derived) = match crate::capability::configured_burn_weight(id, overrides) {
        Some(weight) => (weight, false),
        None => (price_weights.get(id).copied().unwrap_or(1.0), true),
    };
    if weight <= 1.0 {
        return 0.0;
    }
    let fraction = quota.effective_fraction_for(provider_of(id));
    let scale = if derived {
        pressure_multiplier(fraction) - PRESSURE_MULTIPLIER_FLOOR
    } else {
        pressure_multiplier(fraction)
    };
    burn_k(tier) * weight.ln() * scale
}
