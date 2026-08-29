//! Vision (image-input) capability for a model id.
//!
//! Split out of `catalog.rs` rather than raising that file's architecture-size ratchet: the two
//! functions here are one cohesive concern (does this id accept images?) with their own pattern
//! tables, and `catalog.rs` was already the largest implementation owner in the crate.
//!
//! Both are name heuristics, not live capability queries — providers do not expose modality
//! uniformly. `catalog` re-exports them, so `catalog::supports_vision` keeps working.

/// Whether a model id is known to accept image input (vision). Providers don't expose this
/// uniformly, so — like [`crate::catalog::is_routable`] and the capability priors in `capability.rs` — this is a
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
        "qwen2.5-vl",
        "qwen3-vl",
        // xAI: every Grok model accepts image input.
        "grok",
    ];
    VISION_PATTERNS.iter().any(|p| m.contains(p))
}

/// Best-effort vision capability classification for callers that need to validate an explicit
/// model pin. `Some(false)` is returned only for model families whose text-only status is known;
/// `None` means the catalog has no reliable modality information and the provider should decide.
/// This distinction matters for custom/OpenRouter models whose ids are not covered by the
/// heuristic vision allowlist but may still accept images.
pub fn vision_capability(id: &str) -> Option<bool> {
    if supports_vision(id) {
        return Some(true);
    }
    let m = id.to_lowercase();
    // `Some(false)` is a HARD REJECT on a pinned model (400), so a pattern here must not be
    // broader than the vision allowlist it complements. Substring matching made three of these
    // swallow real vision models: bare "gpt-4" also matched `gpt-4.5-preview` and
    // `gpt-4-vision-preview`; "llama3.2" matched Ollama's dotless `llama3.2-vision:11b`; and
    // "qwen2.5-" matched the multimodal `qwen2.5-omni-7b`. Each was rejected with "does not
    // support image input" while in fact supporting it.
    //
    // Anything containing a vision marker is therefore never classified text-only — when the id
    // says it takes images and the allowlist simply has not heard of it, `None` (let the provider
    // decide) is the honest answer, not a hard reject.
    const VISION_MARKERS: &[&str] = &["vision", "-vl", "omni", "multimodal"];
    if VISION_MARKERS.iter().any(|marker| m.contains(marker)) {
        return None;
    }
    const TEXT_ONLY_PATTERNS: &[&str] = &[
        "gpt-3",
        // Exact legacy gpt-4 ids only. `gpt-4o`/`-turbo`/`.1` return vision above, but `gpt-4.5`
        // and future `gpt-4.x` must fall through to None rather than be rejected outright.
        "gpt-4-0",
        "gpt-4-1106",
        "gpt-4-32k",
        "claude-2",
        "claude-instant",
        "llama-3.1",
        "llama-3.3",
        "deepseek-v3",
        "mistral-large",
        "davinci",
    ];
    if m.ends_with("gpt-4") || m.ends_with("llama3.2") || m.ends_with("qwen2.5") {
        return Some(false);
    }
    TEXT_ONLY_PATTERNS
        .iter()
        .any(|pattern| m.contains(pattern))
        .then_some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Some(false)` is a hard 400 on a pinned model, so over-matching here rejects working
    /// models. These four were all wrongly rejected by substring patterns.
    #[test]
    fn known_vision_models_are_never_classified_text_only() {
        for id in [
            "openai::gpt-4.5-preview",
            "openai::gpt-4-vision-preview",
            "ollama::llama3.2-vision:11b",
            "qwen2.5-omni-7b",
        ] {
            assert_ne!(
                vision_capability(id),
                Some(false),
                "{id} must not be rejected"
            );
        }
    }

    /// The genuinely text-only families must still be caught, or the pin check stops protecting.
    #[test]
    fn legacy_text_only_models_are_still_rejected() {
        for id in [
            "openai::gpt-4",
            "anthropic::claude-2.1",
            "groq::llama-3.3-70b",
        ] {
            assert_eq!(vision_capability(id), Some(false), "{id} must be text-only");
        }
        assert_eq!(vision_capability("openai::gpt-4o"), Some(true));
        assert_eq!(vision_capability("openrouter::stealth/ox-alpha"), None);
    }
}
