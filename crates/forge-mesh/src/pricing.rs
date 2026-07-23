//! Per-model pricing and cost computation (FR-5, A-7). Rates are bundled defaults and
//! user-overridable via config, so a provider price change needs no release.

use std::collections::HashMap;

/// USD price per 1,000 tokens for a model's input and output. `cache_read_per_1k` is the discounted
/// rate for prompt tokens served from the provider's cache; `None` means we have no cache rate, so
/// cached tokens fall back to the full input rate (no discount assumed).
#[derive(Debug, Clone, Copy)]
pub struct ModelRate {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    pub cache_read_per_1k: Option<f64>,
}

impl From<forge_config::PriceOverride> for ModelRate {
    fn from(o: forge_config::PriceOverride) -> Self {
        ModelRate {
            input_per_1k: o.input_per_1k,
            output_per_1k: o.output_per_1k,
            cache_read_per_1k: None,
        }
    }
}

/// A table of model id -> rate. Unknown models cost nothing (e.g. local Ollama).
#[derive(Debug, Clone)]
pub struct Pricing {
    rates: HashMap<String, ModelRate>,
}

/// Bundled default rates (USD per 1k tokens) for the models Forge ships in its defaults,
/// approximating mid-2026 list prices. Overridable via config (A-7).
/// Documented in docs/features/mesh-routing.md. INVARIANT: any METERED model added to the
/// catalog defaults MUST get an entry here (or a fetched/config price) — an absent entry prices
/// as $0.0 and cost-tiered routing then treats the most expensive model as the cheapest.
const DEFAULT_RATES: &[(&str, f64, f64)] = &[
    ("openai::gpt-4o-mini", 0.00015, 0.0006),
    // Opus 4.8's actual list price is $5/$25 per 1M tokens (0.005/0.025 per 1k). The prior entry
    // here (0.015/0.075, i.e. $15/$75) was a copied Opus 4.1 rate (4.1 genuinely is $15/$75) — ~3x
    // too high for 4.8 — and inflated its estimated_cost enough to distort cost-tiered routing
    // comparisons against other frontier models.
    ("anthropic::claude-opus-4-8", 0.005, 0.025),
    // Claude Fable 5 / Mythos 5 are $10/$50 per 1M (0.010/0.050 per 1k) — the fleet's priciest
    // models. Without an entry they price as $0 (unknown = free), so cost-tiered routing would
    // treat the most expensive metered models as the cheapest. Verified against Anthropic's
    // pricing page (platform.claude.com/docs/en/about-claude/pricing), 2026-07-10.
    ("anthropic::claude-fable-5", 0.010, 0.050),
    ("anthropic::claude-mythos-5", 0.010, 0.050),
    // Additional BYOK providers (approx mid-2026 list prices, USD per 1k tokens).
    // Override via config [mesh.pricing] if a price changes (A-7).
    ("gemini::gemini-2.5-flash", 0.0003, 0.0025),
    ("gemini::gemini-2.5-pro", 0.00125, 0.01),
    ("deepseek::deepseek-chat", 0.00027, 0.0011),
    ("xai::grok-4", 0.003, 0.015),
    // Local models (ollama::*) and gateway/per-model providers (open_router::*, where the
    // effective price depends on the routed model) are intentionally absent -> free unless
    // priced via config. cost_for() returns 0.0 for any unlisted model (never panics).
];

impl Default for Pricing {
    fn default() -> Self {
        let rates = DEFAULT_RATES
            .iter()
            .map(|&(id, input_per_1k, output_per_1k)| {
                (
                    id.to_string(),
                    ModelRate {
                        input_per_1k,
                        output_per_1k,
                        cache_read_per_1k: None,
                    },
                )
            })
            .collect();
        Self { rates }
    }
}

impl Pricing {
    /// Build from explicit rates (used by config overrides and tests).
    pub fn from_rates(rates: HashMap<String, ModelRate>) -> Self {
        Self { rates }
    }

