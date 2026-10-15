//! Vision (image-input) capability for a model id.
//!
//! Split out of `catalog.rs` rather than raising that file's architecture-size ratchet: the
//! functions here are one cohesive concern (does this id accept images?) with their own pattern
//! tables, and `catalog.rs` was already the largest implementation owner in the crate.
//!
//! Resolution order: (1) the provider cannot deliver images at all, (2) catalog DATA — the input
//! modalities OpenRouter and models.dev publish, fetched with the context-window sweep and cached
//! on disk (see [`install_modalities`]), (3) the name heuristic below. `catalog` re-exports the two
//! entry points, so `catalog::supports_vision` keeps working.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

const CACHE_FILE: &str = "vision-modalities.json";

/// Provider namespaces whose transport has no way to carry an image, whatever the model behind
/// them can do. Antigravity (`agy -p <prompt>`) takes one text argument. The claude/codex bridges
/// are NOT here: they forward images over their CLI's own channel (`cli_provider/images.rs`).
const TEXT_ONLY_TRANSPORTS: &[&str] = &["agy-cli"];

fn transport_delivers_images(id: &str) -> bool {
    id.split_once("::")
        .is_none_or(|(provider, _)| !TEXT_ONLY_TRANSPORTS.contains(&provider))
}

fn registry() -> &'static RwLock<Arc<HashMap<String, bool>>> {
    static REGISTRY: OnceLock<RwLock<Arc<HashMap<String, bool>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Arc::new(load_cache())))
}

fn cache_path() -> Option<std::path::PathBuf> {
    forge_config::data_dir().map(|d| d.join(CACHE_FILE))
}

// Unit tests must not read the developer's real cache: a populated one would make heuristic
// assertions depend on whichever catalog was last fetched.
fn load_cache() -> HashMap<String, bool> {
    if cfg!(test) {
        return HashMap::new();
    }
    cache_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Replace the catalog-derived modality table (full ids and `~basename` keys, lowercase) and
/// persist it so a restart — or a process that never ran the network sweep — still has it. An
/// empty table is ignored: a failed fetch must not erase the last good one.
pub fn install_modalities(table: HashMap<String, bool>) {
    if table.is_empty() {
        return;
    }
    if !cfg!(test) {
        if let Some(path) = cache_path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(bytes) = serde_json::to_vec(&table) {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
        }
    }
    *registry()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(table);
}

/// The key a model is indexed under besides its full id: lowercase, no namespace, no vendor path,
/// no `:tag`/`:free` suffix.
pub fn basename_key(id: &str) -> String {
    let model = id.rsplit_once("::").map_or(id, |(_, m)| m);
    let model = model.rsplit('/').next().unwrap_or(model);
    let model = model.split(':').next().unwrap_or(model);
    format!("~{}", model.to_lowercase())
}

fn lookup(table: &HashMap<String, bool>, id: &str) -> Option<bool> {
    table
        .get(&id.to_lowercase())
        .or_else(|| table.get(&basename_key(id)))
        .copied()
}

fn data_vision(id: &str) -> Option<bool> {
    let table = registry()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    lookup(&table, id)
}

/// Whether a model id is known to accept image input AND its provider can deliver one. Catalog
/// data wins; otherwise this is a name-heuristic allowlist — like [`crate::catalog::is_routable`]
/// and the capability priors in `capability.rs` — because providers don't expose modality
/// uniformly. It exists to route AROUND a turn with image attachments landing on a text-only
/// model: that produces an immediate provider 404 ("No endpoints found that support image
/// input"), not a slow/garbled reply, so this is a positive allowlist rather than a block-list.
/// Documented in docs/features/mesh-routing.md.
pub fn supports_vision(id: &str) -> bool {
    if !transport_delivers_images(id) {
        return false;
    }
    data_vision(id).unwrap_or_else(|| heuristic_vision(id))
}

fn heuristic_vision(id: &str) -> bool {
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
        "claude-5",
        "opus",
        "sonnet",
        "haiku",
        // Google: every Gemini model (Pro/Flash/Flash-Lite) accepts image input, and so do the
        // open Gemma 3+ models above 1B.
        "gemini",
        "gemma-3-4b",
        "gemma-3-12b",
        "gemma-3-27b",
        "gemma-3n",
        "gemma-4",
        // Meta: the vision-tuned Llama 3.2 sizes, and every Llama 4 model (natively multimodal).
        // Plain llama-3.2 text-only sizes (1b/3b, no "-vision" suffix) correctly fall through.
        "llama-3.2-11b-vision",
        "llama-3.2-90b-vision",
        "llama-4",
        // Mistral: the vision-tuned line plus the generations that became multimodal — Small 3.1+,
        // Medium 3+, Large 3+ (2512). Large 2 (2407/2411) is text-only and is excluded by
        // `vision_capability`'s text-only table and by catalog data.
        "pixtral",
        "mistral-small-3.1",
        "mistral-small-3.2",
        "mistral-small-4",
        "mistral-small-latest",
        "mistral-medium",
        "mistral-large-2512",
        "mistral-large-3",
        "mistral-large-4",
        "mistral-large-latest",
        "magistral-medium",
        "magistral-small",
        "ministral-3",
        // Qwen's vision-language line: the explicit "-vl-" tag, and the Qwen3-VL family.
        "-vl-",
        "qwen2.5-vl",
        "qwen3-vl",
        "qwen-vl",
        "qvq",
        // Moonshot: Kimi K2.5 and later are natively multimodal; earlier K2 are text-only.
        "kimi-k2.5",
        "kimi-k2.6",
        "kimi-k2.7",
        "kimi-k3",
        "kimi-vl",
        // Zhipu GLM-V line (the `v` suffix marks the vision models: glm-4v, glm-4.5v, glm-5v-*).
        "glm-4v",
        "glm-4.1v",
        "glm-4.5v",
        "glm-4.6v",
        "glm-5v",
        // Other open vision-language families.
        "deepseek-vl",
        "llava",
        "molmo",
        "internvl",
        "minicpm-v",
        "phi-4-multimodal",
        "phi-3.5-vision",
        "vision",
        // xAI: every Grok model accepts image input.
        "grok",
    ];
    VISION_PATTERNS.iter().any(|p| m.contains(p))
}

