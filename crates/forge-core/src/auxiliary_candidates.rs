//! Which configured trivial-tier models are worth trying for an optional side call.

use forge_mesh::ModelCatalog;
use forge_types::ModelHealth;

/// Providers that authenticate without an API key but are still not a safe default: they are
/// local servers that may simply not be running, or an external agent that owns its own auth.
fn is_keyless_local(model: &str) -> bool {
    let provider = forge_config::provider_of(model);
    !provider.is_empty()
        && forge_config::provider_key_env_var(provider).is_none()
        && !forge_provider::is_cli_bridge(model)
        && !provider.ends_with("-oauth")
}

/// Whether `model` may serve a recap/suggestion/memory/diagnosis side call.
///
/// A benched model is skipped, as the router would skip it. A keyless local server (ollama,
/// llama.cpp, LM Studio, a custom keyless endpoint) passes `has_api_key` whether or not it is
/// running, so it must additionally appear in this process's discovered catalog: discovery
/// already probed it, and one that failed to answer is absent. Without a catalog (mock/offline)
/// nothing vouches for a local server, so it is not used.
pub(crate) fn usable_for_side_call(
    model: &str,
    health: &ModelHealth,
    catalog: Option<&ModelCatalog>,
) -> bool {
    if health.is_benched(model) {
        return false;
    }
    if !is_keyless_local(model) {
        return true;
    }
    catalog.is_some_and(|c| c.models().iter().any(|m| m == model))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(models: &[&str]) -> ModelCatalog {
        ModelCatalog::new(models.iter().map(|m| m.to_string()).collect())
    }

    #[test]
    fn local_server_needs_a_live_catalog_entry() {
        let health = ModelHealth::default();
        let live = catalog(&["ollama::qwen3:4b"]);
        let dead = catalog(&["groq::llama-3.1-8b-instant"]);
        assert!(usable_for_side_call(
            "ollama::qwen3:4b",
            &health,
            Some(&live)
        ));
        assert!(!usable_for_side_call(
            "ollama::qwen3:4b",
            &health,
            Some(&dead)
        ));
        assert!(!usable_for_side_call("ollama::qwen3:4b", &health, None));
    }

    #[test]
    fn keyed_provider_does_not_need_the_catalog() {
        let health = ModelHealth::default();
        assert!(usable_for_side_call(
            "groq::llama-3.1-8b-instant",
            &health,
            None
        ));
    }

    #[test]
    fn benched_models_and_providers_are_skipped() {
        let health = ModelHealth::new(
            [
                "groq::llama-3.1-8b-instant".to_string(),
                forge_types::provider_bench_key("gemini"),
            ]
            .into_iter()
            .collect(),
        );
        assert!(!usable_for_side_call(
            "groq::llama-3.1-8b-instant",
            &health,
            None
        ));
        assert!(!usable_for_side_call("gemini::flash", &health, None));
        assert!(usable_for_side_call("groq::other", &health, None));
    }
}