    /// Apply user overrides on top of the defaults (overrides win per model id).
    pub fn with_overrides(mut self, overrides: HashMap<String, ModelRate>) -> Self {
        self.rates.extend(overrides);
        self
    }

    /// Bundled defaults with the config's per-model overrides applied (A-7).
    pub fn from_config(config: &forge_config::Config) -> Self {
        let overrides = config
            .mesh
            .pricing
            .iter()
            .map(|(id, &o)| (id.clone(), o.into()))
            .collect();
        Pricing::default().with_overrides(overrides)
    }

    /// Bundled defaults, then prices fetched from a provider's model API (e.g. OpenRouter),
    /// then the config's explicit overrides — so precedence is defaults < fetched < user config.
    /// This is what lets gateway/credit spend be tracked: those models aren't in the bundled
    /// defaults, so without the fetched layer their cost is $0 and the budget cap can't see it.
    pub fn from_config_with_fetched(
        config: &forge_config::Config,
        fetched: impl IntoIterator<Item = (String, f64, f64, Option<f64>)>,
    ) -> Self {
        let fetched_rates = fetched
            .into_iter()
            .map(|(id, input_per_1k, output_per_1k, cache_read_per_1k)| {
                (
                    id,
                    ModelRate {
                        input_per_1k,
                        output_per_1k,
                        cache_read_per_1k,
                    },
                )
            })
            .collect();
        let config_overrides = config
            .mesh
            .pricing
            .iter()
            .map(|(id, &o)| (id.clone(), o.into()))
            .collect();
        Pricing::default()
            .with_overrides(fetched_rates)
            .with_overrides(config_overrides)
    }

    /// Compute the USD cost of a call given token counts. Unknown models cost nothing. Charges all
    /// input at the full rate — use [`cost_for_usage`](Self::cost_for_usage) when cache-read counts
    /// are known so cached tokens get their discounted rate.
    /// Documented in docs/features/mesh-routing.md.
    pub fn cost_for(&self, model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
        match self.rates.get(model) {
            Some(rate) => {
                (input_tokens as f64 / 1000.0) * rate.input_per_1k
                    + (output_tokens as f64 / 1000.0) * rate.output_per_1k
            }
            None => 0.0,
        }
    }

    /// Compute the USD cost of a call from its [`Usage`], pricing cache-read tokens at the model's
    /// discounted cache rate (the provider bills them well below the full input rate). Fresh input
    /// = `input_tokens - cached_input_tokens`. With no cache rate or no cached tokens this equals
    /// [`cost_for`](Self::cost_for). Unknown models cost nothing.
    /// Documented in docs/features/mesh-routing.md.
    pub fn cost_for_usage(&self, model: &str, usage: &forge_types::Usage) -> f64 {
        let Some(rate) = self.rates.get(model) else {
            return 0.0;
        };
        let cached = usage.cached_input_tokens.min(usage.input_tokens);
        let fresh = usage.input_tokens - cached;
        let cache_rate = rate.cache_read_per_1k.unwrap_or(rate.input_per_1k);
        (fresh as f64 / 1000.0) * rate.input_per_1k
            + (cached as f64 / 1000.0) * cache_rate
            + (usage.output_tokens as f64 / 1000.0) * rate.output_per_1k
    }

    /// A *relative* cost comparator for routing: the price of a nominal turn (1000 in / 500
    /// out). Not a forecast — only used to rank candidate models against each other. Unpriced
    /// models (local, gateways) compare as 0.0 (cheapest).
    /// Documented in docs/features/mesh-routing.md.
    pub fn estimated_cost(&self, model: &str) -> f64 {
        self.cost_for(model, NOMINAL_INPUT_TOKENS, NOMINAL_OUTPUT_TOKENS)
    }
}

