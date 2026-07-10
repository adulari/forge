//! Transparent capability priors for ranking *discovered* models per task tier (auto-discovery
//! mesh, docs/features/auto-discovery-mesh.md). The "not hardcoded in config" requirement means
//! these priors live here in code (generic model-family heuristics), never as specific ids in a
//! user's config. A model id maps to a coarse (quality, speed) class by family substring; the
//! per-tier score weights those + cost so the router can pick the best *available* model.

use std::collections::HashMap;

use forge_types::TaskTier;

use crate::bench::BenchmarkScores;

/// Divisor that maps an Artificial Analysis index (~0–70, frontier ≈ 60) onto the same 0–3-ish
/// "quality" scale the family heuristic produced, so cost/conservation terms layered on top in the
/// catalog keep working unchanged (a ~60 index ≈ quality 3.0).
const BENCH_INDEX_DIVISOR: f64 = 20.0;

/// Coarse quality class inferred from a model id's family (0 = unknown/small … 3 = frontier).
pub(crate) fn quality_class(id: &str) -> u8 {
    let m = id.to_lowercase();
    // Domain-specialized fine-tunes and stale-generation models slip through the size-only
    // checks below despite NOT being general-purpose frontier-quality: NVIDIA's NIM catalog
    // re-exports hundreds of partner/community models at "frontier" param counts that are
    // narrowly tuned or simply outdated, not comparable to opus/gpt-5/gemini-pro. Checked
    // BEFORE every size check (including -100b) so a big parameter count can't override this —
    // unlike the small-vs-large overrides above, which is by genuine size, this is by KNOWN
    // weak/narrow product family regardless of size.
    //   - codellama: Meta's 2023 code-only model, predates Llama-3; weaker than modern 70B peers.
    //   - palmyra (Writer): narrow domain (finance/medical/creative) fine-tunes, not general.
    //   - stockmark: Japanese-focused instruct model, narrow general/English coverage.
    if m.contains("codellama") || m.contains("palmyra") || m.contains("stockmark") {
        return 2;
    }
    // Explicit large-parameter counts override product-family naming conventions. A model that
    // states its size as ≥100 B is frontier-class regardless of whether "small" appears in its
    // product-line name (e.g. mistral-small-4-119b is 119 B despite "small" in the name).
    // Checked BEFORE the small-marker group so the product name does not misclassify it.
    if m.contains("-100b") || m.contains("-119b") || m.contains("-120b") || m.contains("-123b") {
        return 3;
    }
    // Mid-size param counts (20–60 B) also override product naming — "mistral-small-3-22b" is
    // 22 B (mid-tier) despite "small" in the product-line name. Checked before the small-marker
    // group for the same reason the frontier guard above is checked first.
    if m.contains("-20b")
        || m.contains("-22b")
        || m.contains("-24b")
        || m.contains("-25b")
        || m.contains("-27b")
        || m.contains("-30b")
        || m.contains("-32b")
        || m.contains("-34b")
        || m.contains("-36b")
        || m.contains("-40b")
        || m.contains("-47b")
        || m.contains("-56b")
        || m.contains("-57b")
    {
        return 2;
    }
    // Small / fast FIRST: a size/speed marker (mini, haiku, -lite, -4b, -8b) downgrades even a
    // frontier-family name — `gpt-5.4-mini` and `gpt-4o-mini` are small, not frontier.
    // Use "-mini" (with dash) not "mini" to avoid matching "minimaxai/minimax-*" (large models).
    // Ollama uses colon-size notation (deepseek-r1:7b, qwen3-coder:8b) — ":Nb" variants are
    // also small-model markers even though the frontier name (deepseek-r1, qwen3-coder) matches
    // the frontier group below. The small check runs first so it wins on both separators.
    // Covers dash-notation too: -4b (gemma-3-4b), -11b (llama-3.2-11b), -13b/-14b (qwen/deepseek
    // distills). Colon variants (:4b, :11b-:14b) cover the same Ollama tag shapes.
    if m.contains("-8b")
        || m.contains(":8b")
        || m.contains("-7b")
        || m.contains(":7b")
        || m.contains("-4b")
        || m.contains(":4b")
        || m.contains("-3b")
        || m.contains(":3b")
        || m.contains("-1b")
        || m.contains(":1b")
        || m.contains(":1.")  // catches :1.5b, :1.6b Ollama tags
        || m.contains("-14b")
        || m.contains(":14b")
        || m.contains("-13b")
        || m.contains(":13b")
        || m.contains("-12b")
        || m.contains(":12b")
        || m.contains("-11b")
        || m.contains(":11b")
        || m.contains("-mini")
        || m.contains("nano")
        || m.contains("haiku")
        || m.contains("instant")
        || m.contains("flash-lite")
        || m.contains("-lite")
        || m.contains("small")
    {
        1
    // Frontier / large.
    } else if m.contains("opus")
        || m.contains("gpt-5")
        || m.contains("sonnet")
        || m.contains("-405b")
        || m.contains("-235b")
        || m.contains("-72b")
        || m.contains("-70b")
        || m.contains("deepseek-r1")
        || (m.contains("deepseek-v4") && !m.contains("flash"))
        || m.contains("qwen3-coder")
        || m.contains("grok-4")
    {
        3
    // Strong mid.
    } else if m.contains("gpt-4")
        || m.contains("-32b")
        || m.contains("-34b")
        || m.contains("gemini-3")
        || m.contains("gemini-2.5-pro")
        || m.contains("deepseek")
        || m.contains("large")
        || (m.contains("pro") && !m.contains("flash"))
    {
        2
    } else {
        // Unknown family — assume a capable default (e.g. `flash`, `llama3.2`, codex models).
        2
    }
}

