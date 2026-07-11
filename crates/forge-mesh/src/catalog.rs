//! A live catalog of usable models, discovered from the providers the user has keys for
//! (auto-discovery mesh, docs/features/mesh-routing.md). This is a plain data holder +
//! ranking; the async *discovery* (querying each provider's model list) lives in the binary
//! (forge-cli), which has the provider client — forge-mesh stays free of that dependency.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use forge_types::{EffortLevel, TaskTier};

/// Integer bench-score band for effort-biased ranking. Banding — rather than the old pairwise
/// "prefer the higher score when the gap is ≥ 1.0" early-return — keeps the sort comparator a
/// TOTAL order: the pairwise rule was intransitive (a within 1 of b, b within 1 of c, yet a and
/// c more than 1 apart yields contradictory orderings) and panicked Rust's sort with
/// "user-provided comparison function does not correctly implement a total order" the first time
/// a white-hot turn ranked a full multi-hundred-model catalog. Unbenched models (`None`) band to
/// `i64::MIN`, sorting below every benched model at high effort — proven quality was the ask.
/// Documented in docs/features/mesh-routing.md.
fn bench_band(score: Option<f64>) -> i64 {
    score.map(|v| v.floor() as i64).unwrap_or(i64::MIN)
}

use crate::bench::BenchmarkScores;
use crate::capability::{capability_score_b, is_frontier_b, CAPABLE_BENCH_THRESHOLD};
use crate::pricing::Pricing;

/// Discovered `provider::model` ids the user can actually use right now.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Documented in docs/features/mesh-routing.md.
pub struct ModelCatalog {
    /// Deserialization applies the same bare-id guard as [`ModelCatalog::new`], so a catalog
    /// cache written before the guard existed can't re-introduce empty-named bridge rows.
    #[serde(deserialize_with = "de_named_models")]
    models: Vec<String>,
    /// Measured performance scores (ADR-0011), attached at discovery. When present the router ranks
    /// on real benchmark data; when absent it falls back to the family-name heuristic.
    bench: Option<BenchmarkScores>,
    /// Config overrides for `subscription_burn_weight` (`mesh.burn_weights`), keyed by bare model
    /// name. Empty by default — a no-op, `route_score` falls back to the bundled table.
    #[serde(default)]
    burn_weights: HashMap<String, f64>,
}

/// The provider prefix of a `provider::model` id (`"groq"` from `"groq::llama-3.1-8b"`).
/// Documented in docs/features/mesh-routing.md.
pub fn provider_of(id: &str) -> &str {
    id.split("::").next().unwrap_or(id)
}

fn de_named_models<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let mut models = Vec::<String>::deserialize(d)?;
    models.retain(|m| !m.ends_with("::"));
    Ok(models)
}

/// A $0-marginal subscription bridge (the locally-installed claude/codex CLI) or subscription
/// OAuth provider (`xai-oauth::`), as opposed to a metered or genuinely-free API. Kept separate
/// from "free" in the overview counts.
/// Documented in docs/features/mesh-routing.md.
pub fn is_subscription(id: &str) -> bool {
    id.starts_with("claude-cli::")
        || id.starts_with("codex-cli::")
        || id.starts_with("agy-cli::")
        || id.starts_with("xai-oauth::")
        || id.starts_with("codex-oauth::")
}

/// Whether a model is genuinely free to call. "Free" needs *positive* evidence, not just a missing
/// price: OpenRouter is a paid gateway exposing hundreds of metered models (incl. frontier ones
/// like Claude Opus) that we hold no per-model price for — reading "unpriced" as "free" there is
/// the bug. So for OpenRouter, only its `:free`-suffixed variants count; everything else is paid.
/// OpenCode Zen (`opencode_go`) is the same trap: a curated gateway that mixes genuinely-free
/// models with premium ones (glm/kimi/qwen-max) — all billed against ONE shared key balance, none
/// priced in our table. Treating its unpriced premium models as free silently burns that balance
/// (the bug the user hit), so it's paid-by-default too; mark a known-free one via a `:free` suffix
/// or a config price of `0`. Other unpriced providers (local `ollama::`, free-tier
/// `groq`/`cerebras`) are genuinely free.
/// Documented in docs/features/mesh-routing.md.
pub fn is_free(id: &str, cost: f64, subscription: bool) -> bool {
    if subscription {
        return false;
    }
    let provider = provider_of(id);
    // Standing free tiers remain free even when the bundled rate table records their avoided cost.
    if matches!(provider, "ollama" | "groq") || (provider == "gemini" && !id.contains("pro")) {
        return true;
    }
    // Custom OpenAI-compatible providers (NVIDIA NIM, SambaNova, Mistral, Cerebras, …) carry their
    // own free/paid flag in the registry — a standing free tier counts as genuinely free.
    if forge_config::custom_provider(provider).is_some_and(|cp| cp.free) {
        return true;
    }
    if cost > f64::EPSILON {
        return false;
    }
    match provider {
        // Paid gateways: only their explicit `:free`-suffixed variants are free.
        "openrouter" | "opencode_go" => id.contains(":free"),
        // Every other metered API provider (openai, xai, deepseek, anthropic, minimax, mimo, …) has
        // no standing free model tier — only temporary signup/trial credits — so an UNPRICED model
        // is paid-with-unknown-cost, NOT free. Reading "no price in our bundled table" as "free" was
        // the bug — it billed the user by routing to e.g. gpt-5-pro thinking it cost $0. A model
        // counts free only with positive evidence (a config price of 0, or a `:free` variant).
        _ => false,
    }
}

/// Whether a model id is a general chat/text-generation model the mesh can route an arbitrary turn
/// to. Provider model lists mix in *task-specific* endpoints that either break a general turn or
/// silently mangle it — image (`imagen`, `veo`, `lyria`, `*-image`, `nano-banana`), audio/TTS
/// (`*-tts`, `whisper`, `*-audio`), embeddings/rerankers, translation-only models (e.g.
/// `riva-translate`, which just translates/echoes any non-translation prompt), async deep-research,
/// `computer-use`/`robotics`, and moderation/guard models. Routing a general "reply with JSON …"
/// prompt to one of these produces garbage (the translation-model bug), so they are excluded from
/// the general routing ranking. They stay visible in `forge models`; a caller that specifically
/// wants one still pins it explicitly (which bypasses this general-pool filter).
/// Documented in docs/features/mesh-routing.md.
pub fn is_routable(id: &str) -> bool {
    let m = id.to_lowercase();
    const BLOCK: &[&str] = &[
        "imagen",
        "veo",
        "lyria",
        "nano-banana",
        "image",
        "-tts",
        "tts-",
        "whisper",
        "embedding",
        "-embed", // embed variants not caught by "embedding" (e.g. `nv-embedqa`, `text-embed`)
        "embed-", // ditto (e.g. Cohere `embed-english-v3.0`)
        "rerank", // reranker endpoints score pairs; they don't do chat completion
        "translate", // translation-only models (e.g. `riva-translate`) can't follow general prompts
        "deep-research",
        "computer-use",
        "robotics",
        "guard",
        "safeguard",
        "content-safety",
        "moderation",
        "-audio",
        "audio-",
        "-ocr",
        "sora",       // video generation
        "realtime",   // realtime voice/audio sessions, not a chat-completions model
        "transcribe", // speech-to-text
        "babbage",    // legacy base-completion models (not chat)
        "davinci",
    ];
    !BLOCK.iter().any(|b| m.contains(b))
}

/// Whether a model id is known to accept image input (vision). Providers don't expose this
/// uniformly, so — like [`is_routable`] and the capability priors in `capability.rs` — this is a
/// name-heuristic allowlist, not a live capability query. It exists to route AROUND a turn with
/// image attachments landing on a text-only model: that produces an immediate provider 404
/// ("No endpoints found that support image input"), not a slow/garbled reply like the
/// `is_routable` mismatches, so this is a positive allowlist rather than a block-list.
/// Documented in docs/features/mesh-routing.md.
pub fn supports_vision(id: &str) -> bool {
    let m = id.to_lowercase();
    const VISION_PATTERNS: &[&str] = &[
        // OpenAI: 4o, 4-turbo, 4.1, every gpt-5, and the o-series reasoning models all accept
        // image input; bare "gpt-4" (pre-turbo) and legacy completion models do not.
        "gpt-4o",
        "gpt-4-turbo",
        "gpt-4.1",
        "gpt-5",
        "o1",
        "o3",
        "o4",
        // Anthropic: every Claude 3+ family (3, 3.5, 3.7, 4, 4.5) is vision-capable — ids in this
        // catalog appear both dotted/dashed ("claude-3.5-sonnet", "claude-opus-4-8") and as a bare
        // family alias with no "claude-" prefix at all ("opus", "sonnet", "haiku", the claude-cli
        // bridge's default names) — those aliases only exist from Claude 3 onward. Pre-3 models
        // (`claude-2.1`, `claude-instant-1.2`) correctly fall through as non-vision.
        "claude-3",
        "claude-4",
        "opus",
        "sonnet",
        "haiku",
        // Google: every Gemini model (Pro/Flash/Flash-Lite) accepts image input.
        "gemini",
        // Meta: the vision-tuned Llama 3.2 sizes, and every Llama 4 model (natively multimodal).
        // Plain llama-3.2 text-only sizes (1b/3b, no "-vision" suffix) correctly fall through.
        "llama-3.2-11b-vision",
        "llama-3.2-90b-vision",
        "llama-4",
        // Mistral's vision-tuned line.
        "pixtral",
        // Qwen's vision-language line: the explicit "-vl-" tag, and the Qwen3-VL family.
        "-vl-",
        "qwen3-vl",
        // xAI: every Grok model accepts image input.
        "grok",
    ];
    VISION_PATTERNS.iter().any(|p| m.contains(p))
}

/// A model's cost class for routing: `0` genuinely free (local/free-tier), `1` subscription
/// ($0 marginal but burns the user's plan quota), `2` metered/paid. The mesh prefers low classes
/// for cheap tiers (preserve quota) and the subscription flagship for complex work.
/// Documented in docs/features/mesh-routing.md.
pub(crate) fn cost_class(id: &str, cost: f64) -> u8 {
    if is_subscription(id) {
        1
    } else if is_free(id, cost, false) {
        0
    } else {
        2
    }
}

/// How much a tier *wants* each cost class (added to the capability score). The policy:
/// - Trivial: prefer genuinely-free, so easy tasks don't burn subscription quota.
/// - Standard: subscription ≈ free, a slight subscription edge (use the good $0 models).
/// - Complex: prefer the subscription flagship (strongest reliable, $0 marginal); free as backup.
///
/// Documented in docs/features/mesh-routing.md; value asserted in sync by
/// `doc_sync::mesh_routing_doc_matches_live_constants`.
pub(crate) fn cost_pref(tier: TaskTier, class: u8) -> f64 {
    match (tier, class) {
        (TaskTier::Trivial, 0) => 1.0,
        (TaskTier::Trivial, 1) => 0.3,
        (TaskTier::Trivial, _) => -0.6,
        (TaskTier::Standard, 0) => 0.5,
        (TaskTier::Standard, 1) => 0.6,
        (TaskTier::Standard, _) => -0.4,
        (TaskTier::Complex, 0) => 0.4,
        (TaskTier::Complex, 1) => 0.8,
        (TaskTier::Complex, _) => 0.0,
    }
}

