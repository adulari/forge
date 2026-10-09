//! `auto` temper on the CLI-bridge path. `mcp-serve` has no `Session`, so an Unknown shell command
//! (see `forge_core::permission::AutoVerdict`) would otherwise always be relayed to the user as a
//! prompt. Here it first goes through the same bounded classifier the in-process session uses; an
//! allow skips the prompt, anything else (a no-key box, a timeout, an unusable answer) falls
//! through to the existing relay, so the failure mode is the prompt the user had before.

use super::*;

use forge_core::auto_classifier::{
    bridge_classifier_models, run_classifier, Classified, ClassifierCache, ClassifierInput,
    CLASSIFIER_MAX_SECS,
};
use forge_core::permission::AutoVerdict;
use forge_provider::Provider;

/// A CLI-bridge model starts a whole agent process, so it gets longer than an API model; the
/// alternative is the user's prompt, which can take minutes.
const BRIDGE_MODEL_WAIT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Default)]
pub(super) struct AutoClassify {
    /// Per bridge process, which lives as long as the conversation it serves.
    cache: std::sync::Mutex<ClassifierCache>,
    provider: tokio::sync::OnceCell<Arc<dyn Provider>>,
}

impl AutoClassify {
    /// Upgrade `(Ask, Some(Unknown))` to Allow when the classifier vouches for the command; every
    /// other outcome is returned unchanged.
    pub(super) async fn settle(
        &self,
        decision: PermissionDecision,
        verdict: Option<AutoVerdict>,
        args: &Value,
        config: &Config,
        store: &Store,
    ) -> PermissionDecision {
        let unknown = decision == PermissionDecision::Ask && verdict == Some(AutoVerdict::Unknown);
        if !unknown || !config.permissions.auto_classifier {
            return decision;
        }
        let command = args.get("command").and_then(Value::as_str).unwrap_or("");
        let provider = self
            .provider
            .get_or_init(|| async {
                crate::build_provider_and_router(
                    config,
                    false,
                    None,
                    None,
                    Default::default(),
                    Default::default(),
                )
                .0
            })
            .await;
        let mut models = bridge_classifier_models(config, store);
        // The user's own plan, when the parent named a cheap model on it: slower than an API model,
        // but still quicker than waiting on a prompt.
        if let Some(own) = std::env::var(forge_provider::BRIDGE_CLASSIFIER_MODEL_ENV)
            .ok()
            .filter(|m| !m.is_empty() && !models.contains(m))
        {
            models.push(own);
        }
        let session = std::env::var(forge_core::snapshot::ENV_SESSION).unwrap_or_default();
        let cooldown = std::time::Duration::from_secs(config.mesh.failover_cooldown_secs);
        match self
            .classify(&**provider, store, &session, &models, cooldown, command)
            .await
        {
            Classified::Allow(_) => PermissionDecision::Allow,
            Classified::Ask(_) => decision,
        }
    }

    /// Cached verdict for `command`, else the first model in `models` that answers. No model, or
    /// none that answers, is an Ask; only answered verdicts are cached.
    async fn classify(
        &self,
        provider: &dyn Provider,
        store: &Store,
        session: &str,
        models: &[String],
        cooldown: std::time::Duration,
        command: &str,
    ) -> Classified {
        if let Some(hit) = self.cache.lock().unwrap().get(command).cloned() {
            return hit;
        }
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut last = "no usable classifier model".to_string();
        for model in models {
            let input = ClassifierInput {
                command,
                cwd: &cwd,
                workspace: Some(&cwd),
                request: "",
            };
            let max_wait = if forge_provider::is_cli_bridge(model) {
                BRIDGE_MODEL_WAIT
            } else {
                std::time::Duration::from_secs(CLASSIFIER_MAX_SECS)
            };
            match run_classifier(provider, store, session, model, cooldown, max_wait, input).await {
                Ok(answered) => {
                    self.cache.lock().unwrap().insert(command, answered.clone());
                    return answered;
                }
                Err(why) => last = why,
            }
        }
        Classified::Ask(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Reply {
        calls: AtomicUsize,
        text: &'static str,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl Provider for Reply {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[forge_provider::ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(forge_provider::ProviderError::Unavailable("down".into()));
            }
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: self.text.into(),
                tool_calls: Vec::new(),
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    const COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);

    fn provider(text: &'static str, fail: bool) -> Reply {
        Reply {
            calls: AtomicUsize::new(0),
            text,
            fail,
        }
    }

    fn models() -> Vec<String> {
        vec!["groq::a".into(), "groq::b".into()]
    }

    #[tokio::test]
    async fn an_allow_is_cached_per_normalized_command() {
        let store = Store::open_in_memory().unwrap();
        let p = provider(r#"{"verdict":"allow","reason":"echoes"}"#, false);
        let auto = AutoClassify::default();
        for command in ["./build.sh", "./build.sh  "] {
            let v = auto
                .classify(&p, &store, "s", &models(), COOLDOWN, command)
                .await;
            assert_eq!(v, Classified::Allow("echoes".into()));
        }
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failures_ask_try_the_next_model_and_are_not_cached() {
        let store = Store::open_in_memory().unwrap();
        let down = provider("", true);
        let auto = AutoClassify::default();
        for _ in 0..2 {
            let v = auto
                .classify(&down, &store, "s", &models(), COOLDOWN, "./x.sh")
                .await;
            assert!(matches!(v, Classified::Ask(_)));
        }
        assert_eq!(down.calls.load(Ordering::SeqCst), 4, "both models, twice");
        let junk = provider("probably fine", false);
        let v = auto
            .classify(&junk, &store, "s", &models(), COOLDOWN, "./x.sh")
            .await;
        assert!(matches!(v, Classified::Ask(_)));
        let none = auto
            .classify(&junk, &store, "s", &[], COOLDOWN, "./y.sh")
            .await;
        assert_eq!(none, Classified::Ask("no usable classifier model".into()));
    }

    #[tokio::test]
    async fn only_an_unknown_ask_is_ever_upgraded() {
        let store = Store::open_in_memory().unwrap();
        let config = Config::default();
        let auto = AutoClassify::default();
        let args = serde_json::json!({ "command": "./build.sh" });
        for (decision, verdict) in [
            (
                PermissionDecision::Ask,
                Some(AutoVerdict::Risky("rm -rf".into())),
            ),
            (PermissionDecision::Ask, None),
            (PermissionDecision::Deny, Some(AutoVerdict::Unknown)),
            (PermissionDecision::Allow, Some(AutoVerdict::Safe)),
        ] {
            let settled = auto.settle(decision, verdict, &args, &config, &store).await;
            assert_eq!(settled, decision);
        }
        let mut off = Config::default();
        off.permissions.auto_classifier = false;
        let settled = auto
            .settle(
                PermissionDecision::Ask,
                Some(AutoVerdict::Unknown),
                &args,
                &off,
                &store,
            )
            .await;
        assert_eq!(settled, PermissionDecision::Ask);
    }
}
