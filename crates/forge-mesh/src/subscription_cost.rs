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

/// Largest per-model weekly quota on the OpenCode Go plan, in dollars — the bucket whose models
/// drain the shared pool slowest per dollar spent.
pub const OPENCODE_GO_LARGEST_WEEKLY_QUOTA: f64 = 30.0;

/// Per-model weekly quota on OpenCode Go, keyed by the bare model id as the catalog names it.
///
/// The Go dashboard (read 2026-09-02) shows that the weekly pool percentage is the SUM of
/// per-model percentages and that each model has its OWN weekly dollar quota: $7.50, $15 or $30.
/// A dollar spent on a $7.50-quota model therefore consumes four times as much of the weekly pool
/// as a dollar on a $30-quota model, which token price alone cannot see. The usage endpoint
/// (`GET /zen/go/v1/usage`, probed 2026-09-02 with `?breakdown=true`, `?detail=models`,
/// `/usage/models`, `/usage/breakdown`, `/quota`, `/limits`) returns only the three window
/// percentages, so this bundled table is the only source. A model absent here is left neutral
/// (multiplier 1.0) rather than guessed at.
const OPENCODE_GO_WEEKLY_QUOTAS: &[(&str, f64)] = &[
    ("grok-4.6", 7.5),
    ("kimi-k3", 7.5),
    ("gpt-5.6-luna", 7.5),
    ("qwen3.8-max", 7.5),
    ("grok-4.5", 7.5),
    ("deepseek-v4-pro", 7.5),
    ("mimo-v2.5-pro", 7.5),
    ("glm-5.3", 7.5),
    ("deepseek-v4-flash-vision-exp", 7.5),
    ("qwen3.7-max", 15.0),
    ("glm-5.3-flash", 15.0),
    ("deepseek-v4-flash", 15.0),
    ("qwen3.8-flash", 15.0),
    ("hy4-preview", 15.0),
    // The dashboard row is truncated after "MiniMax..."; M3 and M2.5 are listed in the $30 row,
    // so M2.7 is the remaining MiniMax model.
    ("minimax-m2.7", 15.0),
    ("muse-spark-1.2-contributor", 30.0),
    ("qwen3.5-plus", 30.0),
    ("qwen3.6-plus", 30.0),
    ("qwen3.7-plus", 30.0),
    ("minimax-m3", 30.0),
    ("minimax-m2.5", 30.0),
    ("kimi-k2.7-code", 30.0),
    ("kimi-k2.6", 30.0),
    ("glm-5.2", 30.0),
    ("glm-5.1", 30.0),
    ("glm-5", 30.0),
    ("mimo-v2.5", 30.0),
    ("longcat-2.0", 30.0),
    ("hy3", 30.0),
];

/// The weekly dollar quota of an OpenCode Go model, from the bundled dashboard table.
pub fn opencode_go_weekly_quota(id: &str) -> Option<f64> {
    if provider_of(id) != "opencode_go" {
        return None;
    }
    let bare = crate::capability::bare_model(id);
    OPENCODE_GO_WEEKLY_QUOTAS
        .iter()
        .find(|(model, _)| *model == bare)
        .map(|(_, quota)| *quota)
}

/// How many times faster than a largest-quota model a dollar on `id` drains the Go weekly pool:
/// `largest quota / this model's quota`. `None` for non-Go ids and for models the table does
/// not know, which the caller must treat as neutral.
pub fn opencode_go_quota_multiplier(id: &str) -> Option<f64> {
    opencode_go_weekly_quota(id).map(|quota| OPENCODE_GO_LARGEST_WEEKLY_QUOTA / quota)
}

/// The inspector's rendering of the multiplier, e.g. `quota $7.50/wk → x4.0`.
pub fn opencode_go_quota_note(id: &str) -> Option<String> {
    let quota = opencode_go_weekly_quota(id)?;
    Some(format!(
        "quota ${quota:.2}/wk → x{:.1}",
        OPENCODE_GO_LARGEST_WEEKLY_QUOTA / quota
    ))
}

/// Derive relative plan-burn weights from tracked prices for subscription providers whose model
/// families have no curated weight. The cheapest priced sibling is 1.0; providers with fewer than
/// two priced models are left neutral.
///
/// OpenCode Go's dollar-denominated allowance is shared by every model while their token prices
/// vary widely (<https://opencode.ai/docs/go/#usage-limits>, read 2026-09-01). Reference list price
/// is therefore the deterministic spend-rate proxy, without charging subscription turns to the
/// user's metered budget or hardcoding a model preference list. On Go the price is further
/// multiplied by [`opencode_go_quota_multiplier`]: the pool is drained per model quota, not per
/// dollar, so a $7.50-quota model at the same price weighs 4x a $30-quota one.
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
            let cost = cost * opencode_go_quota_multiplier(model).unwrap_or(1.0);
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

