//! Which reasoning rung each provider surface can actually be asked for, and which one Forge
//! really sent.
//!
//! A Forge `/effort` pin is a single five-rung ladder (low → medium → high → xhigh → white-hot),
//! but no two provider surfaces expose the same ladder, and several expose none at all. Passing a
//! pin straight through therefore produces one of three silent failures: the request is rejected,
//! the rung is ignored, or — most misleading — nothing is sent and the model quietly runs at the
//! provider's own default while Forge reports the pinned level.
//!
//! So resolution is explicit and its result is carried, not inferred: [`resolve`] returns both the
//! rung that will be requested and WHY, and every surface that renders effort reads the answer from
//! here rather than echoing the pin back at the user.
//!
//! The ladders below are recorded from each binary's own `--help` (checked 2026-09-05), never
//! guessed. A surface whose ladder is unknown is modelled as unknown, not assumed to be five rungs.

use crate::EffortLevel;

/// Why the requested rung is or is not the rung being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortReason {
    /// No pin is active, so Forge sends no effort field and the provider applies its own default.
    /// The rung the model then runs at is NOT knowable from here.
    ProviderDefault,
    /// The requested rung is supported and is what gets sent.
    AsRequested,
    /// The surface's ladder stops below the request, so the top rung it does have is sent.
    ClampedToCeiling,
    /// The surface exposes no reasoning-effort control at all; the pin cannot reach the model and
    /// only affects Forge-side routing.
    NoControl,
}

/// The outcome of asking a provider surface for a rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffortDecision {
    /// What the session asked for (`None` when nothing is pinned).
    pub requested: Option<EffortLevel>,
    /// What is actually put on the wire (`None` when nothing is sent).
    pub sent: Option<EffortLevel>,
    pub reason: EffortReason,
}

impl EffortDecision {
    /// Whether Forge itself determined the rung the model runs at. False whenever the model is
    /// running at a provider default — the case a status readout must not present as a known level.
    pub fn is_forge_set(self) -> bool {
        self.sent.is_some()
    }
}

/// Every rung, weakest first — the ladder a `/effort` pin is expressed in.
const FULL_LADDER: &[EffortLevel] = &[
    EffortLevel::Low,
    EffortLevel::Medium,
    EffortLevel::High,
    EffortLevel::XHigh,
    EffortLevel::WhiteHot,
];

/// The rungs a provider surface accepts, weakest first. Empty means the surface has no control.
///
/// `provider` is the namespace of a Forge id (`codex-oauth::gpt-6-astra` → `codex-oauth`).
pub fn ladder(provider: &str, model: &str) -> &'static [EffortLevel] {
    match provider {
        // `claude --effort <level>` documents exactly Forge's five rungs: "(low, medium, high,
        // xhigh, max)". White-hot maps onto `max`.
        "claude-cli" => FULL_LADDER,
        // `agy --effort` documents "(low|medium|high)" and nothing above it.
        "agy-cli" => &FULL_LADDER[..3],
        // Codex takes a config override rather than a flag, and validates the value against the
        // model ("Supported reasoning efforts: …"). Its own models cache carries a
        // `supported_reasoning_efforts` field but ships it null, so there is no authoritative list
        // to read; xhigh is included because it is a value codex has accepted in this install's
        // own config. `max` is deliberately excluded — no evidence it is accepted, and being
        // clamped one rung down is recoverable where a rejected turn is not.
        "codex-cli" | "codex-oauth" => &FULL_LADDER[..4],
        // Everything else goes over the generic (genai/OpenAI-compatible) path, where an effort
        // field is only meaningful for a reasoning model.
        _ => {
            if model_has_reasoning_control(model) {
                &FULL_LADDER[..4]
            } else {
                &[]
            }
        }
    }
}

/// Resolve a pin against what a provider surface can actually be asked for.
pub fn resolve(provider: &str, model: &str, requested: Option<EffortLevel>) -> EffortDecision {
    let rungs = ladder(provider, model);
    let Some(requested_level) = requested else {
        return EffortDecision {
            requested,
            sent: None,
            reason: EffortReason::ProviderDefault,
        };
    };
    let Some(&ceiling) = rungs.last() else {
        return EffortDecision {
            requested,
            sent: None,
            reason: EffortReason::NoControl,
        };
    };
    if rungs.contains(&requested_level) {
        return EffortDecision {
            requested,
            sent: Some(requested_level),
            reason: EffortReason::AsRequested,
        };
    }
    EffortDecision {
        requested,
        sent: Some(ceiling),
        reason: EffortReason::ClampedToCeiling,
    }
}

/// The argv that asks a CLI bridge for `level`.
///
/// Each binary spells the request differently — claude and agy take a `--effort` flag, codex has
/// no flag and takes a config override — so the spelling lives here beside the ladder rather than
/// in the bridge, where a new provider could quietly grow a fourth convention.
///
/// `provider` is a bridge namespace (`claude-cli`, `codex-cli`, `agy-cli`). Returns empty for a
/// surface with no CLI form.
pub fn bridge_args(provider: &str, level: EffortLevel) -> Vec<String> {
    let wire = wire_name(level);
    match provider {
        "claude-cli" | "agy-cli" => vec!["--effort".to_string(), wire.to_string()],
        // The harness path passes `--ignore-user-config`, so without this override a harness turn
        // runs at codex's built-in default while the user's own `model_reasoning_effort` is
        // deliberately not read — and the text path silently inherits whatever that file says.
        // Setting it explicitly makes both paths run at the rung Forge asked for.
        "codex-cli" => vec![
            "-c".to_string(),
            format!("model_reasoning_effort=\"{wire}\""),
        ],
        _ => Vec::new(),
    }
}

