//! Automatic retirement of provider-wide exclusions.
//!
//! An auth exclusion is a guess about a credential, made from one failed call. Nothing in the
//! runtime ever re-tested that guess: `forge models --probe` and `forge auth` were the only things
//! that cleared a `__forge_provider__::<name>` row, so a wrong verdict silently removed an entire
//! subscription from routing for its whole window while the bridge answered a manual `forge run`
//! in four seconds.
//!
//! The probe machinery already exists and costs nothing on a subscription bridge. This runs it
//! automatically, in the background, only when a provider-wide exclusion is actually in force.

use forge_provider::Provider;
use forge_store::Store;
use std::sync::Arc;

/// Wall-clock budget for one liveness ping. Matches `probe_models`.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Re-test each currently-excluded provider in the background and clear the ones that answer.
///
/// Deliberately one-sided: a passing probe retires the exclusion, a failing one changes nothing.
/// Re-benching here would let a provider that is repeatedly probed have its window extended
/// indefinitely, turning a bounded exclusion back into an open-ended one.
pub(crate) fn retire_verified_provider_exclusions(
    store: &Arc<Store>,
    config: &forge_config::Config,
    excluded: Vec<(String, i64, String)>,
) {
    if excluded.is_empty() {
        return;
    }
    let Some(catalog) = super::load_cached_catalog() else {
        return;
    };
    let targets: Vec<(String, String)> = excluded
        .into_iter()
        .filter_map(|(provider, _, _)| {
            let model = probe_target(&catalog, &provider)?;
            Some((provider, model))
        })
        .collect();
    if targets.is_empty() {
        return;
    }
    let store = Arc::clone(store);
    let config = config.clone();
    tokio::spawn(async move {
        for (provider, model) in targets {
            if probe_answers(&config, &model).await {
                let _ = store.clear_provider_health(&provider);
                let _ = store.clear_model_health(&model);
            }
        }
    });
}

/// The model to ping for `provider`: its first catalog alias. Any alias proves the credential,
/// which is the only thing a provider-scope exclusion claims.
fn probe_target(catalog: &forge_mesh::ModelCatalog, provider: &str) -> Option<String> {
    catalog
        .models()
        .iter()
        .find(|model| forge_config::provider_of(model) == provider)
        .cloned()
}

/// One bounded, tool-bearing ping. Mirrors `probe_models`: a no-tool ping would pass on models
/// that cannot actually serve a Forge turn.
async fn probe_answers(config: &forge_config::Config, model: &str) -> bool {
    let harness = config.mesh.bridge_mode == forge_config::BridgeMode::Harness;
    let provider = super::DispatchProvider::new(harness)
        .with_max_output_tokens(config.mesh.effective_max_output_tokens());
    let ping = [forge_types::Message::user("ping")];
    let probe_tool = [forge_provider::ToolSpec {
        name: "noop".to_string(),
        description: "A no-op used to verify the model accepts tool calls.".to_string(),
        schema: serde_json::json!({"type": "object", "properties": {}}),
    }];
    let mut sink = |_: forge_provider::StreamEvent| {};
    matches!(
        tokio::time::timeout(
            PROBE_TIMEOUT,
            provider.complete(model, &ping, &probe_tool, &mut sink),
        )
        .await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_target_picks_an_alias_of_the_named_provider_only() {
        let catalog = forge_mesh::ModelCatalog::new(vec![
            "groq::openai/gpt-oss-20b".to_string(),
            "claude-cli::sonnet".to_string(),
            "claude-cli::opus".to_string(),
        ]);
        assert_eq!(
            probe_target(&catalog, "claude-cli").as_deref(),
            Some("claude-cli::sonnet")
        );
        assert_eq!(probe_target(&catalog, "codex-cli"), None);
    }
}