/// How much of a subscription's pool ONE request is worth, as an ordinal class. Percentages
/// alone cannot say this: OpenCode Go's whole 5-hour pool is $12 (≈110 Kimi K3 requests), while
/// a Max-20x Claude plan's is hundreds of times larger, yet both read "25% used" the same way.
/// The class is the only honest capacity signal Forge has for percentage-only plans, so it is
/// deliberately coarse and its sources are named here rather than dressed up as numbers:
/// - `opencode_go`: documented $12 / 5h, $30 / week, $60 / month
///   (<https://opencode.ai/docs/go/#usage-limits>, read 2026-09-02) → `Tiny`.
/// - CLI bridges by the plan slug `forge init` captured: `*20x*` → `Large`; `max` / `pro` →
///   `Medium`; `plus` / `team` → `Small`. Unset → `Unknown` (no penalty; never guess a pool).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolCapacity {
    Tiny,
    Small,
    Medium,
    Large,
    Unknown,
}

pub(crate) fn pool_capacity(provider: &str, plan: &str) -> PoolCapacity {
    if provider == "opencode_go" {
        return PoolCapacity::Tiny;
    }
    let plan = plan.to_ascii_lowercase();
    if plan.is_empty() {
        PoolCapacity::Unknown
    } else if plan.contains("20x") {
        PoolCapacity::Large
    } else if plan.contains("max") || plan.contains("pro") {
        PoolCapacity::Medium
    } else if plan.contains("plus") || plan.contains("team") {
        PoolCapacity::Small
    } else {
        PoolCapacity::Unknown
    }
}

impl PoolCapacity {
    /// Relative share of the pool one request consumes; `Tiny` = 1.0 by definition.
    fn request_share(self) -> f64 {
        match self {
            PoolCapacity::Tiny => 1.0,
            PoolCapacity::Small => 0.5,
            PoolCapacity::Medium => 0.25,
            PoolCapacity::Large => 0.1,
            PoolCapacity::Unknown => 0.0,
        }
    }
}

/// Penalty for spending a request out of a small or nearly-spent pool. `request_share` says how
/// big one request is relative to the pool; the scarcity factor (1 / remaining, capped at 3x)
/// says how much of that pool is left, so the same model on a fuller, larger pool always ranks
/// above itself on an emptier, smaller one at equal score. Tier-scaled like the burn penalty:
/// a complex task may still justify the spend, a trivial one never does.
///
/// Live case this exists for (2026-09-02): at opencode_go 28% / codex-cli 25% every subscription
/// model carried the same flat conservation penalty, so `opencode_go::kimi-k3` (3.27) outranked
/// `codex-oauth::gpt-5.6-sol` (2.96) although one Kimi request took ~1% of a $12 pool and one
/// Sol request a fraction of that from a far larger plan.
pub(crate) fn pool_capacity_penalty(
    id: &str,
    tier: TaskTier,
    quota: &forge_types::SubscriptionQuota,
) -> f64 {
    let provider = provider_of(id);
    let share = pool_capacity(provider, quota.plan_for(provider)).request_share();
    if share <= 0.0 {
        return 0.0;
    }
    let remaining = (1.0 - quota.effective_fraction_for(provider)).max(0.0);
    let scarcity = (1.0 / remaining.max(1.0 / 3.0)).clamp(1.0, 3.0);
    burn_k(tier) * share * scarcity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_weekly_quotas_follow_the_dashboard_buckets() {
        assert_eq!(opencode_go_weekly_quota("opencode_go::grok-4.6"), Some(7.5));
        assert_eq!(
            opencode_go_weekly_quota("opencode_go::glm-5.3-flash"),
            Some(15.0)
        );
        assert_eq!(
            opencode_go_weekly_quota("opencode_go::muse-spark-1.2-contributor"),
            Some(30.0)
        );
        assert_eq!(
            opencode_go_quota_multiplier("opencode_go::kimi-k3"),
            Some(4.0)
        );
        assert_eq!(
            opencode_go_quota_multiplier("opencode_go::minimax-m2.7"),
            Some(2.0)
        );
        assert_eq!(
            opencode_go_quota_multiplier("opencode_go::glm-5"),
            Some(1.0)
        );
        assert_eq!(
            opencode_go_quota_note("opencode_go::gpt-5.6-luna").as_deref(),
            Some("quota $7.50/wk → x4.0")
        );
    }

    #[test]
    fn unknown_models_and_other_providers_stay_neutral() {
        // The usage endpoint carries no per-model data, so the bundled table is the only source;
        // anything it does not list must not be guessed at.
        assert_eq!(opencode_go_quota_multiplier("opencode_go::kimi-k2.5"), None);
        assert_eq!(
            opencode_go_quota_multiplier("codex-oauth::gpt-5.6-luna"),
            None
        );
        assert_eq!(opencode_go_quota_note("claude-cli::opus"), None);
        let pricing = Pricing::from_rates(HashMap::from([
            (
                "openrouter::x/known".to_string(),
                crate::pricing::ModelRate {
                    input_per_1k: 0.001,
                    output_per_1k: 0.002,
                    cache_read_per_1k: None,
                },
            ),
            (
                "openrouter::x/kimi-k2.5".to_string(),
                crate::pricing::ModelRate {
                    input_per_1k: 0.001,
                    output_per_1k: 0.002,
                    cache_read_per_1k: None,
                },
            ),
        ]));
        let weights = price_derived_burn_weights(
            &[
                "opencode_go::known".to_string(),
                "opencode_go::kimi-k2.5".to_string(),
            ],
            &pricing,
        );
        assert!((weights["opencode_go::kimi-k2.5"] - 1.0).abs() < 1e-9);
    }
}