/// A mild, defensible provider prior (a tiebreak nudge, never a hard rule):
/// - code-heavy task → the coding-tuned flagships (codex/claude bridges + their APIs) get a small
///   lift over general models;
/// - trivial non-code → the fast cheap-bulk providers (groq/gemini) get a small lift.
///
/// Documented in docs/features/mesh-routing.md.
fn code_prior(provider: &str, code_heavy: bool, tier: TaskTier) -> f64 {
    if code_heavy {
        // `xai-oauth`/`xai` are deliberately excluded: there is no xai CLI bridge twin, so no
        // surface asymmetry to correct for here, and granting grok the coding-flagship bonus is
        // out of scope for the OAuth-supersedes-bridge work this arm was extended for (§ below).
        return match provider {
            "codex-cli" | "claude-cli" | "anthropic" | "openai" | "codex-oauth" => 0.3,
            _ => 0.0,
        };
    }
    if tier == TaskTier::Trivial && matches!(provider, "groq" | "gemini") {
        return 0.2;
    }
    0.0
}

/// Per-tier scaling for the subscription burn-weight penalty (Fix 2,
/// docs/design/subscription-efficiency-routing.md): cheap tiers avoid flagship burn hard, since a
/// trivial/standard task rarely needs the expensive sibling's extra capability; Complex still
/// wants the flagship but tie-breaks toward the cheaper sibling when capability is close.
/// Documented in docs/features/mesh-routing.md; value asserted in sync by `doc_sync::mesh_routing_doc_matches_live_constants`.
pub(crate) const BURN_K_TRIVIAL: f64 = 1.0;
pub(crate) const BURN_K_STANDARD: f64 = 0.7;
pub(crate) const BURN_K_COMPLEX: f64 = 0.15;

/// Documented in docs/features/mesh-routing.md.
fn burn_k(tier: TaskTier) -> f64 {
    match tier {
        TaskTier::Trivial => BURN_K_TRIVIAL,
        TaskTier::Standard => BURN_K_STANDARD,
        TaskTier::Complex => BURN_K_COMPLEX,
    }
}

/// Linear map from a subscription's live consumed-window fraction (`SubscriptionQuota::
/// effective_fraction_for`, pace-projected per #573) to a penalty multiplier: 0.5 when the window
/// is fresh, 2.0 when it is nearly spent. This composes with — and does not duplicate —
/// `conserve_decision`: that fires per-prompt to spread whole turns off the subscription onto a
/// free-frontier alternative; this only scales how hard a same-subscription tie-break (e.g. Sol vs
/// Luna) leans toward the cheaper sibling.
/// Documented in docs/features/mesh-routing.md.
fn pressure_multiplier(fraction: f64) -> f64 {
    (0.5 + 1.5 * fraction).clamp(0.5, 2.0)
}

/// Fix 2: the subscription plan-burn penalty for a subscription model, scaled by tier urgency and
/// live quota pressure. `ln(weight)` keeps a 5x burn from swamping a genuine capability gap (ln 5
/// ≈ 1.61) and makes a weight of 1.0 (cheapest sibling, or any unknown model) contribute exactly
/// zero — so behaviour is unchanged for every model with no burn-weight entry.
/// Documented in docs/features/mesh-routing.md.
fn subscription_burn_penalty(
    id: &str,
    tier: TaskTier,
    quota: &forge_types::SubscriptionQuota,
    overrides: &HashMap<String, f64>,
) -> f64 {
    let weight = crate::capability::subscription_burn_weight(id, overrides);
    if weight <= 1.0 {
        return 0.0;
    }
    let fraction = quota.effective_fraction_for(provider_of(id));
    burn_k(tier) * weight.ln() * pressure_multiplier(fraction)
}

/// Provider pairs where a native OAuth surface dispatches the SAME model catalog as a CLI bridge:
/// `(oauth, bridge)`. When the catalog contains `<oauth>::X`, the twin `<bridge>::X` is demoted by
/// [`BRIDGE_SUPERSEDE_PENALTY`] — see [`superseded_bridge_ids`]. Native OAuth runs Forge's own
/// harness instead of shelling out to the CLI's agent loop, so once it can dispatch a model it is
/// structurally the better surface for that model; the bridge stays reachable as failover.
/// Documented in docs/features/mesh-routing.md.
pub(crate) const OAUTH_SUPERSEDES: &[(&str, &str)] = &[("codex-oauth", "codex-cli")];

/// Fixed score penalty applied to a bridge model id when its OAuth twin is present in the catalog
/// ([`OAUTH_SUPERSEDES`], via [`superseded_bridge_ids`]). The twins otherwise score identically —
/// same bare model name → same capability, burn weight, and cost class, `code_prior` tied (Fix 2
/// above), quota shared at the store layer — so a flat penalty this large guarantees the OAuth
/// twin outranks the bridge twin at EVERY tier, while leaving the bridge in the ranked chain as a
/// natural failover if OAuth errors at dispatch. This implements "prefer OAuth, bridge only when
/// OAuth is unavailable" via failover ordering rather than removing the bridge outright.
/// Documented in docs/features/mesh-routing.md; value asserted in sync by
/// `doc_sync::mesh_routing_doc_matches_live_constants`.
pub(crate) const BRIDGE_SUPERSEDE_PENALTY: f64 = 1.0;

/// The set of full `bridge::model` ids in `models` whose OAuth twin (per [`OAUTH_SUPERSEDES`]) is
/// ALSO present in `models` — computed once per ranking pass (`ranked_seeded`/`ranked_rows`) so
/// the per-candidate demotion is an O(1) set lookup rather than an O(n²) per-candidate rescan of
/// the catalog. Catalog presence of an oauth model implies a live OAuth session (discovery is
/// gated on `has_codex_oauth_session()` in forge-cli), so no session-probing happens here.
/// Documented in docs/features/mesh-routing.md.
fn superseded_bridge_ids(models: &[String]) -> std::collections::HashSet<String> {
    let mut oauth_bare_names: HashMap<&str, std::collections::HashSet<&str>> = HashMap::new();
    for m in models {
        let provider = provider_of(m);
        if OAUTH_SUPERSEDES.iter().any(|(oauth, _)| *oauth == provider) {
            if let Some((_, bare)) = m.split_once("::") {
                oauth_bare_names.entry(provider).or_default().insert(bare);
            }
        }
    }
    let mut superseded = std::collections::HashSet::new();
    for m in models {
        let provider = provider_of(m);
        let Some((oauth, _)) = OAUTH_SUPERSEDES.iter().find(|(_, b)| *b == provider) else {
            continue;
        };
        if let Some((_, bare)) = m.split_once("::") {
            if oauth_bare_names
                .get(oauth)
                .is_some_and(|s| s.contains(bare))
            {
                superseded.insert(m.clone());
            }
        }
    }
    superseded
}

/// The full routing score for one model: capability fit + cost-class preference + the mild prior,
/// minus the subscription burn-weight penalty (Fix 2), a quota status penalty so a near-limit
/// subscription drops below its alternatives (L3), and the OAuth-supersedes-bridge penalty
/// ([`BRIDGE_SUPERSEDE_PENALTY`]) when `superseded` is set. The penalties are applied in the SCORE
/// (not just a post-sort) so non-subscription alternatives make it into the truncated shortlist —
/// otherwise the top picks are all the (pressured) subscription.
/// Documented in docs/features/mesh-routing.md.
#[allow(clippy::too_many_arguments)]
fn route_score(
    id: &str,
    tier: TaskTier,
    cost: f64,
    code_heavy: bool,
    quota: &forge_types::SubscriptionQuota,
    bench: Option<&BenchmarkScores>,
    burn_weight_overrides: &HashMap<String, f64>,
    superseded: bool,
) -> f64 {
    let mut base = capability_score_b(id, tier, code_heavy, bench)
        + cost_pref(tier, cost_class(id, cost))
        + code_prior(provider_of(id), code_heavy, tier)
        - crate::capability::tool_reliability_penalty(id);
    if superseded {
        base -= BRIDGE_SUPERSEDE_PENALTY;
    }
    if is_subscription(id) {
        base -= subscription_burn_penalty(id, tier, quota, burn_weight_overrides);
        match quota.status_for(provider_of(id)) {
            forge_types::QuotaStatus::Exhausted => return base - 100.0, // effectively last
            forge_types::QuotaStatus::Warning => return base - 5.0,     // below any plausible alt
            forge_types::QuotaStatus::Ok => {}
        }
    }
    base
}

/// Soft demotion applied to subscription models when this prompt is chosen for conservation.
/// Large enough to drop an `Ok` subscription below the best free-frontier alternative, small
/// enough that the subscription stays in the shortlist as a fallback if every alternative fails.
/// Documented in docs/features/mesh-routing.md; value asserted in sync by `doc_sync::mesh_routing_doc_matches_live_constants`.
pub(crate) const CONSERVE_PENALTY: f64 = 4.0;

/// How freely a plan may be spent: a bigger plan has more headroom, so it is conserved *less*
/// (lower factor → lower spread probability). Unknown/unset plans stay neutral (1.0) — we don't
/// over-conserve a plan the user never told us about.
/// Documented in docs/features/mesh-routing.md.
fn plan_factor(slug: &str) -> f64 {
    let s = slug.to_lowercase();
    if s.contains("20x") {
        0.8
    } else if s.contains("max") || s.contains("pro") {
        0.85
    } else {
        1.0 // plus / team / unknown
    }
}

/// Probability that this prompt routes OFF the subscriptions onto a free-frontier model, given the
/// tier, how full the strictest window is (`fraction`), the plan headroom, and code-heaviness.
/// Trivial always spreads (subs are never worth spending on it); Standard mostly spreads; Complex
/// spreads a minority while fresh and ramps to ~1.0 as the window approaches the 80% Warning line.
///
/// `fraction` is a plain 0.0–1.0 input to this pure function — it doesn't know where the number
/// came from. Real routing (`conserve_decision`) and the `/mesh` inspector (`spread_probability`)
/// both pass `SubscriptionQuota::effective_fraction_for`, which is pace-projected: a window
/// burning fast early on is passed in as if it were already at its projected reset-time usage,
/// so the ramp above starts ahead of the overrun instead of reacting to one already at Warning.
/// Documented in docs/features/mesh-routing.md.
fn conserve_probability(tier: TaskTier, fraction: f64, plan: &str, code_heavy: bool) -> f64 {
    if tier == TaskTier::Trivial {
        return 1.0;
    }
    let base = match tier {
        TaskTier::Trivial => unreachable!(),
        TaskTier::Standard => 0.65,
        TaskTier::Complex if code_heavy => 0.15, // code-heavy complex: subscriptions earn their keep
        TaskTier::Complex => 0.30,
    };
    let ramp = (fraction / 0.80).clamp(0.0, 1.0) * (1.0 - base);
    ((base + ramp) * plan_factor(plan)).clamp(0.0, 1.0)
}

/// Whether a model qualifies as a capable alternative for `tier`, bench-aware. For Complex the
/// bar is frontier (bench ≥ `FRONTIER_BENCH_THRESHOLD`, else name-heuristic class 3); for
/// Standard it's capable mid (bench ≥ `CAPABLE_BENCH_THRESHOLD`, else class 2). This prevents
/// conservation from firing based on a nominally-large but measurably-weak old model (e.g. a
/// Hermes 405B at score 9.0 would pass the old name check but fails the bench threshold).
/// Documented in docs/features/mesh-routing.md.
fn is_capable_alternative(id: &str, tier: TaskTier, bench: Option<&BenchmarkScores>) -> bool {
    match tier {
        TaskTier::Complex => is_frontier_b(id, bench),
        TaskTier::Standard => match bench.and_then(|b| b.score_for(id)) {
            Some(s) => s.intelligence >= CAPABLE_BENCH_THRESHOLD,
            None => crate::capability::quality_class(id) >= 2,
        },
        TaskTier::Trivial => true,
    }
}