/// Minimum Artificial Analysis intelligence index that qualifies a model as "frontier-class" for
/// the conservation guard (Complex alternative) and overview stats. Calibrated to exclude
/// nominally-large but measurably-weak older models (Llama 3.3 70B = 10.0, Hermes 405B = 9.0)
/// while including capable modern ones (DeepSeek R1 = 20.1, Gemini 2.5 Pro = 27.0).
pub(crate) const FRONTIER_BENCH_THRESHOLD: f64 = 20.0;

/// Minimum intelligence index for a "capable mid" model — used in the Standard-tier conservation
/// guard. Excludes the weakest small models (Llama 3.1 8B = 6.1, GPT-4o-mini = 6.9) while
/// retaining capable ones (Llama 3.3 70B = 10.0, GPT-4o = 12.3).
pub(crate) const CAPABLE_BENCH_THRESHOLD: f64 = 8.0;

/// Ranking demotion for models with unreliable STRUCTURED tool-calling. Forge is a tool-driven
/// harness: a model that emits tool calls as TEXT instead of structured calls is a poor pick even
/// when its raw intelligence/coding bench ranks it top. `forge-provider::tool_recovery` salvages
/// the leaked markup, but only after a wasted round-trip (and weaker models can stall outright), so
/// we'd rather route to an equally-capable peer that calls tools cleanly. The penalty is sized to
/// drop an offender BELOW a comparable tool-reliable model while keeping it in the fallback chain.
const TOOL_UNRELIABLE_PENALTY: f64 = 3.0;

/// Tool-call-reliability penalty for `id` (0.0 = clean). Evidence-based, not a capability judgement:
/// the **Gemini *flash* family** leaks function-call markup as text (`<function=…>` / `<invoke>`)
/// observed both via genai's native adapter and through OpenRouter, despite a top intelligence
/// score. Matched by name so it spans providers (`gemini::…`, `openrouter::google/gemini-…-flash`).
/// Reversible: drop the entry once the upstream tool-call parsing is fixed.
pub(crate) fn tool_reliability_penalty(id: &str) -> f64 {
    let l = id.to_lowercase();
    if l.contains("gemini") && l.contains("flash") {
        TOOL_UNRELIABLE_PENALTY
    } else {
        0.0
    }
}

/// Whether a model id reads as frontier-class — benchmark-aware when scores are available. A
/// measured intelligence index ≥ `FRONTIER_BENCH_THRESHOLD` supersedes the name heuristic, so
/// nominally-large but measurably-weak old models (Hermes 405B = 9.0) are correctly excluded while
/// unnamed but high-scoring models are correctly included. Falls back to the name heuristic when
/// no score exists for the model.
pub fn is_frontier_b(id: &str, bench: Option<&BenchmarkScores>) -> bool {
    match bench.and_then(|b| b.score_for(id)) {
        Some(s) => s.intelligence >= FRONTIER_BENCH_THRESHOLD,
        None => quality_class(id) == 3,
    }
}