/// Nominal token mix used only to rank candidate models by relative cost.
/// Documented in docs/features/mesh-routing.md; value asserted in sync by `doc_sync::mesh_routing_doc_matches_live_constants`.
pub(crate) const NOMINAL_INPUT_TOKENS: u64 = 1000;
pub(crate) const NOMINAL_OUTPUT_TOKENS: u64 = 500;

/// A conservative context window (tokens) assumed for a model we have NO better figure for —
/// neither a fetched window (provider API) nor a hardcoded bridge value in [`context_limit`]. 32k
/// is the common floor for modern chat models, so trimming a transcript to this rarely overflows an
/// unknown model while still letting a real turn through. Used by the core to bound what it sends.
pub const CONSERVATIVE_CONTEXT_WINDOW: u32 = 32_000;

/// A provider-independent context window that Forge knows authoritatively and should prefer over
/// missing or stale discovery metadata. Keep this deliberately narrow: most API models belong in
/// the fetched `model_context` cache, while entries here cover documented models whose compatible
/// `/models` endpoint omits the field (or a third-party catalog reports a provisional floor).
pub fn authoritative_context_limit(model: &str) -> Option<u32> {
    let model_id = model
        .split_once("::")
        .map_or(model, |(_, model_id)| model_id)
        .rsplit('/')
        .next()
        .unwrap_or(model);
    if model_id.eq_ignore_ascii_case("qwen3.7-max")
        || model_id.eq_ignore_ascii_case("qwen3.8-max-preview")
    {
        Some(1_000_000)
    } else {
        None
    }
}