/// Whether a genuine non-subscription alternative of the right calibre exists for `tier` — a
/// guard so conservation never drops a hard task onto a weak model when the only capable option
/// IS the subscription. Complex needs a frontier alternative; Standard a capable (mid+) one.
/// Documented in docs/features/mesh-routing.md.
fn has_nonsub_alternative(
    models: &[String],
    tier: TaskTier,
    bench: Option<&BenchmarkScores>,
) -> bool {
    models
        .iter()
        .any(|m| !is_subscription(m) && is_routable(m) && is_capable_alternative(m, tier, bench))
}

/// One model's scored row for the routing inspector: the score broken out so a human can see WHY
/// it ranked where it did. `rotation`/`fine` are the tiebreak keys (kept for a stable, explainable
/// sort), not shown directly.
#[derive(Debug, Clone)]
pub struct ScoreRow {
    pub model: String,
    pub provider: String,
    /// Pure capability fit for the tier (speed/quality blend).
    pub capability: f64,
    /// 0 = free, 1 = subscription, 2 = paid.
    pub cost_class: u8,
    /// Conservation demotion applied to this model for this prompt (0.0 if none).
    pub conserve_penalty: f64,
    /// Final ranking score (capability + cost/code priors − quota − conservation).
    pub final_score: f64,
    pub subscription: bool,
    pub frontier: bool,
    rotation: u64,
    weight: u8,
    fine: f64,
    pub bench_score: Option<f64>,
    pub cost: f64,
    pub speed: u8,
}

/// The full, inspectable conservation decision for a prompt (the data the `/mesh` inspector and
/// `forge mesh explain` surface). `fired` is what routing acts on.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConserveDecision {
    /// Conservation enabled in config.
    pub enabled: bool,
    /// A subscription is present AND a capable non-subscription alternative exists for the tier.
    pub eligible: bool,
    /// The spread probability used (max conservation pull across the present subscriptions).
    pub probability: f64,
    /// The deterministic per-prompt draw in [0,1).
    pub roll: f64,
    /// `roll < probability` — this prompt spreads off the subscriptions.
    pub fired: bool,
}

/// Decide — deterministically for this prompt — whether to spread off the subscriptions. Takes the
/// strongest conservation pull across the present subscription providers (protect whichever is most
/// pressured / smallest-plan), then draws a stable per-prompt value against it. Does not fire when
/// disabled, when there are no subscriptions, or when no capable alternative exists.
///
/// The fraction driving that pull is pace-projected (`SubscriptionQuota::effective_fraction_for`),
/// not just the point-in-time fraction — a window burning fast early on is treated as if it's
/// already at its projected reset-time usage, so spreading ramps up ahead of the overrun.
pub(crate) fn conserve_decision(
    models: &[String],
    tier: TaskTier,
    code_heavy: bool,
    seed: u64,
    quota: &forge_types::SubscriptionQuota,
    bench: Option<&BenchmarkScores>,
) -> ConserveDecision {
    let mut d = ConserveDecision {
        enabled: quota.conserve_enabled(),
        ..Default::default()
    };
    if !d.enabled {
        return d;
    }
    let mut sub_providers: Vec<&str> = models
        .iter()
        .filter(|m| is_subscription(m))
        .map(|m| provider_of(m))
        .collect();
    sub_providers.sort_unstable();
    sub_providers.dedup();
    d.eligible = !sub_providers.is_empty() && has_nonsub_alternative(models, tier, bench);
    if !d.eligible {
        return d;
    }
    d.probability = sub_providers
        .iter()
        .map(|prov| {
            conserve_probability(
                tier,
                quota.effective_fraction_for(prov),
                quota.plan_for(prov),
                code_heavy,
            )
        })
        .fold(0.0_f64, f64::max);
    d.roll = (stable_hash(&format!("{seed}:conserve")) % 10_000) as f64 / 10_000.0;
    d.fired = d.roll < d.probability;
    d
}

pub(crate) fn provider_conservation_fired(
    provider: &str,
    tier: TaskTier,
    code_heavy: bool,
    decision: ConserveDecision,
    quota: &forge_types::SubscriptionQuota,
) -> bool {
    decision.eligible
        && decision.roll
            < conserve_probability(
                tier,
                quota.effective_fraction_for(provider),
                quota.plan_for(provider),
                code_heavy,
            )
}

/// A per-prompt provider ordering key: hashing `seed:provider` means different prompts rotate
/// which provider wins a genuine score tie, so a workload spreads across equally-good providers
/// (claude ↔ codex) instead of always picking the alphabetically-first one — while staying fully
/// deterministic for a given prompt.
/// Documented in docs/features/mesh-routing.md.
fn provider_rotation(provider: &str, seed: u64) -> u64 {
    stable_hash(&format!("{seed}:{provider}"))
}

/// How heavy a model is on its subscription (1 = light, 3 = the heavy flagship). When two models
/// tie on score — e.g. `claude-cli::opus` and `claude-cli::sonnet` both rank q3 for a complex task
/// — the mesh should spend the LIGHTER one to conserve the flagship's quota. This distinguishes
/// siblings the capability prior treats as equal (opus and sonnet are both "frontier"). It only
/// matters as a tiebreak: a genuinely weaker model (mini/haiku) already scores lower and never
/// enters the tie. Family-agnostic via name markers, so new bridges order sensibly too.
/// Documented in docs/features/mesh-routing.md.
fn model_weight(id: &str) -> u8 {
    let m = id.to_lowercase();
    if m.contains("opus") || m.contains("-pro") || m.contains("-max") || m.contains("ultra") {
        3
    } else if m.contains("haiku")
        || m.contains("-mini")
        || m.contains("nano")
        || m.contains("flash")
        || m.contains("-lite")
        || m.contains("instant")
    {
        1
    } else {
        2 // sonnet, gpt-5.x, and other mid-tier flagships
    }
}

/// A fine within-family capability key (the first version number in the id: `gpt-5.5`→5.5,
/// `claude-opus-4-8`→4.8, `gpt-4o-mini`→4.0). Used as a LATE tiebreak — after the provider
/// rotation — so it only orders models of the *same* provider/class: never pick `gpt-5.2` over
/// `gpt-5.5` when both are the same $0 subscription. It never competes across providers (the
/// rotation already separated those), so a higher raw number can't make one provider always win.
/// Documented in docs/features/mesh-routing.md.
fn fine_capability(id: &str) -> f64 {
    let bytes = id.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_digit() {
        i += 1;
    }
    // Cap digits actually accumulated (not just `i`'s advance) so a long digit run in a model id
    // (embedded hash, snowflake id, timestamp, ...) can't overflow the u32 accumulator; 9 digits
    // safely fits (max 999,999,999 < u32::MAX) and is far beyond any real version number.
    const MAX_DIGITS: u32 = 9;
    let mut major: u32 = 0;
    let mut major_digits = 0u32;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        if major_digits < MAX_DIGITS {
            major = major * 10 + (bytes[i] - b'0') as u32;
            major_digits += 1;
        }
        i += 1;
    }
    // An immediately-following `.` or `-` then digits is the minor version (`5.4`, `4-8`).
    let mut frac = 0.0;
    if i < bytes.len()
        && (bytes[i] == b'.' || bytes[i] == b'-')
        && i + 1 < bytes.len()
        && bytes[i + 1].is_ascii_digit()
    {
        i += 1;
        let (mut minor, mut digits) = (0u32, 0i32);
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            if (digits as u32) < MAX_DIGITS {
                minor = minor * 10 + (bytes[i] - b'0') as u32;
                digits += 1;
            }
            i += 1;
        }
        frac = minor as f64 / 10f64.powi(digits);
    }
    major as f64 + frac
}

/// A small deterministic FNV-1a hash (no external deps); used for the seed and provider rotation.
/// Documented in docs/features/mesh-routing.md.
pub fn stable_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A discovered model classified for display (the `/models` browser + `forge models`). Pure view
/// data derived from the id + pricing — no health/network state (the caller overlays "benched").
#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    /// Full `provider::model` id.
    pub id: String,
    /// Provider prefix (`anthropic`, `groq`, `claude-cli`, …).
    pub provider: String,
    /// The model name after `::` (empty for a bare bridge id, meaning its default model).
    pub name: String,
    /// Frontier-class by the capability prior (`opus`/`gpt-5`/`-70b`/…).
    pub frontier: bool,
    /// Genuinely free (local/ollama, free-tier APIs, or an OpenRouter `:free` variant) — see
    /// [`is_free`]. NOT merely "unpriced": a paid OpenRouter model is `paid`, not `free`.
    pub free: bool,
    /// Metered: either a known price > 0, or a gateway model with no free evidence (e.g. a paid
    /// OpenRouter model we hold no price for). Mutually exclusive with `free` and `subscription`.
    pub paid: bool,
    /// A $0-marginal subscription CLI bridge (claude-cli/codex-cli).
    pub subscription: bool,
    /// Estimated USD for a nominal turn (0 = subscription/unpriced; a paid model may still be 0
    /// here when we have no per-model rate for it, e.g. an OpenRouter gateway model).
    pub cost: f64,
}

impl ModelInfo {
    fn classify(id: &str, pricing: &Pricing, bench: Option<&BenchmarkScores>) -> Self {
        let subscription = is_subscription(id);
        let cost = pricing.estimated_cost(id);
        let free = is_free(id, cost, subscription);
        Self {
            id: id.to_string(),
            provider: provider_of(id).to_string(),
            name: id
                .split_once("::")
                .map(|(_, n)| n)
                .unwrap_or("")
                .to_string(),
            frontier: is_frontier_b(id, bench),
            free,
            paid: !subscription && !free,
            subscription,
            cost,
        }
    }
}

/// Aggregate counts across the whole catalog, for the overview header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogStats {
    pub total: usize,
    pub providers: usize,
    pub frontier: usize,
    pub free: usize,
    pub subscription: usize,
    pub paid: usize,
}

/// One provider's discovered models, frontier-first then alphabetical.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderGroup {
    pub provider: String,
    pub models: Vec<ModelInfo>,
}

impl ProviderGroup {
    pub fn total(&self) -> usize {
        self.models.len()
    }
    pub fn frontier(&self) -> usize {
        self.models.iter().filter(|m| m.frontier).count()
    }
    pub fn free(&self) -> usize {
        self.models.iter().filter(|m| m.free).count()
    }
    pub fn paid(&self) -> usize {
        self.models.iter().filter(|m| m.paid).count()
    }
}