/// Whether a model id reads as frontier-class (top quality prior) — used to count "frontier"
/// models in the `/models` overview. Heuristic-only; the bench-aware version is [`is_frontier_b`].
pub fn is_frontier(id: &str) -> bool {
    is_frontier_b(id, None)
}

/// Bare model name after the `provider::` prefix (`"opus"` from `"claude-cli::opus"`,
/// `"gpt-5.6-sol"` from `"codex-oauth::gpt-5.6-sol"`). Mirrors the split-once idiom used elsewhere
/// in this crate (`bench.rs`, `pricing.rs::context_limit`) for stripping the provider prefix.
fn bare_model(id: &str) -> &str {
    id.split_once("::").map(|(_, m)| m).unwrap_or(id)
}

/// Bundled relative subscription plan-burn weights (docs/design/subscription-efficiency-routing.md,
/// Fix 1), normalized so the cheapest sibling of a family = 1.0. Derived from Anthropic's + OpenAI's
/// published list prices on a nominal 1000-in/500-out mix (`in + 0.5·out`, Haiku 4.5 = the 1.0
/// baseline) — do NOT add entries beyond this table without the same derivation; an absent entry
/// correctly defaults to neutral (1.0) rather than a guess. Weights, per verified list prices:
///   - GPT-5.6 Sol $5/$30 → 5.0, Terra $2.50/$15 → 2.5, Luna $1/$6 → 1.0
///   - Claude Fable 5 / Mythos 5 $10/$50 → 10.0 (the fleet's most expensive models; a neutral
///     default here would leave the single most expensive model with ZERO burn penalty)
///   - Claude Opus 4.8 $5/$25 → 5.0, Sonnet 5 $3/$15 (steady-state) → 3.0, Haiku 4.5 $1/$5 → 1.0
///
/// Family words are matched via [`crate::bench::tokens`] — the SAME tokenizer `bench::score_for`
/// uses — so a family resolves identically under every id form the catalog produces (the API id
/// `anthropic::claude-fable-5`, the `claude-cli::fable` bridge alias, `codex-oauth::gpt-5.6-sol`,
/// etc.) rather than a single hardcoded literal. Returns `Some` only for a KNOWN id, so
/// `subscription_burn_weight` can tell a known weight of 1.0 (Luna/Haiku, genuinely cheapest) apart
/// from an unknown model that merely defaults to 1.0. Backs the `route_score` burn penalty (Fix 2)
/// for every entry here. `speed_class` (Fix 3) only consults the narrower
/// [`gpt56_family_speed_weight`] — see its doc comment for why the Claude entries are excluded from
/// that specific substitution.
fn known_burn_weight(id: &str) -> Option<f64> {
    let toks = crate::bench::tokens(bare_model(id));
    let has = |w: &str| toks.iter().any(|t| t == w);
    // GPT-5.6 family: the sub-name (sol/terra/luna) is the family identifier.
    if has("sol") {
        Some(5.0)
    } else if has("terra") {
        Some(2.5)
    } else if has("luna") {
        Some(1.0)
    // Claude families, matched on the family word. Fable/Mythos checked before Opus: their real AA
    // source name carries an "(… Opus 4.8 Fallback)" tail, but Forge ids never do — this ordering
    // is just belt-and-suspenders so a stray "opus" token can never outrank the fable/mythos match.
    } else if has("fable") || has("mythos") {
        Some(10.0)
    } else if has("opus") {
        Some(5.0)
    } else if has("sonnet") {
        // Steady-state $3/$15 per 1M. Sonnet 5 is on introductory pricing ($2/$10 per 1M) through
        // 2026-08-31, reverting to $3/$15 on 2026-09-01; the steady-state value is encoded
        // deliberately so this constant doesn't silently go stale the day the intro price expires.
        Some(3.0)
    } else if has("haiku") {
        Some(1.0)
    } else {
        None
    }
}

