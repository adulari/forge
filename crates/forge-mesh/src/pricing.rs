//! Per-model pricing and cost computation (FR-5, A-7). Rates are bundled defaults and
//! user-overridable via config, so a provider price change needs no release.

use std::collections::HashMap;

/// USD price per 1,000 tokens for a model's input and output.
#[derive(Debug, Clone, Copy)]
pub struct ModelRate {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
}

impl From<forge_config::PriceOverride> for ModelRate {
    fn from(o: forge_config::PriceOverride) -> Self {
        ModelRate {
            input_per_1k: o.input_per_1k,
            output_per_1k: o.output_per_1k,
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
const DEFAULT_RATES: &[(&str, f64, f64)] = &[
    ("openai::gpt-4o-mini", 0.00015, 0.0006),
    ("anthropic::claude-opus-4-8", 0.015, 0.075),
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

    /// Compute the USD cost of a call given token counts. Unknown models cost nothing.
    pub fn cost_for(&self, model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
        match self.rates.get(model) {
            Some(rate) => {
                (input_tokens as f64 / 1000.0) * rate.input_per_1k
                    + (output_tokens as f64 / 1000.0) * rate.output_per_1k
            }
            None => 0.0,
        }
    }

    /// A *relative* cost comparator for routing: the price of a nominal turn (1000 in / 500
    /// out). Not a forecast — only used to rank candidate models against each other. Unpriced
    /// models (local, gateways) compare as 0.0 (cheapest).
    pub fn estimated_cost(&self, model: &str) -> f64 {
        self.cost_for(model, NOMINAL_INPUT_TOKENS, NOMINAL_OUTPUT_TOKENS)
    }
}

/// Nominal token mix used only to rank candidate models by relative cost.
const NOMINAL_INPUT_TOKENS: u64 = 1000;
const NOMINAL_OUTPUT_TOKENS: u64 = 500;

/// A conservative context window (tokens) assumed for a model we have NO better figure for —
/// neither a fetched window (provider API) nor a family match in [`context_limit`]. 32k is the
/// common floor for modern chat models, so trimming a transcript to this rarely overflows an
/// unknown model while still letting a real turn through. Used by the core to bound what it sends.
pub const CONSERVATIVE_CONTEXT_WINDOW: u32 = 32_000;

/// The context-window size (in tokens) for a model id, or `None` when we don't have a
/// well-established figure. Matched by family substring on the id (after any `provider::`), with a
/// provider fallback for the subscription bridges (whose bare ids carry no model name). This is the
/// *heuristic* layer: a fetched per-model window (provider API, persisted in the store) should take
/// precedence — see the core's effective-window lookup. Returns `None` for a truly unknown model so
/// the statusline can omit a fabricated denominator; the core falls back to
/// [`CONSERVATIVE_CONTEXT_WINDOW`] only when it must actually bound a request.
pub fn context_limit(model: &str) -> Option<u32> {
    let provider = model.split("::").next().unwrap_or("");
    let m = model.rsplit("::").next().unwrap_or(model).to_lowercase();
    let has = |s: &str| m.contains(s);
    // 256k+ frontier long-context families.
    let frontier_256k = has("gpt-5") || has("o1") || has("o3") || has("o4");
    // 128k is the modern default for most capable open / hosted models.
    let mid_128k = has("gpt-4o")
        || has("gpt-4.1")
        || has("gpt-4")
        || has("gpt-oss")
        || has("llama-4")
        || has("llama4")
        || has("llama-3")
        || has("llama3")
        || has("glm")
        || has("kimi")
        || has("phi")
        || has("command")
        || has("cohere")
        || has("nemotron")
        || has("grok")
        || has("nova")
        || has("minimax")
        || has("mimo");
    let limit = if has("claude") || has("opus") || has("sonnet") || has("haiku") {
        200_000
    } else if has("gemini") {
        1_000_000
    } else if frontier_256k {
        256_000
    } else if mid_128k {
        128_000
    } else if has("deepseek") {
        64_000
    } else if has("qwen") || has("mistral") || has("mixtral") || has("gemma") {
        32_000
    } else {
        // The subscription bridges carry no model name in their bare id (`claude-cli::`), so match
        // on the provider: Claude Code ≈ 200k, Codex/gpt-5 ≈ 256k. Generous but real.
        match provider {
            "claude-cli" => 200_000,
            "codex-cli" => 256_000,
            _ => return None,
        }
    };
    Some(limit)
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
    fn context_limit_known_families_and_none_for_unknown() {
        assert_eq!(context_limit("anthropic::claude-opus-4-8"), Some(200_000));
        assert_eq!(context_limit("gemini::gemini-2.5-pro"), Some(1_000_000));
        assert_eq!(context_limit("openai::gpt-4o"), Some(128_000));
        // Common free families now have conservative figures so the core can bound a turn.
        assert_eq!(
            context_limit("openrouter::qwen/qwen3-coder:free"),
            Some(32_000)
        );
        assert_eq!(
            context_limit("openrouter::openai/gpt-oss-120b:free"),
            Some(128_000)
        );
        assert_eq!(
            context_limit("openrouter::nvidia/nemotron-3-nano-30b-a3b:free"),
            Some(128_000)
        );
        // Subscription bridges match on the provider (bare id carries no model name).
        assert_eq!(context_limit("claude-cli::"), Some(200_000));
        assert_eq!(context_limit("codex-cli::gpt-5.5"), Some(256_000));
        // A truly unknown model → None (the gauge shows used tokens, no fake denominator).
        assert_eq!(context_limit("ollama::some-local-model"), None);
    }

    #[test]
    fn defaults_price_the_paid_models() {
        let pricing = Pricing::default();
        assert!(pricing.cost_for("openai::gpt-4o-mini", 1000, 1000) > 0.0);
        assert!(pricing.cost_for("anthropic::claude-opus-4-8", 1000, 1000) > 0.0);
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