impl ModelCatalog {
    pub fn new(mut models: Vec<String>) -> Self {
        // A bare bridge id (`claude-cli::`) is a valid manual *pin* for the CLI's own default
        // model, but as a catalog entry it's an empty-named row that can never match a benchmark
        // or context window — every catalog entry must name a model. Discovery no longer emits
        // them; this also keeps ids cached before that fix (or hand-written) out.
        models.retain(|m| !m.ends_with("::"));
        Self {
            models,
            bench: None,
            burn_weights: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    pub fn models(&self) -> &[String] {
        &self.models
    }

    /// Attach measured benchmark scores (ADR-0011) so ranking uses real performance data. A `None`
    /// or empty set is a no-op — ranking stays on the family heuristic.
    pub fn with_benchmarks(mut self, bench: Option<BenchmarkScores>) -> Self {
        self.bench = bench.filter(|b| !b.is_empty());
        self
    }

    /// Attach `mesh.burn_weights` config overrides (Fix 1,
    /// docs/design/subscription-efficiency-routing.md), keyed by bare model name. An empty map is
    /// a no-op — `route_score` falls back to the bundled `subscription_burn_weight` table.
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_burn_weights(mut self, overrides: HashMap<String, f64>) -> Self {
        self.burn_weights = overrides;
        self
    }

    /// How many of the catalog's models have a benchmark score (for `forge benchmarks` coverage).
    pub fn benchmark_coverage(&self) -> (usize, usize) {
        match &self.bench {
            Some(b) => (
                self.models
                    .iter()
                    .filter(|m| b.score_for(m).is_some())
                    .count(),
                self.models.len(),
            ),
            None => (0, self.models.len()),
        }
    }

    /// The discovered models ranked best-first for `tier` (display / non-prompt callers): the
    /// cost-tiered routing score with a neutral context (not code-heavy, fixed seed). The live
    /// router uses [`ranked_seeded`](Self::ranked_seeded) so genuine ties spread across providers
    /// per prompt instead of always picking the alphabetically-first one.
    pub fn ranked_for(&self, tier: TaskTier, pricing: &Pricing, top: usize) -> Vec<String> {
        self.ranked_seeded(
            tier,
            pricing,
            top,
            false,
            0,
            &forge_types::SubscriptionQuota::default(),
            None,
        )
    }

    /// Prompt-aware ranking: cost-tiered capability score, with genuine ties broken by a
    /// per-prompt `seed` rotation across providers (fair spread) then id (stable). `code_heavy`
    /// applies the mild coding-provider prior. The single place the routing policy lives.
    #[allow(clippy::too_many_arguments)]
    pub fn ranked_seeded(
        &self,
        tier: TaskTier,
        pricing: &Pricing,
        top: usize,
        code_heavy: bool,
        seed: u64,
        quota: &forge_types::SubscriptionQuota,
        effort: Option<EffortLevel>,
    ) -> Vec<String> {
        // Proactive subscription conservation: for this prompt, decide whether to spread off the
        // subscription bridges onto a free-frontier model (so a complex/standard-heavy workload
        // doesn't exhaust the plan). When it fires, subscriptions take a soft penalty so the best
        // alternative leads while the subscription stays available as a fallback.
        let conservation = conserve_decision(
            &self.models,
            tier,
            code_heavy,
            seed,
            quota,
            self.bench.as_ref(),
        );
        let superseded = superseded_bridge_ids(&self.models);

        struct ScoredModel<'a> {
            id: &'a String,
            route_score: f64,
            cost_class: u8,
            provider_rotation: u64,
            model_weight: u8,
            fine_capability: f64,
            bench_score: Option<f64>,
            cost: f64,
            speed: u8,
            conserved: bool,
        }

        let mut scored: Vec<ScoredModel> = self
            .models
            .iter()
            .filter(|m| is_routable(m))
            .map(|m| {
                let cost = pricing.estimated_cost(m);
                let mut score = route_score(
                    m,
                    tier,
                    cost,
                    code_heavy,
                    quota,
                    self.bench.as_ref(),
                    &self.burn_weights,
                    superseded.contains(m),
                );
                let conserved = is_subscription(m)
                    && provider_conservation_fired(
                        provider_of(m),
                        tier,
                        code_heavy,
                        conservation,
                        quota,
                    );
                if conserved {
                    score -= CONSERVE_PENALTY;
                }
                let bench_score = self.bench.as_ref().and_then(|b| b.score_for(m)).map(|s| {
                    if code_heavy {
                        s.coding
                    } else {
                        s.intelligence
                    }
                });
                ScoredModel {
                    id: m,
                    route_score: score,
                    cost_class: cost_class(m, cost),
                    provider_rotation: provider_rotation(provider_of(m), seed),
                    model_weight: model_weight(m),
                    fine_capability: fine_capability(m),
                    bench_score,
                    cost,
                    speed: crate::capability::speed_class(m),
                    conserved,
                }
            })
            .collect();

        let active_effort = effort.unwrap_or(EffortLevel::Medium);
        scored.sort_by(|a, b| match active_effort {
            EffortLevel::High | EffortLevel::XHigh | EffortLevel::WhiteHot => {
                // Compare by integer bench-score BAND first, not a pairwise "gap ≥ 1.0"
                // early-return: the pairwise rule was intransitive (a within 1 of b, b within
                // 1 of c, but a and c more than 1 apart → contradictory orderings) and panicked
                // Rust's sort with "comparison function does not correctly implement a total
                // order" the first time a white-hot turn ranked a full catalog. Unbenched
                // models (no band) sort below benched ones — at high effort you asked for
                // proven quality.
                a.conserved
                    .cmp(&b.conserved)
                    .then_with(|| bench_band(b.bench_score).cmp(&bench_band(a.bench_score)))
                    .then_with(|| b.route_score.total_cmp(&a.route_score))
                    .then_with(|| a.cost_class.cmp(&b.cost_class))
                    .then_with(|| a.provider_rotation.cmp(&b.provider_rotation))
                    .then_with(|| a.model_weight.cmp(&b.model_weight))
                    .then_with(|| b.fine_capability.total_cmp(&a.fine_capability))
                    .then_with(|| a.id.cmp(b.id))
            }
            EffortLevel::Low => {
                // Pure cheapest-first. The old bench early-return here had the same
                // intransitivity as the high arm, and bench-gating never matched low effort's
                // intent ("cheapest acceptable") anyway.
                a.cost_class
                    .cmp(&b.cost_class)
                    .then_with(|| a.cost.total_cmp(&b.cost))
                    .then_with(|| b.speed.cmp(&a.speed))
                    .then_with(|| {
                        b.route_score
                            .total_cmp(&a.route_score)
                            .then_with(|| a.provider_rotation.cmp(&b.provider_rotation))
                            .then_with(|| a.model_weight.cmp(&b.model_weight))
                            .then_with(|| b.fine_capability.total_cmp(&a.fine_capability))
                            .then_with(|| a.id.cmp(b.id))
                    })
            }
            EffortLevel::Medium => b
                .route_score
                .total_cmp(&a.route_score)
                .then_with(|| a.cost_class.cmp(&b.cost_class))
                .then_with(|| a.provider_rotation.cmp(&b.provider_rotation))
                .then_with(|| a.model_weight.cmp(&b.model_weight))
                .then_with(|| b.fine_capability.total_cmp(&a.fine_capability))
                .then_with(|| a.id.cmp(b.id)),
        });

        scored.into_iter().take(top).map(|s| s.id.clone()).collect()
    }

    /// The full ranked candidate table for a tier with each model's score broken out — the data
    /// behind `/mesh` and `forge mesh explain`. Same ordering as [`ranked_seeded`](Self::ranked_seeded),
    /// including the banded bench comparison (see [`bench_band`]).
    /// but every routable model is returned (not truncated) with its capability, cost class, the
    /// conservation penalty applied (if any), and the final score. Pure (no health/usability — the
    /// router overlays that).
    pub fn ranked_rows(
        &self,
        tier: TaskTier,
        pricing: &Pricing,
        code_heavy: bool,
        seed: u64,
        quota: &forge_types::SubscriptionQuota,
        effort: Option<EffortLevel>,
    ) -> (ConserveDecision, Vec<ScoreRow>) {
        let decision = conserve_decision(
            &self.models,
            tier,
            code_heavy,
            seed,
            quota,
            self.bench.as_ref(),
        );
        let superseded = superseded_bridge_ids(&self.models);
        let mut rows: Vec<ScoreRow> = self
            .models
            .iter()
            .filter(|m| is_routable(m))
            .map(|m| {
                let cost = pricing.estimated_cost(m);
                let base = route_score(
                    m,
                    tier,
                    cost,
                    code_heavy,
                    quota,
                    self.bench.as_ref(),
                    &self.burn_weights,
                    superseded.contains(m),
                );
                let sub = is_subscription(m);
                let penalty = if sub
                    && provider_conservation_fired(
                        provider_of(m),
                        tier,
                        code_heavy,
                        decision,
                        quota,
                    ) {
                    CONSERVE_PENALTY
                } else {
                    0.0
                };
                let bench_score = self.bench.as_ref().and_then(|b| b.score_for(m)).map(|s| {
                    if code_heavy {
                        s.coding
                    } else {
                        s.intelligence
                    }
                });
                ScoreRow {
                    model: m.clone(),
                    provider: provider_of(m).to_string(),
                    capability: capability_score_b(m, tier, code_heavy, self.bench.as_ref()),
                    cost_class: cost_class(m, cost),
                    conserve_penalty: penalty,
                    final_score: base - penalty,
                    subscription: sub,
                    frontier: is_frontier_b(m, self.bench.as_ref()),
                    rotation: provider_rotation(provider_of(m), seed),
                    weight: model_weight(m),
                    fine: fine_capability(m),
                    bench_score,
                    cost,
                    speed: crate::capability::speed_class(m),
                }
            })
            .collect();

        let active_effort = effort.unwrap_or(EffortLevel::Medium);
        rows.sort_by(|a, b| match active_effort {
            EffortLevel::High | EffortLevel::XHigh | EffortLevel::WhiteHot => {
                // Banded, not pairwise — same total-order fix as `ranked_seeded` (see the
                // comment there; the pairwise gap rule panicked the sort on real catalogs).
                (a.conserve_penalty > 0.0)
                    .cmp(&(b.conserve_penalty > 0.0))
                    .then_with(|| bench_band(b.bench_score).cmp(&bench_band(a.bench_score)))
                    .then_with(|| b.final_score.total_cmp(&a.final_score))
                    .then_with(|| a.cost_class.cmp(&b.cost_class))
                    .then_with(|| a.rotation.cmp(&b.rotation))
                    .then_with(|| a.weight.cmp(&b.weight))
                    .then_with(|| b.fine.total_cmp(&a.fine))
                    .then_with(|| a.model.cmp(&b.model))
            }
            EffortLevel::Low => {
                // Pure cheapest-first — same rationale as `ranked_seeded`'s Low arm.
                a.cost_class
                    .cmp(&b.cost_class)
                    .then_with(|| a.cost.total_cmp(&b.cost))
                    .then_with(|| b.speed.cmp(&a.speed))
                    .then_with(|| {
                        b.final_score
                            .total_cmp(&a.final_score)
                            .then_with(|| a.rotation.cmp(&b.rotation))
                            .then_with(|| a.weight.cmp(&b.weight))
                            .then_with(|| b.fine.total_cmp(&a.fine))
                            .then_with(|| a.model.cmp(&b.model))
                    })
            }
            EffortLevel::Medium => b
                .final_score
                .total_cmp(&a.final_score)
                .then_with(|| a.cost_class.cmp(&b.cost_class))
                .then_with(|| a.rotation.cmp(&b.rotation))
                .then_with(|| a.weight.cmp(&b.weight))
                .then_with(|| b.fine.total_cmp(&a.fine))
                .then_with(|| a.model.cmp(&b.model)),
        });
        (decision, rows)
    }

    /// The per-provider spread probability for a tier (the `/mesh` quota view) — how likely a task
    /// of this tier routes off that subscription given its window fraction + plan.
    pub fn spread_probability(tier: TaskTier, fraction: f64, plan: &str, code_heavy: bool) -> f64 {
        conserve_probability(tier, fraction, plan, code_heavy)
    }

    /// Every discovered model classified for display (id order preserved).
    pub fn infos(&self, pricing: &Pricing) -> Vec<ModelInfo> {
        self.models
            .iter()
            .map(|m| ModelInfo::classify(m, pricing, self.bench.as_ref()))
            .collect()
    }

    /// Headline counts across the catalog (total / providers / frontier / free / subscription /
    /// paid) for the overview.
    pub fn stats(&self, pricing: &Pricing) -> CatalogStats {
        let infos = self.infos(pricing);
        let mut providers: Vec<&str> = infos.iter().map(|m| m.provider.as_str()).collect();
        providers.sort_unstable();
        providers.dedup();
        CatalogStats {
            total: infos.len(),
            providers: providers.len(),
            frontier: infos.iter().filter(|m| m.frontier).count(),
            free: infos.iter().filter(|m| m.free).count(),
            subscription: infos.iter().filter(|m| m.subscription).count(),
            paid: infos.iter().filter(|m| m.paid).count(),
        }
    }

    /// Models grouped by provider for the drill-in browser. Providers are ordered by model count
    /// (richest first), ties by name; within a group, frontier models lead, then alphabetical.
    pub fn by_provider(&self, pricing: &Pricing) -> Vec<ProviderGroup> {
        let mut groups: Vec<ProviderGroup> = Vec::new();
        // Skip bare bridge ids (`claude-cli::`, `codex-cli::`) — they are valid routing
        // aliases for the CLI's own default model but show up as confusingly empty rows.
        for info in self
            .infos(pricing)
            .into_iter()
            .filter(|m| !m.name.is_empty())
        {
            match groups.iter_mut().find(|g| g.provider == info.provider) {
                Some(g) => g.models.push(info),
                None => groups.push(ProviderGroup {
                    provider: info.provider.clone(),
                    models: vec![info],
                }),
            }
        }
        for g in &mut groups {
            g.models.sort_by(|a, b| {
                b.frontier
                    .cmp(&a.frontier)
                    .then_with(|| a.name.cmp(&b.name))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        groups.sort_by(|a, b| {
            b.models
                .len()
                .cmp(&a.models.len())
                .then_with(|| a.provider.cmp(&b.provider))
        });
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ModelCatalog {
        ModelCatalog::new(vec![
            "groq::llama-3.1-8b-instant".into(),
            "groq::llama-3.3-70b-versatile".into(),
            "anthropic::claude-opus-4-8".into(),
            "ollama::llama3.2".into(),
        ])
    }

    #[test]
    fn xai_oauth_is_subscription_not_free() {
        assert!(is_subscription("xai-oauth::grok-4"));
        assert!(!is_free("xai-oauth::grok-4", 0.0, true));
        assert!(is_subscription("codex-oauth::gpt-5.5"));
        assert!(!is_free("codex-oauth::gpt-5.5", 0.0, true));
    }

    #[test]
    fn ranks_a_small_fast_model_first_for_trivial() {
        let r = catalog().ranked_for(TaskTier::Trivial, &Pricing::default(), 2);
        assert_eq!(r.first().unwrap(), "groq::llama-3.1-8b-instant");
    }

    #[test]
    fn benchmark_scores_override_the_name_heuristic() {
        use crate::bench::BenchmarkScores;
        // By name heuristic, gpt-5.2 is frontier (q3) and "mystery-x" is unknown (q2) → gpt wins.
        let cat = ModelCatalog::new(vec![
            "openai::gpt-5.2".into(),
            "openrouter::acme/mystery-x".into(),
        ]);
        let plain = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 2);
        assert_eq!(
            plain[0], "openai::gpt-5.2",
            "heuristic: named frontier leads"
        );

        // Now attach REAL scores where mystery-x measures far higher than gpt-5.2 → it must lead.
        let mut b = BenchmarkScores::new();
        b.insert("gpt-5.2", 35.0, 30.0);
        b.insert("acme mystery-x", 68.0, 66.0);
        let cat = cat.with_benchmarks(Some(b));
        let ranked = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 2);
        assert_eq!(
            ranked[0], "openrouter::acme/mystery-x",
            "benchmark data must override the name heuristic: {ranked:?}"
        );
        let (covered, total) = cat.benchmark_coverage();
        assert_eq!((covered, total), (2, 2));
    }

    #[test]
    fn tool_unreliable_gemini_flash_ranks_below_a_comparable_tool_reliable_model() {
        use crate::bench::BenchmarkScores;
        // Equal top benchmark scores: without the tool-reliability penalty these would tie. The
        // Gemini *flash* model leaks tool calls as text, so it must rank BELOW the tool-reliable
        // peer for a (tool-driven) Complex task — while staying in the chain as a fallback.
        let cat = ModelCatalog::new(vec![
            "openrouter::google/gemini-3.5-flash".into(),
            "openrouter::deepseek/deepseek-v4".into(),
        ]);
        let mut b = BenchmarkScores::new();
        b.insert("google gemini-3.5-flash", 60.0, 58.0);
        b.insert("deepseek deepseek-v4", 60.0, 58.0);
        let cat = cat.with_benchmarks(Some(b));
        let r = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 2);
        assert_eq!(
            r[0], "openrouter::deepseek/deepseek-v4",
            "tool-reliable peer outranks tool-leaky gemini-flash at equal bench: {r:?}"
        );
        assert!(
            r.contains(&"openrouter::google/gemini-3.5-flash".to_string()),
            "gemini-flash stays in the chain as a fallback: {r:?}"
        );
    }

    #[test]
    fn ranks_a_frontier_model_first_for_complex() {
        let r = catalog().ranked_for(TaskTier::Complex, &Pricing::default(), 3);
        // opus (paid, q3) vs groq-70b (free, q3): free bonus tips it to the free 70b.
        assert!(
            r.first().unwrap().contains("70b") || r.first().unwrap().contains("opus"),
            "a frontier-class model leads: {r:?}"
        );
        assert!(
            !r.first().unwrap().contains("8b"),
            "not the tiny model: {r:?}"
        );
    }

    #[test]
    fn non_chat_models_are_excluded_from_routing() {
        // Provider lists mix in image/video/tts/embedding/deep-research endpoints. The mesh must
        // never route a turn to one — for trivial that was picking a slow deep-research model.
        assert!(!is_routable("gemini::deep-research-pro-preview-12-2025"));
        assert!(!is_routable("gemini::imagen-4.0-generate-001"));
        assert!(!is_routable("gemini::veo-3.0-generate-001"));
        assert!(!is_routable("gemini::gemini-2.5-flash-image"));
        assert!(!is_routable("gemini::gemini-embedding-001"));
        assert!(!is_routable("groq::whisper-large-v3"));
        assert!(!is_routable("groq::meta-llama/llama-prompt-guard-2-86m"));
        // OpenAI's list mixes in video / realtime-voice / speech-to-text / legacy base models too.
        assert!(!is_routable("openai::sora-2"));
        assert!(!is_routable("openai::sora-2-pro"));
        assert!(!is_routable("openai::gpt-realtime"));
        assert!(!is_routable("openai::gpt-realtime-mini"));
        assert!(!is_routable("openai::gpt-4o-transcribe"));
        assert!(!is_routable("openai::davinci-002"));
        assert!(!is_routable("openai::babbage-002"));
        assert!(is_routable("gemini::gemini-flash-lite-latest"));
        assert!(is_routable("codex-cli::gpt-5.5"));
        assert!(is_routable("groq::llama-3.1-8b-instant"));
        assert!(
            is_routable("openai::gpt-5.5"),
            "real chat model stays routable"
        );
        assert!(
            is_routable("openai::gpt-4o-search-preview"),
            "search-augmented chat stays routable"
        );

        // A trivial pick from a gemini-like set must be a fast chat model, not deep-research.
        let cat = ModelCatalog::new(vec![
            "gemini::deep-research-pro-preview-12-2025".into(),
            "gemini::gemini-flash-lite-latest".into(),
            "gemini::imagen-4.0-generate-001".into(),
        ]);
        let r = cat.ranked_for(TaskTier::Trivial, &Pricing::default(), 5);
        assert_eq!(
            r.first().unwrap(),
            "gemini::gemini-flash-lite-latest",
            "{r:?}"
        );
        assert!(!r
            .iter()
            .any(|m| m.contains("deep-research") || m.contains("imagen")));
    }

    #[test]
    fn task_specific_models_are_excluded_from_general_routing() {
        // Translation-only, reranker, and embedding-variant endpoints appear in provider model
        // lists (esp. NVIDIA NIM) but can't answer a general chat/JSON turn — routing to
        // `riva-translate` just echoes/translates the prompt (the reported bug). They must be
        // filtered out of the general routing pool.
        assert!(!is_routable(
            "nvidia::nvidia/riva-translate-4b-instruct-v1.1"
        ));
        assert!(!is_routable("nvidia::nvidia/llama-3.2-nv-rerankqa-1b-v2"));
        assert!(!is_routable("nvidia::nvidia/nv-embedqa-e5-v5"));
        assert!(!is_routable("cohere::embed-english-v3.0"));
        // A real instruct model whose id merely contains "instruct" stays routable.
        assert!(is_routable("nvidia::meta/llama-3.3-70b-instruct"));

        // With a translation model in the catalog alongside a real instruct model, ranking picks
        // the instruct model — never the translator.
        let cat = ModelCatalog::new(vec![
            "nvidia::nvidia/riva-translate-4b-instruct-v1.1".into(),
            "nvidia::meta/llama-3.3-70b-instruct".into(),
        ]);
        let r = cat.ranked_for(TaskTier::Trivial, &Pricing::default(), 5);
        assert!(
            !r.iter().any(|m| m.contains("riva-translate")),
            "translation model must not be in the general routing pool: {r:?}"
        );
        assert_eq!(r.first().unwrap(), "nvidia::meta/llama-3.3-70b-instruct");
    }

    #[test]
    fn empty_catalog_ranks_to_nothing() {
        assert!(ModelCatalog::default()
            .ranked_for(TaskTier::Standard, &Pricing::default(), 3)
            .is_empty());
    }

    fn overview_catalog() -> ModelCatalog {
        ModelCatalog::new(vec![
            "anthropic::claude-opus-4-8".into(),            // frontier, paid
            "openai::gpt-4o-mini".into(),                   // small, paid
            "groq::llama-3.1-8b-instant".into(),            // small, free (unpriced free-tier)
            "groq::llama-3.3-70b-versatile".into(),         // frontier, free
            "ollama::llama3.2".into(),                      // free, local
            "claude-cli::sonnet".into(),                    // subscription bridge
            "openrouter::anthropic/claude-opus-4".into(), // frontier, PAID gateway (no price, no :free)
            "openrouter::deepseek/deepseek-r1:free".into(), // frontier, free (:free variant)
            "opencode_go::glm-5.2".into(),                // PAID gateway model billing key balance
        ])
    }

    #[test]
    fn openrouter_unpriced_models_are_paid_unless_free_suffixed() {
        let infos = overview_catalog().infos(&Pricing::default());
        // A paid OpenRouter frontier model we hold no price for must NOT read as free (the bug).
        let opus = infos
            .iter()
            .find(|m| m.id == "openrouter::anthropic/claude-opus-4")
            .unwrap();
        assert!(opus.frontier && opus.paid && !opus.free, "{opus:?}");
        // Its `:free` sibling is correctly free.
        let r1 = infos.iter().find(|m| m.id.contains(":free")).unwrap();
        assert!(r1.free && !r1.paid, "{r1:?}");
    }

    #[test]
    fn opencode_zen_unpriced_models_are_paid_not_free() {
        // OpenCode Zen bills a shared key balance for premium models (glm/kimi/qwen-max). Reading
        // its unpriced models as free silently drains that balance — they must read as paid.
        let infos = overview_catalog().infos(&Pricing::default());
        let glm = infos
            .iter()
            .find(|m| m.id == "opencode_go::glm-5.2")
            .unwrap();
        assert!(glm.paid && !glm.free, "{glm:?}");
    }

    #[test]
    fn unpriced_metered_api_models_are_paid_not_free() {
        // The live billing bug: gpt-5.5 / gpt-5-pro / gemini-3-pro have no entry in the bundled
        // price table, so the old `_ => true` fallback read them as FREE and cost-routing would
        // bill the user. An UNPRICED model from a metered API provider must read as paid; only
        // genuinely-free providers (local/free-tier) are free without a price.
        let cat = ModelCatalog::new(vec![
            "openai::gpt-5.5".into(),
            "openai::gpt-5-pro".into(),
            "gemini::gemini-3-pro-preview".into(),
            "xai::grok-4".into(),
            "deepseek::deepseek-v4-pro".into(),
            "ollama::qwen2.5-coder:3b".into(),
        ]);
        let infos = cat.infos(&Pricing::default());
        for id in [
            "openai::gpt-5.5",
            "openai::gpt-5-pro",
            "gemini::gemini-3-pro-preview",
            "xai::grok-4",
            "deepseek::deepseek-v4-pro",
        ] {
            let m = infos.iter().find(|m| m.id == id).unwrap();
            assert!(
                m.paid && !m.free,
                "unpriced metered API model must be paid, not free: {m:?}"
            );
        }
        let local = infos.iter().find(|m| m.provider == "ollama").unwrap();
        assert!(local.free, "local ollama is genuinely free");
    }

    #[test]
    fn gemini_flash_is_free_but_pro_is_paid() {
        // Gemini keeps a standing free tier for Flash / Flash-Lite (and Gemma), but Pro is paid-only
        // since Apr 2026. Unpriced Flash → free; unpriced Pro → paid.
        let cat = ModelCatalog::new(vec![
            "gemini::gemini-3-flash-preview".into(),
            "gemini::gemini-2.5-flash-lite".into(),
            "gemini::gemini-flash-latest".into(),
            "gemini::gemini-3-pro-preview".into(),
            "gemini::gemini-pro-latest".into(),
        ]);
        let infos = cat.infos(&Pricing::default());
        for id in [
            "gemini::gemini-3-flash-preview",
            "gemini::gemini-2.5-flash-lite",
            "gemini::gemini-flash-latest",
        ] {
            let m = infos.iter().find(|m| m.id == id).unwrap();
            assert!(
                m.free && !m.paid,
                "unpriced Gemini Flash is free-tier: {m:?}"
            );
        }
        for id in ["gemini::gemini-3-pro-preview", "gemini::gemini-pro-latest"] {
            let m = infos.iter().find(|m| m.id == id).unwrap();
            assert!(m.paid && !m.free, "Gemini Pro is paid-only: {m:?}");
        }
    }

    #[test]
    fn paid_free_and_subscription_are_mutually_exclusive() {
        for m in overview_catalog().infos(&Pricing::default()) {
            let n = [m.free, m.paid, m.subscription]
                .iter()
                .filter(|b| **b)
                .count();
            assert_eq!(n, 1, "exactly one category per model: {m:?}");
        }
    }

    #[test]
    fn classifies_frontier_free_and_subscription() {
        let infos = overview_catalog().infos(&Pricing::default());
        let opus = infos.iter().find(|m| m.id.contains("opus")).unwrap();
        assert!(opus.frontier && !opus.free && !opus.subscription && opus.cost > 0.0);

        let g70 = infos.iter().find(|m| m.id.contains("70b")).unwrap();
        assert!(g70.frontier && g70.free, "free frontier groq model");

        let local = infos.iter().find(|m| m.provider == "ollama").unwrap();
        assert!(local.free && !local.frontier && local.cost == 0.0);

        let bridge = infos.iter().find(|m| m.provider == "claude-cli").unwrap();
        assert!(
            bridge.subscription && !bridge.free,
            "subscription bridge is not counted as free"
        );
        assert_eq!(bridge.name, "sonnet");
    }

    #[test]
    fn bare_bridge_ids_never_enter_the_catalog() {
        // `claude-cli::` (empty model name) is a routing pin, not a catalog row — it rendered as
        // a garbage empty id in `forge models` and could never match a benchmark or window.
        let cat = ModelCatalog::new(vec![
            "claude-cli::".into(),
            "codex-cli::".into(),
            "agy-cli::".into(),
            "claude-cli::fable".into(),
        ]);
        assert_eq!(cat.models(), ["claude-cli::fable".to_string()]);
        // The serde path (catalog cache written before the guard) is filtered too.
        let cached: ModelCatalog =
            serde_json::from_str(r#"{"models":["claude-cli::","claude-cli::fable"],"bench":null}"#)
                .unwrap();
        assert_eq!(cached.models(), ["claude-cli::fable".to_string()]);
    }

    #[test]
    fn stats_count_each_category() {
        let s = overview_catalog().stats(&Pricing::default());
        assert_eq!(s.total, 9);
        assert_eq!(s.providers, 7); // anthropic, openai, groq, ollama, claude-cli, openrouter, opencode_go
        assert_eq!(s.frontier, 5); // anthropic-opus, groq-70b, claude-cli-sonnet, or-opus, or-deepseek-r1
        assert_eq!(s.subscription, 1); // claude-cli
        assert_eq!(s.free, 4); // groq-8b, groq-70b, ollama, or-deepseek-r1:free
        assert_eq!(s.paid, 4); // anthropic-opus, gpt-4o-mini, or-opus, opencode-glm
    }

    #[test]
    fn within_a_subscription_family_the_higher_version_wins() {
        // The gpt-5.2-over-5.5 bug: among same-provider, same-class $0 models, never pick the
        // lesser sibling. fine_capability orders 5.5 > 5.4 > 5.2 (and the mini stays a small/
        // trivial model, not a complex pick).
        let cat = ModelCatalog::new(vec![
            "codex-cli::gpt-5.2".into(),
            "codex-cli::gpt-5.4".into(),
            "codex-cli::gpt-5.5".into(),
            "codex-cli::gpt-5.4-mini".into(),
        ]);
        let r = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 4);
        assert_eq!(
            r[0], "codex-cli::gpt-5.5",
            "highest version leads complex: {r:?}"
        );
        assert!(
            r.iter().position(|m| m == "codex-cli::gpt-5.5").unwrap()
                < r.iter().position(|m| m == "codex-cli::gpt-5.2").unwrap(),
            "5.5 must rank above 5.2: {r:?}"
        );
        // The mini is small-class → it is NOT the complex pick.
        assert_ne!(r[0], "codex-cli::gpt-5.4-mini");
    }

    #[test]
    fn on_a_score_tie_the_lighter_sibling_wins() {
        // opus and sonnet both rank q3 (frontier) for complex → identical score. The mesh should
        // spend the lighter sonnet, conserving opus' quota. (User rule: lightest-on-tie.)
        let cat = ModelCatalog::new(vec!["claude-cli::opus".into(), "claude-cli::sonnet".into()]);
        let r = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 2);
        assert_eq!(
            r[0], "claude-cli::sonnet",
            "lighter sibling leads on a tie: {r:?}"
        );

        // But a genuinely weaker sibling (haiku, lower score) must NOT jump ahead for complex.
        let cat2 = ModelCatalog::new(vec![
            "claude-cli::opus".into(),
            "claude-cli::sonnet".into(),
            "claude-cli::haiku".into(),
        ]);
        let r2 = cat2.ranked_for(TaskTier::Complex, &Pricing::default(), 3);
        assert_eq!(r2[0], "claude-cli::sonnet");
        assert_eq!(
            r2.last().unwrap(),
            "claude-cli::haiku",
            "weak sibling stays last: {r2:?}"
        );
    }

    #[test]
    fn bench_aware_conservation_guard_rejects_weak_large_models() {
        use crate::bench::BenchmarkScores;
        // Hermes 405B name-heuristic is q3 (via "-405b"), so the old guard would say "yes, capable
        // frontier alternative" and enable conservation. Bench score 9.0 is below
        // FRONTIER_BENCH_THRESHOLD (20.0), so with bench data the guard must refuse.
        let mut b = BenchmarkScores::new();
        b.insert("hermes 405b", 9.0, 8.0);
        let models = vec![
            "claude-cli::sonnet".to_string(),
            "openrouter::nousresearch/hermes-3-llama-3.1-405b".to_string(),
        ];
        let quota = forge_types::SubscriptionQuota::default()
            .with_fractions(std::collections::HashMap::from([(
                "claude-cli".to_string(),
                0.85,
            )]))
            .with_plans(std::collections::HashMap::from([(
                "claude-cli".to_string(),
                "plus".to_string(),
            )]))
            .with_conserve(true);
        let d = conserve_decision(
            &models,
            forge_types::TaskTier::Complex,
            false,
            42,
            &quota,
            Some(&b),
        );
        assert!(
            !d.eligible,
            "hermes 405B bench score 9.0 < FRONTIER_BENCH_THRESHOLD — not a frontier alternative: {d:?}"
        );
    }

    #[test]
    fn bench_aware_frontier_classification() {
        use crate::bench::BenchmarkScores;
        // A model the name heuristic misses (unknown family) but bench-scoring above
        // FRONTIER_BENCH_THRESHOLD must be classified as frontier.
        let mut b = BenchmarkScores::new();
        b.insert("acme mystery x", 55.0, 48.0);
        let cat =
            ModelCatalog::new(vec!["openrouter::acme/mystery-x".into()]).with_benchmarks(Some(b));
        let infos = cat.infos(&Pricing::default());
        assert!(
            infos[0].frontier,
            "bench 55.0 > FRONTIER_BENCH_THRESHOLD → frontier: {:?}",
            infos[0]
        );
    }

    /// Regression: the effort-biased comparator used a pairwise "prefer higher bench when the
    /// gap ≥ 1.0" rule that was NOT a total order (a within 1 of b, b within 1 of c, a and c
    /// more than 1 apart → contradictory orderings). With enough models whose scores straddle
    /// the threshold, `sort_by` panicked with "user-provided comparison function does not
    /// correctly implement a total order" — hit live on the first white-hot routed turn.
    /// The banded comparator must rank a saturated catalog without panicking.
    #[test]
    fn effort_ranking_is_a_total_order_on_threshold_straddling_scores() {
        use crate::bench::BenchmarkScores;
        let mut b = BenchmarkScores::new();
        let mut ids: Vec<String> = Vec::new();
        for i in 0..80 {
            // Adjacent gaps of 0.6 — every neighbor "ties", every third model doesn't: the
            // exact shape that broke the old pairwise rule.
            b.insert(&format!("t m{i}"), 30.0 + (i as f64) * 0.6, 20.0);
            ids.push(format!("openrouter::t/m{i}"));
        }
        ids.push("openrouter::t/unbenched".into());
        let cat = ModelCatalog::new(ids).with_benchmarks(Some(b));
        let ranked = cat.ranked_seeded(
            TaskTier::Trivial,
            &Pricing::default(),
            100,
            false,
            7,
            &forge_types::SubscriptionQuota::default(),
            Some(EffortLevel::WhiteHot),
        );
        assert_eq!(ranked.len(), 81, "every model ranked, no panic");
        assert_eq!(
            ranked.last().map(String::as_str),
            Some("openrouter::t/unbenched"),
            "unbenched sinks below benched at white-hot effort"
        );
    }

    #[test]
    fn fine_capability_parses_versions() {
        assert!(fine_capability("codex-cli::gpt-5.5") > fine_capability("codex-cli::gpt-5.4"));
        assert!(fine_capability("codex-cli::gpt-5.4") > fine_capability("codex-cli::gpt-5.2"));
        assert!(
            (fine_capability("anthropic::claude-opus-4-8") - 4.8).abs() < 1e-9,
            "4-8 → 4.8"
        );
        assert_eq!(fine_capability("ollama::llama3"), 3.0);
    }

    #[test]
    fn fine_capability_does_not_overflow_on_long_digit_runs() {
        // Regression: model ids are sourced from external provider/gateway catalogs and could
        // contain a long digit run (embedded hash, snowflake id, timestamp, ...). The accumulator
        // must not overflow `u32` (dev/test builds have `overflow-checks = true` and would panic).
        let _ = fine_capability("openrouter::model-99999999999999999999");
        let _ = fine_capability("openrouter::model-1.99999999999999999999");
        let _ = fine_capability("openrouter::model-18446744073709551616");
    }

    #[test]
    fn groups_by_provider_richest_first_frontier_leads() {
        let groups = overview_catalog().by_provider(&Pricing::default());
        // groq has 2 models → it leads.
        assert_eq!(groups[0].provider, "groq");
        assert_eq!(groups[0].total(), 2);
        // within groq, the frontier 70b sorts before the 8b.
        assert!(groups[0].models[0].id.contains("70b"));
        assert_eq!(groups[0].frontier(), 1);
        assert_eq!(groups[0].free(), 2);
    }

    fn effort_test_catalog(a_score: f64, b_score: f64) -> (ModelCatalog, Pricing) {
        use crate::bench::BenchmarkScores;
        use crate::pricing::ModelRate;
        use std::collections::HashMap;

        let mut rates = HashMap::new();
        rates.insert(
            "openai::model-a".to_string(),
            ModelRate {
                input_per_1k: 0.1,
                output_per_1k: 0.1,
                cache_read_per_1k: None,
            },
        );
        rates.insert(
            "openai::model-b".to_string(),
            ModelRate {
                input_per_1k: 0.001,
                output_per_1k: 0.001,
                cache_read_per_1k: None,
            },
        );

        let mut bench = BenchmarkScores::new();
        bench.insert("openai model-a", a_score, a_score);
        bench.insert("openai model-b", b_score, b_score);

        (
            ModelCatalog::new(vec!["openai::model-a".into(), "openai::model-b".into()])
                .with_benchmarks(Some(bench)),
            Pricing::from_rates(rates),
        )
    }

    #[test]
    fn none_and_medium_effort_keep_existing_routing_order() {
        use forge_types::{EffortLevel, SubscriptionQuota};

        let (cat, pricing) = effort_test_catalog(25.0, 20.0);
        let quota = SubscriptionQuota::default();
        let none = cat.ranked_seeded(TaskTier::Complex, &pricing, 2, false, 0, &quota, None);
        let medium = cat.ranked_seeded(
            TaskTier::Complex,
            &pricing,
            2,
            false,
            0,
            &quota,
            Some(EffortLevel::Medium),
        );

        assert_eq!(none, medium);
    }

    #[test]
    fn high_effort_prefers_higher_benchmark_over_lower_cost() {
        use forge_types::{EffortLevel, SubscriptionQuota};

        let (cat, pricing) = effort_test_catalog(25.0, 20.0);
        let r = cat.ranked_seeded(
            TaskTier::Complex,
            &pricing,
            2,
            false,
            0,
            &SubscriptionQuota::default(),
            Some(EffortLevel::High),
        );

        assert_eq!(r[0], "openai::model-a");
    }

    #[test]
    fn low_effort_prefers_lower_cost_when_benchmark_gap_is_small() {
        use forge_types::{EffortLevel, SubscriptionQuota};

        let (cat, pricing) = effort_test_catalog(20.5, 20.0);
        let r = cat.ranked_seeded(
            TaskTier::Complex,
            &pricing,
            2,
            false,
            0,
            &SubscriptionQuota::default(),
            Some(EffortLevel::Low),
        );

        assert_eq!(r[0], "openai::model-b");
    }

    // ── Routing scenario tests: no model should monopolise all tiers ──────────────────

    fn minimax_catalog() -> ModelCatalog {
        // A realistic NVIDIA NIM catalog: minimax-m3 (large free) vs a genuinely fast small
        // model (llama-8b on groq, also free). minimax-m3's name formerly matched the "mini"
        // small-model check, giving it speed_class=3 and making it win every tier.
        ModelCatalog::new(vec![
            "nvidia::minimaxai/minimax-m3".into(),
            "groq::llama-3.1-8b-instant".into(),
            "groq::llama-3.3-70b-versatile".into(),
        ])
    }

    #[test]
    fn minimax_m3_does_not_win_trivial_over_fast_small_model() {
        // Trivial tier heavily weights speed (s*2 + q*0.5). After the -mini fix, minimax-m3
        // is quality_class=2 (speed_class=2), while llama-8b is quality_class=1 (speed_class=3).
        // llama-8b must lead on trivial; minimax-m3 must NOT be first.
        let r = minimax_catalog().ranked_for(TaskTier::Trivial, &Pricing::default(), 3);
        assert_ne!(
            r[0], "nvidia::minimaxai/minimax-m3",
            "minimax-m3 must not win trivial over a genuinely fast small model: {r:?}"
        );
        assert_eq!(
            r[0], "groq::llama-3.1-8b-instant",
            "the fast 8b model must lead trivial: {r:?}"
        );
    }

    #[test]
    fn minimax_m3_does_not_monopolise_all_tiers_without_bench() {
        // Without benchmark data the heuristic alone must not funnel every tier to minimax.
        let cat = minimax_catalog();
        let trivial = cat.ranked_for(TaskTier::Trivial, &Pricing::default(), 1);
        let complex = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 1);
        assert_ne!(
            trivial[0], "nvidia::minimaxai/minimax-m3",
            "trivial must not go to minimax: {trivial:?}"
        );
        // Complex is fine to go to 70b or minimax — just asserting trivial spreads.
        let _ = complex;
    }