/// Best-effort vision capability classification for callers that need to validate an explicit
/// model pin. `Some(false)` is returned only for models whose text-only status is known (catalog
/// data, an image-less transport, or a text-only family); `None` means there is no reliable
/// modality information and the provider should decide. This distinction matters for
/// custom/OpenRouter models whose ids are not covered by the data or heuristic but may still
/// accept images.
pub fn vision_capability(id: &str) -> Option<bool> {
    if !transport_delivers_images(id) {
        return Some(false);
    }
    if let Some(known) = data_vision(id) {
        return Some(known);
    }
    if heuristic_vision(id) {
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
        // Mistral Large 2 only; Large 3+ (2512, `-latest`) is multimodal and caught above.
        "mistral-large-2407",
        "mistral-large-2411",
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

    #[test]
    fn catalog_data_beats_the_name_heuristic_in_both_directions() {
        let table = HashMap::from([
            ("zai::glm-5.3".to_string(), false),
            ("~glm-5.3-flash".to_string(), true),
            ("xai::grok-code-fast-1".to_string(), false),
        ]);
        assert_eq!(lookup(&table, "zai::glm-5.3"), Some(false));
        assert_eq!(lookup(&table, "inferhub::GLM-5.3-Flash"), Some(true));
        assert_eq!(lookup(&table, "xai::grok-code-fast-1"), Some(false));
        assert_eq!(lookup(&table, "inferhub::unlisted"), None);
        assert_eq!(basename_key("openrouter::vendor/Model-X:free"), "~model-x");
    }

    #[test]
    fn installed_data_overrides_heuristic_and_empty_install_is_ignored() {
        install_modalities(HashMap::from([("zz-test::kept".to_string(), true)]));
        install_modalities(HashMap::new());
        assert!(supports_vision("zz-test::kept"));
        assert_eq!(vision_capability("zz-test::kept"), Some(true));
    }

    #[test]
    fn transport_without_an_image_channel_never_reports_vision() {
        assert!(!supports_vision("agy-cli::gemini-3-pro"));
        assert_eq!(vision_capability("agy-cli::claude-sonnet-4-5"), Some(false));
        assert!(supports_vision("claude-cli::sonnet"));
        assert!(supports_vision("codex-cli::gpt-5.6-luna"));
    }

    #[test]
    fn heuristic_covers_the_gaps_the_old_allowlist_had() {
        for id in [
            "kimi::kimi-k3",
            "bedrock::global.moonshotai.kimi-k3",
            "zai::glm-4.5v",
            "zai::glm-5v-turbo",
            "deepseek::deepseek-vl2",
            "mistral::mistral-small-latest",
            "mistral::mistral-medium-3",
            "mistral::mistral-large-2512",
            "openrouter::qwen/qwen-vl-max",
            "ollama::llava:13b",
            "ollama::llama3.2-vision:11b",
        ] {
            assert!(supports_vision(id), "{id} accepts images");
        }
        assert_ne!(
            vision_capability("mistral::mistral-large-latest"),
            Some(false)
        );
        assert_eq!(
            vision_capability("mistral::mistral-large-2411"),
            Some(false)
        );
        assert!(!supports_vision("kimi::kimi-k2-thinking"));
    }
}