/// Relative subscription plan-burn weight for `id` (Fix 1): how much of the plan quota one
/// equivalent request costs relative to the cheapest sibling of its family (weight 1.0). Config
/// overrides (`mesh.burn_weights`, keyed by the BARE model name) take precedence over the bundled
/// table. Unknown models default to 1.0 (neutral, no penalty) — load-bearing: an unpriced/unknown
/// model must get zero penalty in `route_score` so nothing regresses.
pub(crate) fn subscription_burn_weight(id: &str, overrides: &HashMap<String, f64>) -> f64 {
    let bare = bare_model(id);
    if let Some(&w) = overrides.get(bare) {
        return w;
    }
    known_burn_weight(id).unwrap_or(1.0)
}

/// The subset of `known_burn_weight`'s table that `speed_class` is allowed to derive its class
/// from: the GPT-5.6 family (`sol`/`terra`/`luna`), where all three siblings have a dedicated,
/// distinctly-valued entry — the exact case D2 diagnosed (all three tied at the slowest class by
/// name alone). The Claude entries (`opus`/`sonnet`/`haiku`) are intentionally EXCLUDED here even
/// though they're valid `known_burn_weight` lookups for `subscription_burn_weight`'s route-score
/// penalty: `speed_class` feeds a shared 1–3 scale compared directly against every OTHER model
/// family, most of which (including today's `gpt-5.4`/`gpt-5.5`, not yet renamed to the 5.6 line)
/// have no burn-weight entry and stay pinned at the coarse quality_class-derived speed. Deriving
/// Claude's speed from burn weight would give `sonnet` a `speed_class` bump vs. those still-coarse
/// peers with no matching upgrade — verified live: it let `claude-cli::sonnet` outscore
/// `codex-cli::gpt-5.4`/`gpt-5.5` on every Standard AND Complex prompt in a mixed catalog, silently
/// re-introducing the single-provider monopoly `routing_spreads_across_providers_not_only_claude`
/// exists to prevent. `subscription_burn_weight`'s route-score penalty already penalizes Claude's
/// heavier siblings correctly; this file scopes speed_class narrowly to the one case it was asked
/// to fix.
fn gpt56_family_speed_weight(id: &str) -> Option<f64> {
    match bare_model(id).to_lowercase().as_str() {
        "gpt-5.6-sol" => Some(5.0),
        "gpt-5.6-terra" => Some(2.5),
        "gpt-5.6-luna" => Some(1.0),
        _ => None,
    }
}

/// Coarse speed class — roughly the inverse of size (3 = fastest small model). When the id is a
/// KNOWN member of the GPT-5.6 family (see [`gpt56_family_speed_weight`]), the class is derived
/// from relative burn (cheapest sibling → fast/3, mid → 2, flagship → 1) instead of the name
/// heuristic — this is what lets `gpt-5.6-luna` score as fast as it actually is even though its
/// name carries no size/speed marker (Fix 3; luna/terra/sol all landed in the same slowest class
/// under the old heuristic). Falls back to the size/quality-class heuristic, UNCHANGED, for every
/// other id (`mini`, `-30b`, `opus`/`sonnet`/`haiku`, unfamiliar ids, …).
pub(crate) fn speed_class(id: &str) -> u8 {
    if let Some(w) = gpt56_family_speed_weight(id) {
        return if w <= 1.0 {
            3
        } else if w <= 3.0 {
            2
        } else {
            1
        };
    }
    match quality_class(id) {
        3 => 1,
        2 => 2,
        _ => 3,
    }
}

/// The pure *capability* fit of a model for a tier (higher = better), with no cost/provider terms
/// — those are layered on in the catalog's routing score so cost-tiering + spread stay in one
/// place. Trivial favours speed, Complex favours quality, Standard balances. Heuristic-only
/// (no benchmark data); thin wrapper over [`capability_score_b`]. Test-only — production paths call
/// `capability_score_b` directly so they can pass benchmark data + code-heaviness.
#[cfg(test)]
pub(crate) fn capability_score(id: &str, tier: TaskTier) -> f64 {
    capability_score_b(id, tier, false, None)
}