    #[test]
    fn minimax_m3_does_not_monopolise_all_tiers_with_high_bench_score() {
        use crate::bench::BenchmarkScores;
        // Even with a high AA intelligence score (35, a real value for MiniMax M3), the
        // trivial tier must still prefer the genuinely fast small model.
        let mut b = BenchmarkScores::new();
        b.insert("minimax m3", 35.0, 33.0);
        b.insert("llama 3.1 8b instant", 6.1, 5.0);
        b.insert("llama 3.3 70b versatile", 10.0, 9.0);
        let cat = minimax_catalog().with_benchmarks(Some(b));

        let trivial = cat.ranked_for(TaskTier::Trivial, &Pricing::default(), 3);
        assert_ne!(
            trivial[0], "nvidia::minimaxai/minimax-m3",
            "even with high bench score minimax must not dominate trivial: {trivial:?}"
        );

        // Complex: with bench 35 vs 10, minimax is the strongest available — that IS correct.
        let complex = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 3);
        assert_ne!(
            complex[0], "groq::llama-3.1-8b-instant",
            "tiny 8b must not win complex: {complex:?}"
        );
    }

    #[test]
    fn fast_small_model_leads_trivial_across_provider_mix() {
        // Realistic multi-provider set: ensure a tiny fast free model beats large frontier
        // on trivial regardless of how many large models are present.
        let cat = ModelCatalog::new(vec![
            "nvidia::minimaxai/minimax-m3".into(),
            "nvidia::meta/llama-3.1-70b-instruct".into(),
            "groq::llama-3.1-8b-instant".into(),
            "claude-cli::opus".into(),
            "openrouter::deepseek/deepseek-r1:free".into(),
        ]);
        let r = cat.ranked_for(TaskTier::Trivial, &Pricing::default(), 5);
        assert_eq!(
            r[0], "groq::llama-3.1-8b-instant",
            "fast free 8b must win trivial in a mixed catalog: {r:?}"
        );
    }

    #[test]
    fn no_provider_monopolises_all_three_tiers_in_realistic_catalog() {
        // With a balanced catalog, the routing should spread across tiers: no single model
        // wins trivial + standard + complex simultaneously (healthy tier differentiation).
        use crate::bench::BenchmarkScores;
        let cat = ModelCatalog::new(vec![
            "nvidia::minimaxai/minimax-m3".into(),
            "groq::llama-3.1-8b-instant".into(),
            "groq::llama-3.3-70b-versatile".into(),
            "claude-cli::sonnet".into(),
        ]);
        let mut b = BenchmarkScores::new();
        b.insert("minimax m3", 35.0, 33.0);
        b.insert("llama 3.1 8b instant", 6.1, 5.0);
        b.insert("llama 3.3 70b versatile", 10.0, 9.0);
        let cat = cat.with_benchmarks(Some(b));

        let trivial = cat.ranked_for(TaskTier::Trivial, &Pricing::default(), 1);
        let complex = cat.ranked_for(TaskTier::Complex, &Pricing::default(), 1);

        assert_ne!(
            trivial[0], complex[0],
            "trivial and complex must not route to the same model — healthy tier spread expected: trivial={:?} complex={:?}",
            trivial, complex
        );
        assert_eq!(
            trivial[0], "groq::llama-3.1-8b-instant",
            "trivial must pick the fast 8b: {trivial:?}"
        );
    }

    #[test]
    fn supports_vision_recognizes_known_vision_families() {
        for id in [
            "openai::gpt-4o",
            "openai::gpt-4-turbo",
            "openai::gpt-4.1",
            "openai::gpt-5.4",
            "openai::o3",
            "anthropic::claude-opus-4-8",
            "anthropic::claude-3.5-sonnet",
            "claude-cli::sonnet",
            "claude-cli::opus",
            "gemini::gemini-2.5-pro",
            "gemini::gemini-2.5-flash",
            "openrouter::meta-llama/llama-3.2-90b-vision-instruct",
            "openrouter::meta-llama/llama-4-scout",
            "mistral::pixtral-12b-2409",
            "openrouter::qwen/qwen2.5-vl-72b-instruct",
            "openrouter::qwen/qwen3-vl-235b-a22b",
            "xai::grok-4",
        ] {
            assert!(
                supports_vision(id),
                "{id} should be recognized as vision-capable"
            );
        }
    }

    #[test]
    fn supports_vision_rejects_text_only_models() {
        for id in [
            "groq::llama-3.1-8b-instant",
            "groq::llama-3.3-70b-versatile",
            "openai::gpt-4",
            "anthropic::claude-2.1",
            "anthropic::claude-instant-1.2",
            "deepseek::deepseek-v3",
            "openrouter::qwen/qwen2.5-72b-instruct",
            "ollama::llama3.2",
            "mistral::mistral-large-2411",
            "openai::davinci-002",
        ] {
            assert!(
                !supports_vision(id),
                "{id} should NOT be recognized as vision-capable"
            );
        }
    }

    // --- Fix 2: subscription burn-weight penalty in route_score ---------------------------

    #[test]
    fn gpt56_luna_outranks_sol_at_trivial_and_standard_with_a_fresh_quota() {
        // No bench data: quality_class ties sol/luna at the same frontier class (both match
        // "gpt-5"), so the outcome is driven entirely by the speed_class Luna now correctly gets
        // credit for (Fix 3) plus its lower burn penalty (Fix 2, both effectively zero at weight
        // 1.0). Same provider (codex-oauth) on both ids, so no rotation/tiebreak noise.
        let cat = ModelCatalog::new(vec![
            "codex-oauth::gpt-5.6-sol".into(),
            "codex-oauth::gpt-5.6-luna".into(),
        ]);
        let fresh = forge_types::SubscriptionQuota::default();
        for tier in [TaskTier::Trivial, TaskTier::Standard] {
            let r = cat.ranked_seeded(tier, &Pricing::default(), 2, false, 0, &fresh, None);
            assert_eq!(
                r[0], "codex-oauth::gpt-5.6-luna",
                "{tier:?}: luna should outrank sol with a fresh quota: {r:?}"
            );
        }
    }

    #[test]
    fn gpt56_sol_still_outranks_luna_at_complex_given_a_real_capability_gap() {
        // At Complex, capability (q, weight 2.0) dominates speed (weight 0.25) — but only a real
        // measured capability gap can express that in the score, since the name heuristic alone
        // ties sol/luna's quality_class. This is what makes Sol worth its 5x burn: a genuinely
        // higher intelligence index, exactly the real-world justification for the burn-weight
        // table (Sol costs 5x because it measurably performs better, not merely by name).
        use crate::bench::BenchmarkScores;
        let cat = ModelCatalog::new(vec![
            "codex-oauth::gpt-5.6-sol".into(),
            "codex-oauth::gpt-5.6-luna".into(),
        ]);
        let mut b = BenchmarkScores::new();
        b.insert("gpt-5.6-sol", 62.0, 60.0);
        b.insert("gpt-5.6-luna", 38.0, 36.0);
        let cat = cat.with_benchmarks(Some(b));
        let fresh = forge_types::SubscriptionQuota::default();
        let r = cat.ranked_seeded(
            TaskTier::Complex,
            &Pricing::default(),
            2,
            false,
            0,
            &fresh,
            None,
        );
        assert_eq!(
            r[0], "codex-oauth::gpt-5.6-sol",
            "sol should still win complex given a real capability gap: {r:?}"
        );
    }

    #[test]
    fn near_exhausted_quota_penalizes_sol_more_than_a_fresh_one() {
        let cat = ModelCatalog::new(vec!["codex-oauth::gpt-5.6-sol".into()]);
        let fresh = forge_types::SubscriptionQuota::default();
        let mut fr = HashMap::new();
        fr.insert("codex-oauth".to_string(), 0.95);
        let near_exhausted = forge_types::SubscriptionQuota::new(HashMap::new()).with_fractions(fr);

        let (_, fresh_rows) = cat.ranked_rows(
            TaskTier::Standard,
            &Pricing::default(),
            false,
            0,
            &fresh,
            None,
        );
        let (_, pressured_rows) = cat.ranked_rows(
            TaskTier::Standard,
            &Pricing::default(),
            false,
            0,
            &near_exhausted,
            None,
        );
        let fresh_score = fresh_rows
            .iter()
            .find(|r| r.model == "codex-oauth::gpt-5.6-sol")
            .unwrap()
            .final_score;
        let pressured_score = pressured_rows
            .iter()
            .find(|r| r.model == "codex-oauth::gpt-5.6-sol")
            .unwrap()
            .final_score;
        assert!(
            pressured_score < fresh_score,
            "a near-exhausted quota must penalize sol more than a fresh one: fresh={fresh_score} pressured={pressured_score}"
        );
    }

    #[test]
    fn model_with_no_burn_weight_scores_exactly_as_before() {
        // `codex-cli::gpt-5.4-mini` has no burn-weight table entry (not gpt-5.6-*), so both the
        // burn penalty (weight defaults to 1.0 -> ln(1.0) = 0) and speed_class (falls through to
        // the unaffected quality_class heuristic) must leave its score IDENTICAL to the pre-Fix
        // formula. Hand-computed: quality_class("gpt-5.4-mini") = 1 (small, "-mini" marker),
        // speed_class = 3 (unaffected fallback) -> capability_score_b(Standard) = q + s = 4.0;
        // + cost_pref(Standard, cost_class=1 subscription) = 0.6; + code_prior = 0.0 (not
        // code_heavy); - tool_reliability_penalty = 0.0; - burn_penalty = 0.0 (weight 1.0) = 4.6.
        let id = "codex-cli::gpt-5.4-mini";
        let quota = forge_types::SubscriptionQuota::default();
        let cost = Pricing::default().estimated_cost(id);
        let overrides = HashMap::new();
        let score = route_score(
            id,
            TaskTier::Standard,
            cost,
            false,
            &quota,
            None,
            &overrides,
            false,
        );
        assert!(
            (score - 4.6).abs() < 1e-9,
            "unaffected model's score must match the hand-computed pre-Fix constant exactly: {score}"
        );
    }

    // --- OAuth-supersedes-bridge demotion (per-model, BRIDGE_SUPERSEDE_PENALTY) ------------

    #[test]
    fn superseded_bridge_ids_only_flags_a_bridge_with_a_live_oauth_twin() {
        let with_twin = vec![
            "codex-oauth::gpt-5.6-sol".to_string(),
            "codex-cli::gpt-5.6-sol".to_string(),
        ];
        let s = superseded_bridge_ids(&with_twin);
        assert!(s.contains("codex-cli::gpt-5.6-sol"), "bridge twin flagged");
        assert!(
            !s.contains("codex-oauth::gpt-5.6-sol"),
            "the oauth id itself is never penalized"
        );

        let bridge_only = vec!["codex-cli::gpt-5.6-sol".to_string()];
        assert!(
            superseded_bridge_ids(&bridge_only).is_empty(),
            "no oauth twin present → nothing flagged"
        );

        let unrelated_pair = vec![
            "codex-oauth::gpt-5.6-sol".to_string(),
            "claude-cli::opus".to_string(),
        ];
        assert!(
            superseded_bridge_ids(&unrelated_pair).is_empty(),
            "claude-cli has no entry in OAUTH_SUPERSEDES, so it is never flagged: {:?}",
            superseded_bridge_ids(&unrelated_pair)
        );
    }

    #[test]
    fn oauth_twin_outranks_bridge_twin_at_every_tier_bridge_stays_in_the_chain() {
        // With both surfaces present the twins otherwise score identically (same bare model →
        // same capability/burn/cost class, tied code_prior, default/shared quota), so the flat
        // BRIDGE_SUPERSEDE_PENALTY must make the oauth id win at EVERY tier — while the bridge
        // stays in the ranked output as a failover, not removed.
        let cat = ModelCatalog::new(vec![
            "codex-oauth::gpt-5.6-sol".into(),
            "codex-cli::gpt-5.6-sol".into(),
        ]);
        for tier in [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex] {
            let r = cat.ranked_for(tier, &Pricing::default(), 2);
            assert_eq!(
                r[0], "codex-oauth::gpt-5.6-sol",
                "{tier:?}: the oauth twin must outrank the bridge twin: {r:?}"
            );
            assert!(
                r.contains(&"codex-cli::gpt-5.6-sol".to_string()),
                "{tier:?}: the bridge twin must remain in the ranked output as a fallback: {r:?}"
            );
        }
    }

    #[test]
    fn bridge_with_no_live_oauth_twin_is_not_demoted() {
        // No `codex-oauth::` entry in the catalog at all → no penalty; the score matches the
        // same pre-existing hand-computed baseline as `model_with_no_burn_weight_scores_exactly_as_before`.
        let cat = ModelCatalog::new(vec!["codex-cli::gpt-5.4-mini".into()]);
        let (_, rows) = cat.ranked_rows(
            TaskTier::Standard,
            &Pricing::default(),
            false,
            0,
            &forge_types::SubscriptionQuota::default(),
            None,
        );
        let row = rows
            .iter()
            .find(|r| r.model == "codex-cli::gpt-5.4-mini")
            .unwrap();
        assert!(
            (row.final_score - 4.6).abs() < 1e-9,
            "no oauth twin present anywhere in the catalog → score unchanged: {}",
            row.final_score
        );
    }

    #[test]
    fn bridge_model_absent_from_the_pair_list_is_never_penalized() {
        // `claude-cli` has no entry in `OAUTH_SUPERSEDES` (no xai bridge twin exists to
        // supersede), so `claude-cli::opus` must never take the penalty even alongside an
        // unrelated oauth surface in the same catalog.
        let cat = ModelCatalog::new(vec![
            "claude-cli::opus".into(),
            "codex-oauth::gpt-5.6-sol".into(),
        ]);
        let (_, rows) = cat.ranked_rows(
            TaskTier::Complex,
            &Pricing::default(),
            false,
            0,
            &forge_types::SubscriptionQuota::default(),
            None,
        );
        let with_pair = rows
            .iter()
            .find(|r| r.model == "claude-cli::opus")
            .unwrap()
            .final_score;

        let solo_cat = ModelCatalog::new(vec!["claude-cli::opus".into()]);
        let (_, solo_rows) = solo_cat.ranked_rows(
            TaskTier::Complex,
            &Pricing::default(),
            false,
            0,
            &forge_types::SubscriptionQuota::default(),
            None,
        );
        let without_pair = solo_rows
            .iter()
            .find(|r| r.model == "claude-cli::opus")
            .unwrap()
            .final_score;

        assert!(
            (with_pair - without_pair).abs() < 1e-9,
            "claude-cli::opus's score must be identical whether or not an unrelated oauth model \
             is present: with={with_pair} without={without_pair}"
        );
    }

    #[test]
    fn conservation_is_per_provider_and_overrides_high_effort_bench_bands() {
        let mut bench = BenchmarkScores::new();
        bench.insert("sonnet", 60.0, 60.0);
        bench.insert("gpt-5.5", 60.0, 60.0);
        bench.insert("llama-3.3-70b-versatile", 20.0, 20.0);
        let cat = ModelCatalog::new(vec![
            "claude-cli::sonnet".into(),
            "codex-cli::gpt-5.5".into(),
            "groq::llama-3.3-70b-versatile".into(),
        ])
        .with_benchmarks(Some(bench));
        let quota = forge_types::SubscriptionQuota::default()
            .with_conserve(true)
            .with_fractions(HashMap::from([
                ("claude-cli".into(), 1.0),
                ("codex-cli".into(), 0.0),
            ]));
        let (_, rows) = cat.ranked_rows(
            TaskTier::Complex,
            &Pricing::default(),
            false,
            0,
            &quota,
            Some(EffortLevel::High),
        );
        let claude = rows.iter().find(|r| r.provider == "claude-cli").unwrap();
        let codex = rows.iter().find(|r| r.provider == "codex-cli").unwrap();
        assert_eq!(claude.conserve_penalty, CONSERVE_PENALTY);
        assert_eq!(codex.conserve_penalty, 0.0);
        assert_ne!(
            rows[0].provider, "claude-cli",
            "conservation must override bench band: {rows:?}"
        );
    }

    #[test]
    fn standing_free_tiers_ignore_reference_prices() {
        assert!(is_free("gemini::gemini-2.5-flash", 1.0, false));
        assert!(is_free("groq::llama-3.3-70b-versatile", 1.0, false));
        assert!(!is_free("gemini::gemini-2.5-pro", 1.0, false));
    }

    #[test]
    fn subunit_burn_weight_is_neutral() {
        let overrides = HashMap::from([("sonnet".into(), 0.5)]);
        assert_eq!(
            subscription_burn_penalty(
                "claude-cli::sonnet",
                TaskTier::Complex,
                &forge_types::SubscriptionQuota::default(),
                &overrides
            ),
            0.0
        );
    }

    #[test]
    fn trivial_conservation_is_unconditional_across_plans() {
        for plan in ["", "plus", "pro", "max-20x"] {
            assert_eq!(
                conserve_probability(TaskTier::Trivial, 0.0, plan, false),
                1.0
            );
        }
    }
}
