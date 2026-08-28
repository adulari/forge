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
    const TEXT_ONLY_PATTERNS: &[&str] = &[
        "gpt-3",
        "gpt-4", // gpt-4o, gpt-4-turbo and gpt-4.1 return above as vision-capable.
        "claude-2",
        "claude-instant",
        "llama-3.1",
        "llama-3.3",
        "llama3.2",
        "deepseek-v3",
        "qwen2.5-",
        "mistral-large",
        "davinci",
    ];
    TEXT_ONLY_PATTERNS
        .iter()
        .any(|pattern| m.contains(pattern))
        .then_some(false)
}