/// Capability fit, preferring REAL benchmark scores (ADR-0011) when available. The quality term is
/// the measured index (coding index for `code_heavy` tasks, else the general intelligence index)
/// scaled onto the heuristic's 0–3 range; speed stays a size-derived heuristic (benchmarks don't
/// rank "fast for a trivial edit"). Falls back to the family `quality_class` when the model has no
/// score, so a missing/disabled benchmark layer changes nothing.
pub(crate) fn capability_score_b(
    id: &str,
    tier: TaskTier,
    code_heavy: bool,
    bench: Option<&BenchmarkScores>,
) -> f64 {
    let (q, s) = match bench.and_then(|b| b.score_for(id)) {
        Some(score) => {
            let index = if code_heavy {
                score.coding
            } else {
                score.intelligence
            };
            (
                (index / BENCH_INDEX_DIVISOR).clamp(0.0, 4.0),
                speed_class(id) as f64,
            )
        }
        None => (quality_class(id) as f64, speed_class(id) as f64),
    };
    match tier {
        TaskTier::Trivial => s * 2.0 + q * 0.5,
        TaskTier::Standard => q + s,
        TaskTier::Complex => q * 2.0 + s * 0.25,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_reliability_penalty_flags_gemini_flash_across_providers_only() {
        // The Gemini flash family leaks tool calls as text — penalized regardless of provider prefix.
        for id in [
            "gemini::gemini-3.5-flash",
            "openrouter::google/gemini-3.5-flash",
            "gemini::gemini-2.5-flash-lite",
            "gemini::gemini-flash-latest",
        ] {
            assert!(
                tool_reliability_penalty(id) > 0.0,
                "gemini flash must be penalized: {id}"
            );
        }
        // Tool-reliable models (incl. non-flash Gemini) are not penalized.
        for id in [
            "claude-cli::sonnet",
            "openai::gpt-5.5",
            "gemini::gemini-3-pro-preview",
            "openrouter::deepseek/deepseek-v4",
            "groq::llama-3.3-70b-versatile",
        ] {
            assert_eq!(
                tool_reliability_penalty(id),
                0.0,
                "must not be penalized: {id}"
            );
        }
    }

    #[test]
    fn trivial_prefers_a_fast_small_model_over_a_frontier_one() {
        let small = capability_score("groq::llama-3.1-8b-instant", TaskTier::Trivial);
        let big = capability_score("anthropic::claude-opus-4-8", TaskTier::Trivial);
        assert!(
            small > big,
            "trivial should favour the fast/small model: {small} vs {big}"
        );
    }

    #[test]
    fn complex_prefers_a_frontier_model_over_a_tiny_one() {
        let big = capability_score("anthropic::claude-opus-4-8", TaskTier::Complex);
        let small = capability_score("groq::llama-3.1-8b-instant", TaskTier::Complex);
        assert!(
            big > small,
            "complex should favour the strong model: {big} vs {small}"
        );
    }

    #[test]
    fn mini_and_haiku_are_small_not_frontier() {
        // The reorder fix: a size marker downgrades even a frontier-family name.
        assert!(!is_frontier("codex-cli::gpt-5.4-mini"));
        assert!(!is_frontier("openai::gpt-4o-mini"));
        assert!(!is_frontier("claude-cli::haiku"));
        assert!(is_frontier("codex-cli::gpt-5.4"));
        assert!(is_frontier("claude-cli::opus"));
    }

    #[test]
    fn ollama_colon_size_tags_are_classified_as_small() {
        // Ollama uses colon separators: deepseek-r1:7b, qwen3-coder:8b, deepseek-r1:1.5b.
        // Without the ":Nb" checks these pass the small-group (-7b etc.) and hit the frontier
        // check (deepseek-r1, qwen3-coder) → quality_class=3 for a 7B distilled model.
        assert_eq!(
            quality_class("ollama::deepseek-r1:7b"),
            1,
            "distilled 7b must be small"
        );
        assert_eq!(quality_class("ollama::deepseek-r1:8b"), 1);
        assert_eq!(quality_class("ollama::deepseek-r1:1.5b"), 1);
        assert_eq!(quality_class("ollama::qwen3-coder:7b"), 1);
        assert_eq!(quality_class("ollama::qwen3-coder:8b"), 1);
        // :4b — small distilled model, not in original list (was misclassified as frontier).
        assert_eq!(
            quality_class("ollama::deepseek-r1:4b"),
            1,
            ":4b must be small"
        );
        assert_eq!(quality_class("ollama::qwen3-coder:4b"), 1);
        // :11b/:13b/:14b — small distilled sizes, were misclassified as frontier.
        assert_eq!(
            quality_class("ollama::deepseek-r1:14b"),
            1,
            ":14b must be small"
        );
        assert_eq!(quality_class("ollama::qwen3-coder:14b"), 1);
        assert_eq!(quality_class("ollama::llama3:11b"), 1);
        assert_eq!(quality_class("ollama::codestral:12b"), 1);
        assert_eq!(quality_class("ollama::llama2:13b"), 1);
        // Dash-notation small sizes not previously covered.
        assert_eq!(
            quality_class("openrouter::google/gemma-3-4b-it"),
            1,
            "-4b must be small"
        );
        assert_eq!(
            quality_class("openrouter::meta-llama/llama-3.2-11b-vision"),
            1,
            "-11b must be small"
        );
        assert_eq!(
            quality_class("openrouter::qwen/qwen2.5-14b-instruct"),
            1,
            "-14b must be small"
        );
        // 30B+ Ollama tags are not in the small list — they should be default/frontier.
        assert!(
            quality_class("ollama::deepseek-r1:70b") >= 2,
            "70b is not small"
        );
        assert!(
            quality_class("ollama::qwen3-coder:30b") >= 2,
            "30b is not small"
        );
    }

    #[test]
    fn mid_param_count_overrides_small_product_name() {
        // A model with "small" in the product-line name but an explicit 20–60 B param count must
        // NOT be classified as tiny (quality_class=1). The param-count early return fires first.
        assert!(
            quality_class("openrouter::mistralai/mistral-small-3-22b") >= 2,
            "22 B model must not be tiny despite 'small' in product name"
        );
        assert!(
            quality_class("something::model-small-32b-instruct") >= 2,
            "32 B model must not be tiny"
        );
        assert!(
            quality_class("something::small-40b") >= 2,
            "40 B model must not be tiny"
        );
        // Normal small models (no param-count override) still downgrade correctly.
        assert_eq!(quality_class("mistral::mistral-small-2506"), 1);
    }

    #[test]
    fn niche_or_stale_70b_models_are_not_frontier() {
        // Live mesh bug: `forge mesh` tied codellama-70b + 3 palmyra variants + stockmark-2-100b
        // at #1 on Complex (ahead of bench-scored opus) purely because they match `-70b`/`-100b`.
        // None of these are general-purpose frontier-quality models:
        //   - codellama-70b: Meta's 2023 code-only model, predates Llama-3.
        //   - palmyra-fin/med (Writer): narrow finance/medical domain fine-tunes.
        //   - stockmark-2-100b: Japanese-focused, not a general/English frontier model.
        for id in [
            "nvidia::meta/codellama-70b",
            "nvidia::writer/palmyra-fin-70b-32k",
            "nvidia::writer/palmyra-med-70b",
            "nvidia::writer/palmyra-med-70b-32k",
            "nvidia::stockmark/stockmark-2-100b-instruct",
        ] {
            assert_eq!(
                quality_class(id),
                2,
                "{id} must not be classified frontier purely by param count"
            );
        }
        // Genuine large general-purpose models are unaffected — still frontier.
        assert_eq!(quality_class("nvidia::meta/llama-3.3-70b-instruct"), 3);
        assert_eq!(quality_class("openrouter::qwen/qwen2.5-72b-instruct"), 3);
    }

    #[test]
    fn deepseek_v4_flash_is_not_frontier() {
        // deepseek-v4 → quality_class=3 (frontier), but a flash variant is lighter.
        // The pro/flash guard already exists for other families; apply the same to deepseek-v4.
        assert!(
            quality_class("openrouter::deepseek/deepseek-v4") >= 3,
            "full deepseek-v4 is frontier"
        );
        assert!(
            quality_class("opencode_go::deepseek-v4-flash") < 3,
            "deepseek-v4-flash must not be frontier: {}",
            quality_class("opencode_go::deepseek-v4-flash")
        );
    }

    #[test]
    fn large_param_count_overrides_small_product_name() {
        // "mistral-small-4-119b" is 119 B — product-family "small" must NOT give it speed_class=3.
        // Same false-speed-boost bug as minimax-m3 (which matched "mini"). The -119b guard fires
        // first so these get quality_class=3 (frontier), speed_class=1.
        assert_eq!(
            quality_class("nvidia::mistralai/mistral-small-4-119b-2603"),
            3,
            "119 B model must be frontier despite 'small' in product name"
        );
        assert_eq!(quality_class("something::model-123b-instruct"), 3);
        assert_eq!(quality_class("something::model-120b"), 3);
        // Normal "small" models still downgrade.
        assert_eq!(quality_class("mistral::mistral-small-2506"), 1);
        assert_eq!(quality_class("openai::gpt-4o-mini"), 1);
    }

    #[test]
    fn minimax_is_not_classified_as_small() {
        // "minimax" contains "mini" as a substring — guard against that false match.
        // MiniMax M3 is a large frontier model; it must NOT get quality_class=1 (tiny/fast).
        assert!(quality_class("nvidia::minimaxai/minimax-m3") > 1);
        assert!(quality_class("nvidia::minimaxai/minimax-m2.7") > 1);
        // Real -mini models must still be downgraded.
        assert_eq!(quality_class("openai::gpt-4o-mini"), 1);
        assert_eq!(quality_class("codex-cli::gpt-5.4-mini"), 1);
        // trivial tier: minimax must NOT outscore a real fast model due to speed_class inflation.
        let minimax = capability_score("nvidia::minimaxai/minimax-m3", TaskTier::Trivial);
        let fast = capability_score("groq::llama-3.1-8b-instant", TaskTier::Trivial);
        assert!(
            fast >= minimax,
            "trivial: a genuinely fast small model ({fast}) should beat minimax-m3 ({minimax})"
        );
    }

    #[test]
    fn subscription_burn_weight_matches_the_bundled_table() {
        let no_overrides = HashMap::new();
        for prefix in ["codex-oauth::", "codex-cli::"] {
            assert_eq!(
                subscription_burn_weight(&format!("{prefix}gpt-5.6-sol"), &no_overrides),
                5.0
            );
            assert_eq!(
                subscription_burn_weight(&format!("{prefix}gpt-5.6-terra"), &no_overrides),
                2.5
            );
            assert_eq!(
                subscription_burn_weight(&format!("{prefix}gpt-5.6-luna"), &no_overrides),
                1.0
            );
        }
        assert_eq!(
            subscription_burn_weight("claude-cli::opus", &no_overrides),
            5.0
        );
        assert_eq!(
            subscription_burn_weight("claude-cli::sonnet", &no_overrides),
            3.0
        );
        assert_eq!(
            subscription_burn_weight("claude-cli::haiku", &no_overrides),
            1.0
        );
        // Fable 5 / Mythos 5 — the fleet's most expensive models ($10/$50 per 1M) → weight 10.0.
        assert_eq!(
            subscription_burn_weight("claude-cli::fable", &no_overrides),
            10.0
        );
        assert_eq!(
            subscription_burn_weight("anthropic::claude-mythos-5", &no_overrides),
            10.0
        );
    }

    #[test]
    fn fable_and_mythos_resolve_to_10_under_every_catalog_id_form() {
        let no_overrides = HashMap::new();
        // Both the API id and the claude-cli bridge alias must resolve — bench.rs proves both are
        // live, catalog-produced id forms (see `bare_bridge_alias`/`claude-cli::fable` coverage).
        for id in ["anthropic::claude-fable-5", "claude-cli::fable"] {
            assert_eq!(
                subscription_burn_weight(id, &no_overrides),
                10.0,
                "fable must resolve to 10.0 under id form {id}"
            );
        }
        // Mythos has no bridge alias today (the claude-cli list is fable/opus/sonnet/haiku), but
        // its API id must resolve; a future bare alias would too (token match on "mythos").
        for id in ["anthropic::claude-mythos-5", "claude-cli::mythos"] {
            assert_eq!(
                subscription_burn_weight(id, &no_overrides),
                10.0,
                "mythos must resolve to 10.0 under id form {id}"
            );
        }
    }

    #[test]
    fn fable_does_not_collide_with_the_opus_arm() {
        // Regression guard for the known "Opus 4.8 Fallback" cross-match trap (bench.rs:304-321):
        // Fable's benchmark row NAME contains "Opus", but its Forge id does not — so `claude-fable-5`
        // must resolve via the fable arm (10.0), NEVER the opus arm (5.0). Opus itself stays 5.0.
        let no_overrides = HashMap::new();
        assert_eq!(
            subscription_burn_weight("anthropic::claude-fable-5", &no_overrides),
            10.0,
            "fable must not fall through to the opus (5.0) arm"
        );
        assert_eq!(
            subscription_burn_weight("anthropic::claude-opus-4-8", &no_overrides),
            5.0,
            "opus regression guard: still 5.0"
        );
    }

    #[test]
    fn speed_class_for_fable_is_unchanged_and_never_leaks_into_the_speed_path() {
        // Fable is a known burn-weight entry (10.0) for the route-score penalty, but it must NOT be
        // in `gpt56_family_speed_weight` — else it would re-trigger the provider-monopoly regression.
        // Its speed_class must equal the plain quality_class fallback (2 → speed 2), unchanged.
        assert_eq!(speed_class("claude-cli::fable"), 2);
        assert_eq!(speed_class("anthropic::claude-fable-5"), 2);
    }

    #[test]
    fn subscription_burn_weight_defaults_unknown_models_to_neutral() {
        let no_overrides = HashMap::new();
        for id in [
            "groq::llama-3.1-8b-instant",
            "openai::gpt-5.5",
            "codex-cli::gpt-5.4-mini",
            "ollama::llama3.2",
        ] {
            assert_eq!(
                subscription_burn_weight(id, &no_overrides),
                1.0,
                "unknown model must default to neutral: {id}"
            );
        }
    }

    #[test]
    fn subscription_burn_weight_config_override_wins_over_table() {
        let mut overrides = HashMap::new();
        overrides.insert("gpt-5.6-sol".to_string(), 9.0);
        assert_eq!(
            subscription_burn_weight("codex-oauth::gpt-5.6-sol", &overrides),
            9.0,
            "config override must win over the bundled table"
        );
        // An override keyed by the bare name applies across both codex bridge prefixes.
        assert_eq!(
            subscription_burn_weight("codex-cli::gpt-5.6-sol", &overrides),
            9.0
        );
        // Unrelated ids are unaffected by an override for a different model.
        assert_eq!(
            subscription_burn_weight("codex-oauth::gpt-5.6-terra", &overrides),
            2.5
        );
    }

    #[test]
    fn speed_class_derives_from_burn_weight_for_the_gpt56_family() {
        // The bug this fixes: sol/terra/luna carry no size/speed marker in their name, so the old
        // heuristic landed all three in quality_class 3 → speed_class 1 (slowest), scoring Luna
        // (the cheapest, fastest sibling) exactly as slow as Sol.
        let sol = speed_class("codex-oauth::gpt-5.6-sol");
        let terra = speed_class("codex-oauth::gpt-5.6-terra");
        let luna = speed_class("codex-oauth::gpt-5.6-luna");
        assert!(
            luna > terra && terra > sol,
            "expected luna({luna}) > terra({terra}) > sol({sol})"
        );
        assert_eq!((sol, terra, luna), (1, 2, 3));
    }

    #[test]
    fn speed_class_name_heuristic_unchanged_for_ids_with_no_burn_weight_entry() {
        // Ids with no burn-weight table entry must keep EXACTLY today's name-heuristic behaviour.
        assert_eq!(speed_class("codex-cli::gpt-5.4-mini"), 3);
        assert_eq!(speed_class("something::model-30b"), 2);
        assert_eq!(
            speed_class("groq::llama-3.1-8b-instant"),
            3,
            "small fast model unaffected"
        );
        assert_eq!(
            speed_class("nvidia::minimaxai/minimax-m3"),
            2,
            "unfamiliar frontier-ish family unaffected"
        );
    }
}
