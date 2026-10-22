//! Learn a pinned model's context window before the session sizes its first request.
//!
//! Context windows live in the store, written by the catalog sweep (`context_windows.rs`). A store
//! that has never been swept — a scratch `FORGE_DB`, which is how `forge bench tasks` isolates each
//! task — knows no window at all, so a pinned `inferhub::cb/deepseek-v4.1-flash` (1,048,576 tokens)
//! was sized from the 32k conservative floor: auto-compaction fired at 25.6k tokens, every task on a
//! 390k-line repository compacted three to five times (13 messages → 7, again and again), tool
//! output was clipped to make room, and the window trim dropped messages the model still needed.
//! The cached catalog the run routes from carries no windows, and the background refresh that
//! would have swept them only runs when that catalog is stale.

use super::*;

/// Ceiling on the inline lookup. Every probe in the sweep runs concurrently under a 4 s timeout,
/// so this only bounds a pathological resolver.
const WINDOW_LOOKUP_BUDGET: std::time::Duration = std::time::Duration::from_secs(6);

/// Whether anything short of the conservative floor knows `model`'s window. Mirrors the order of
/// `Session::base_context_window`.
fn window_known(
    config: &forge_config::Config,
    windows: &std::collections::HashMap<String, u32>,
    model: &str,
) -> bool {
    config
        .mesh
        .context_windows
        .get(model)
        .is_some_and(|w| *w > 0)
        || forge_mesh::pricing::authoritative_context_limit(model).is_some()
        || windows.get(model).is_some_and(|w| *w > 0)
        || forge_mesh::pricing::context_limit(model).is_some()
        || forge_mesh::pricing::cross_namespace_window(model, windows).is_some()
}

/// The members of `pin` whose window nothing on this machine knows.
fn pins_without_window(
    config: &forge_config::Config,
    windows: &std::collections::HashMap<String, u32>,
    pin: &str,
) -> Vec<String> {
    pin_members(pin)
        .into_iter()
        .filter(|model| !window_known(config, windows, model))
        .collect()
}

/// Fetch the windows of pinned models nothing knows yet, bounded by [`WINDOW_LOOKUP_BUDGET`].
/// Returns the members still unknown afterwards, which run on the conservative floor.
pub(super) async fn learn_pinned_windows(
    store: &Store,
    config: &forge_config::Config,
    pin: Option<&str>,
    mock: bool,
) -> Vec<String> {
    let Some(pin) = pin.filter(|_| !mock) else {
        return Vec::new();
    };
    let missing = pins_without_window(config, &store.all_model_contexts().unwrap_or_default(), pin);
    if missing.is_empty() {
        return missing;
    }
    let lookup = crate::context_windows::fetch_and_persist(&missing);
    if tokio::time::timeout(WINDOW_LOOKUP_BUDGET, lookup)
        .await
        .is_err()
    {
        tracing::warn!(
            ?missing,
            "context-window lookup for the pinned model timed out"
        );
    }
    pins_without_window(config, &store.all_model_contexts().unwrap_or_default(), pin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_store_knows_no_gateway_window_and_a_swept_one_does() {
        let config = forge_config::Config::default();
        let pin = "inferhub::cb/deepseek-v4.1-flash";
        let mut windows = std::collections::HashMap::new();
        assert_eq!(pins_without_window(&config, &windows, pin), [pin]);

        windows.insert("inferhub::cb/deepseek-v4.1-flash".to_string(), 1_048_576);
        assert!(pins_without_window(&config, &windows, pin).is_empty());
    }

    #[test]
    fn a_window_known_by_any_other_source_needs_no_lookup() {
        let mut config = forge_config::Config::default();
        let windows = std::collections::HashMap::from([(
            "openrouter::deepseek/deepseek-v4.1-flash".to_string(),
            1_048_576,
        )]);
        assert!(
            pins_without_window(&config, &windows, "inferhub::cb/deepseek-v4.1-flash").is_empty(),
            "the same model under another namespace lends its window"
        );
        assert!(pins_without_window(&config, &windows, "claude-cli::sonnet").is_empty());
        config
            .mesh
            .context_windows
            .insert("ollama::qwen3:4b".to_string(), 40_960);
        assert!(pins_without_window(&config, &windows, "ollama::qwen3:4b").is_empty());
        assert_eq!(
            pins_without_window(&config, &windows, "ollama::qwen3:4b,ollama::other"),
            ["ollama::other"]
        );
    }

    #[tokio::test]
    async fn a_mock_or_unpinned_run_never_looks_anything_up() {
        let store = Store::open_in_memory().unwrap();
        let config = forge_config::Config::default();
        assert!(learn_pinned_windows(&store, &config, None, false)
            .await
            .is_empty());
        assert!(
            learn_pinned_windows(&store, &config, Some("inferhub::x"), true)
                .await
                .is_empty()
        );
    }
}