/// The context-window size (in tokens) for a model id, or `None` when we don't have a
/// well-established figure. Only used as a last-resort fallback for subscription CLI and OAuth
/// bridges whose model lists can't be queried via an API; all other models should have their windows fetched from
/// the provider's model endpoint and persisted via `context_windows::fetch_and_persist`. Returns
/// `None` for non-bridge models so the statusline omits a fabricated denominator; the core falls
/// back to
/// [`CONSERVATIVE_CONTEXT_WINDOW`] only when it must actually bound a request.
/// Documented in docs/features/mesh-routing.md.
pub fn context_limit(model: &str) -> Option<u32> {
    // Subscription bridges carry no queryable API — their windows are hardcoded here.
    // Codex OAuth names the same Codex family as the CLI bridge, including the 5.6 aliases
    // and gpt-5.3-codex, so it uses the documented 272k family window as well.
    // All other providers must have their windows fetched from the provider's model endpoint
    // and persisted via `context_windows::fetch_and_persist`; `None` here tells the core to
    // use the DB-stored fetched value or fall back to `CONSERVATIVE_CONTEXT_WINDOW`.
    let (provider, bridge_model) = model.split_once("::").unwrap_or((model, ""));
    match provider {
        // Opus (4.6+) and Sonnet (4.5+) both GA'd to a 1M-token context window in March 2026 —
        // Haiku has not. A flat per-provider figure silently under-assumed the real window once
        // it grew past 200k.
        "claude-cli" if bridge_model.contains("haiku") => Some(200_000),
        "claude-cli" => Some(1_000_000),
        "codex-cli" | "codex-oauth" => Some(272_000),
        "agy-cli" => Some(1_000_000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_is_tokens_times_rate_per_1k() {
        let mut rates = HashMap::new();
        rates.insert(
            "openai::gpt-4o-mini".to_string(),
            ModelRate {
                input_per_1k: 0.00015,
                output_per_1k: 0.0006,
                cache_read_per_1k: None,
            },
        );
        let pricing = Pricing { rates };

        // 1000 input @ 0.00015 + 2000 output @ 0.0006 = 0.00015 + 0.0012 = 0.00135
        let cost = pricing.cost_for("openai::gpt-4o-mini", 1000, 2000);
        assert!((cost - 0.00135).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn unknown_model_is_free() {
        let pricing = Pricing::default();
        assert_eq!(pricing.cost_for("ollama::llama3.2", 5000, 5000), 0.0);
    }

    #[test]
    fn context_limit_covers_subscription_bridges() {
        // CLI bridges can't be queried via API — windows are hardcoded here only.
        // Opus and Sonnet both GA'd to 1M context in March 2026; Haiku did not.
        assert_eq!(context_limit("claude-cli::opus"), Some(1_000_000));
        assert_eq!(context_limit("claude-cli::sonnet"), Some(1_000_000));
        assert_eq!(context_limit("claude-cli::haiku"), Some(200_000));
        assert_eq!(context_limit("claude-cli::"), Some(1_000_000));
        assert_eq!(context_limit("codex-cli::"), Some(272_000));
        assert_eq!(context_limit("codex-cli::gpt-5.5"), Some(272_000));
        assert_eq!(context_limit("codex-oauth::gpt-5.6-terra"), Some(272_000));
        assert_eq!(context_limit("codex-oauth::gpt-5.6-sol"), Some(272_000));
        assert_eq!(context_limit("codex-oauth::gpt-5.6-luna"), Some(272_000));
        assert_eq!(context_limit("codex-oauth::gpt-5.3-codex"), Some(272_000));
        assert_eq!(context_limit("agy-cli::"), Some(1_000_000));
        // All non-bridge models return None — their windows come from DB (fetch_and_persist).
        assert_eq!(context_limit("anthropic::claude-opus-4-8"), None);
        assert_eq!(context_limit("gemini::gemini-2.5-pro"), None);
        assert_eq!(context_limit("openai::gpt-4o"), None);
        assert_eq!(context_limit("openrouter::qwen/qwen3-coder:free"), None);
        assert_eq!(context_limit("nvidia::meta/llama-3.1-405b-instruct"), None);
        assert_eq!(context_limit("groq::llama-3.3-70b-versatile"), None);
        assert_eq!(context_limit("ollama::some-local-model"), None);
    }

    #[test]
    fn authoritative_context_limit_covers_qwen_max_upgrade_family() {
        for model in [
            "qwencloud::qwen3.7-max",
            "qwencloud::qwen3.8-max-preview",
            "openrouter::qwen/qwen3.8-max-preview",
        ] {
            assert_eq!(
                authoritative_context_limit(model),
                Some(1_000_000),
                "{model} should use Qwen Max's documented 1M context"
            );
        }
        assert_eq!(authoritative_context_limit("qwencloud::qwen3.7-plus"), None);
        assert_eq!(authoritative_context_limit("qwencloud::qwen3.6-max"), None);
    }

    #[test]
    fn defaults_price_the_paid_models() {
        let pricing = Pricing::default();
        assert!(pricing.cost_for("openai::gpt-4o-mini", 1000, 1000) > 0.0);
        assert!(pricing.cost_for("anthropic::claude-opus-4-8", 1000, 1000) > 0.0);
    }

    #[test]
    fn fable_and_mythos_are_priced_above_opus_not_free() {
        // The metered-path bug: with no DEFAULT_RATES entry, Fable/Mythos priced as $0 (unknown =
        // free), so cost-tiered routing treated the fleet's most expensive models as the cheapest.
        let p = Pricing::default();
        let opus = p.cost_for("anthropic::claude-opus-4-8", 1000, 1000);
        let fable = p.cost_for("anthropic::claude-fable-5", 1000, 1000);
        let mythos = p.cost_for("anthropic::claude-mythos-5", 1000, 1000);
        assert!(fable > 0.0 && mythos > 0.0, "must not price as free");
        assert!(
            fable > opus,
            "fable ({fable}) must be pricier than opus ({opus})"
        );
        assert!(
            (fable - mythos).abs() < 1e-12,
            "fable and mythos price equally"
        );
        // 1000 in @ 0.010/1k + 1000 out @ 0.050/1k = 0.010 + 0.050 = 0.060.
        assert!((fable - 0.060).abs() < 1e-9, "got {fable}");
    }

    #[test]
    fn defaults_price_the_new_byok_providers() {
        let p = Pricing::default();
        assert!(p.cost_for("gemini::gemini-2.5-flash", 1000, 1000) > 0.0);
        assert!(p.cost_for("gemini::gemini-2.5-pro", 1000, 1000) > 0.0);
        assert!(p.cost_for("deepseek::deepseek-chat", 1000, 1000) > 0.0);
        assert!(p.cost_for("xai::grok-4", 1000, 1000) > 0.0);
    }

    #[test]
    fn unpriced_openrouter_model_is_free_not_a_panic() {
        // Gateway models aren't bundled; cost falls back to 0.0 rather than panicking.
        let p = Pricing::default();
        assert_eq!(
            p.cost_for("open_router::deepseek/deepseek-chat", 9999, 9999),
            0.0
        );
    }

    #[test]
    fn fetched_prices_track_otherwise_unpriced_models_config_still_wins() {
        let mut config = forge_config::Config::default();
        // User pins an explicit price for one model.
        config.mesh.pricing.insert(
            "openrouter::vendor/a".to_string(),
            forge_config::PriceOverride {
                input_per_1k: 9.0,
                output_per_1k: 9.0,
            },
        );
        let fetched = vec![
            // Same model the user overrode — config must win.
            ("openrouter::vendor/a".to_string(), 1.0, 1.0, None),
            // A model with no bundled default and no config — fetched gives it a real price.
            ("openrouter::vendor/b".to_string(), 0.5, 2.0, Some(0.05)),
        ];
        let pricing = Pricing::from_config_with_fetched(&config, fetched);
        // vendor/a: config (9.0/9.0) wins over fetched (1.0/1.0).
        assert!((pricing.cost_for("openrouter::vendor/a", 1000, 1000) - 18.0).abs() < 1e-9);
        // vendor/b: previously $0 (unpriced), now tracked from the fetched rate.
        assert!((pricing.cost_for("openrouter::vendor/b", 1000, 1000) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn cost_for_usage_prices_cached_tokens_at_the_discounted_rate() {
        let fetched = vec![("openrouter::m".to_string(), 1.0, 2.0, Some(0.1))];
        let pricing = Pricing::from_config_with_fetched(&forge_config::Config::default(), fetched);
        // 1000 input of which 800 cached, 500 output.
        // fresh 200 @ 1.0/1k = 0.2; cached 800 @ 0.1/1k = 0.08; output 500 @ 2.0/1k = 1.0 → 1.28.
        let usage = forge_types::Usage {
            input_tokens: 1000,
            output_tokens: 500,
            cached_input_tokens: 800,
            cost_usd: 0.0,
        };
        assert!((pricing.cost_for_usage("openrouter::m", &usage) - 1.28).abs() < 1e-9);
        // Without a cache rate, cached tokens fall back to the full input rate (= cost_for).
        let fetched2 = vec![("openrouter::n".to_string(), 1.0, 2.0, None)];
        let pricing2 =
            Pricing::from_config_with_fetched(&forge_config::Config::default(), fetched2);
        let u2 = forge_types::Usage {
            input_tokens: 1000,
            output_tokens: 500,
            cached_input_tokens: 800,
            cost_usd: 0.0,
        };
        assert!(
            (pricing2.cost_for_usage("openrouter::n", &u2)
                - pricing2.cost_for("openrouter::n", 1000, 500))
            .abs()
                < 1e-9
        );
    }

    #[test]
    fn config_overrides_win_over_defaults() {
        let mut config = forge_config::Config::default();
        config.mesh.pricing.insert(
            "openai::gpt-4o-mini".to_string(),
            forge_config::PriceOverride {
                input_per_1k: 1.0,
                output_per_1k: 2.0,
            },
        );
        let pricing = Pricing::from_config(&config);
        // 1000 in * 1.0/1k + 1000 out * 2.0/1k = 1.0 + 2.0 = 3.0
        assert!((pricing.cost_for("openai::gpt-4o-mini", 1000, 1000) - 3.0).abs() < 1e-9);
    }
}