/// Whether a model served over the generic OpenAI-compatible path takes a reasoning-effort field
/// at all.
///
/// This is a model-FAMILY question, not an effort question: the OpenAI reasoning line rejects a
/// custom `temperature` whether or not an effort hint was set this turn, so the provider path uses
/// the same predicate to decide both. `gpt-6` is on the list because Astra rejects a temperature
/// exactly as the gpt-5 line does.
pub fn model_has_reasoning_control(model: &str) -> bool {
    let m = model.to_lowercase();
    let is_openai_reasoning = ["o1", "o1-", "o3", "o3-", "o4", "o4-", "gpt-5", "gpt-6"]
        .iter()
        .any(|needle| m == *needle || m.contains(&format!("::{needle}")) || m.contains(needle));

    is_openai_reasoning
        || m.contains("thinking")
        || m.contains("reasoning")
        || m.contains("deepseek-r1")
        || m.contains("r1-")
        || m == "deepseek-r1"
}

/// Split a Forge id into its provider namespace and bare model name.
pub fn split_id(id: &str) -> (&str, &str) {
    id.split_once("::").unwrap_or(("", id))
}

/// Whether this surface exposes any reasoning-effort control for this model.
///
/// Distinct from "is a rung being sent": an unpinned turn on a reasoning model sends nothing but
/// still HAS a control, while a non-reasoning model has none at any pin. A readout that conflates
/// the two renders an effort for a model that cannot have one.
pub fn has_control(id: &str) -> bool {
    let (provider, model) = split_id(id);
    !ladder(provider, model).is_empty()
}

/// Resolve a pin for a full Forge id.
pub fn resolve_id(id: &str, requested: Option<EffortLevel>) -> EffortDecision {
    let (provider, model) = split_id(id);
    resolve(provider, model, requested)
}

/// The wire spelling of a rung. White-hot is `max`: it asks the provider for the top reasoning
/// rung, and its additional lift is orchestration guidance in forge-core, not a provider setting.
pub fn wire_name(level: EffortLevel) -> &'static str {
    match level {
        EffortLevel::Low => "low",
        EffortLevel::Medium => "medium",
        EffortLevel::High => "high",
        EffortLevel::XHigh => "xhigh",
        EffortLevel::WhiteHot => "max",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpinned_session_sends_nothing_and_says_so() {
        // The failure this whole module exists to prevent: reporting a level as though the model
        // were running at it when Forge never sent one.
        let d = resolve_id("codex-oauth::gpt-6-astra", None);
        assert_eq!(d.sent, None);
        assert_eq!(d.reason, EffortReason::ProviderDefault);
        assert!(!d.is_forge_set());
    }

    #[test]
    fn a_supported_rung_is_sent_unchanged() {
        let d = resolve_id("codex-oauth::gpt-6-astra", Some(EffortLevel::Medium));
        assert_eq!(d.sent, Some(EffortLevel::Medium));
        assert_eq!(d.reason, EffortReason::AsRequested);
        assert!(d.is_forge_set());
    }

    #[test]
    fn a_pin_above_a_surfaces_ceiling_is_clamped_not_dropped() {
        // agy tops out at high. Dropping the field instead would silently hand the turn to agy's
        // own default, which is not the same thing as "as much effort as agy has".
        let d = resolve_id("agy-cli::gemini-3.8-flash", Some(EffortLevel::WhiteHot));
        assert_eq!(d.sent, Some(EffortLevel::High));
        assert_eq!(d.reason, EffortReason::ClampedToCeiling);
    }

    #[test]
    fn claude_takes_the_whole_ladder_including_the_top_rung() {
        // `claude --effort` documents low, medium, high, xhigh, max — the only surface that
        // accepts a distinct top rung, so white-hot reaches it unclamped.
        let d = resolve_id("claude-cli::opus", Some(EffortLevel::WhiteHot));
        assert_eq!(d.sent, Some(EffortLevel::WhiteHot));
        assert_eq!(d.reason, EffortReason::AsRequested);
        assert_eq!(wire_name(EffortLevel::WhiteHot), "max");
    }

    #[test]
    fn a_non_reasoning_model_reports_no_control_rather_than_a_level() {
        // A pin against a model with no reasoning knob must not render as an effort the model is
        // running at; it only affects Forge-side routing.
        let d = resolve_id("groq::llama-3.3-70b", Some(EffortLevel::High));
        assert_eq!(d.sent, None);
        assert_eq!(d.reason, EffortReason::NoControl);
        assert!(!d.is_forge_set());
    }

    #[test]
    fn having_a_control_is_not_the_same_as_sending_a_rung() {
        // An unpinned reasoning model sends nothing but still has a knob; a non-reasoning model has
        // none at any pin. A readout that conflates these renders an effort for a model that
        // cannot have one.
        assert!(has_control("codex-oauth::gpt-6-astra"));
        assert!(!has_control("groq::llama-3.3-70b"));
        assert!(has_control("claude-cli::opus"));
    }

    #[test]
    fn a_reasoning_model_on_the_generic_path_still_gets_its_rung() {
        let d = resolve_id("explabs::gpt-6-astra", Some(EffortLevel::High));
        assert_eq!(d.sent, Some(EffortLevel::High));
        assert_eq!(d.reason, EffortReason::AsRequested);
    }
}
