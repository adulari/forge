//! `[privacy] zero_retention` at the session level: a retaining model is refused before any
//! request leaves, honestly and without benching it, and allowed requests carry the opt-out.

use super::*;

#[derive(Default)]
struct OptsRecorder {
    calls: Mutex<Vec<(String, bool)>>,
}

#[async_trait::async_trait]
impl Provider for OptsRecorder {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        unreachable!("the guard always forwards complete_with")
    }

    async fn complete_with(
        &self,
        model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        opts: &CompletionOptions,
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.calls
            .lock()
            .unwrap()
            .push((model.to_string(), opts.deny_data_collection));
        Ok(forge_provider::ModelResponse {
            content: "Done.".into(),
            ..Default::default()
        })
    }
}

fn zdr_session(
    dir: &std::path::Path,
    provider: Arc<OptsRecorder>,
    model: &str,
) -> (Arc<Store>, Session, CapturePresenter) {
    let mut config = Config::default();
    config.privacy.zero_retention = true;
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let session = Session::start(
        Arc::clone(&store),
        provider,
        Arc::new(FixedRouter {
            model: model.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(capture.clone()),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    (store, session, capture)
}

#[tokio::test]
async fn a_retaining_route_fails_the_turn_honestly_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(OptsRecorder::default());
    let (_store, mut session, _capture) =
        zdr_session(dir.path(), Arc::clone(&provider), "claude-cli::opus");
    let outcome = session.run_turn("say hi").await.unwrap();
    assert!(
        provider.calls.lock().unwrap().is_empty(),
        "nothing may be sent"
    );
    assert!(
        outcome
            .text
            .contains("zero-retention mode refuses claude-cli::opus"),
        "{}",
        outcome.text
    );
}

#[tokio::test]
async fn an_allowed_route_runs_and_carries_the_opt_out() {
    if std::env::var_os("OLLAMA_HOST").is_some() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(OptsRecorder::default());
    let (_store, mut session, _capture) =
        zdr_session(dir.path(), Arc::clone(&provider), "ollama::llama3.2");
    session.run_turn("say hi").await.unwrap();
    let calls = provider.calls.lock().unwrap().clone();
    assert!(!calls.is_empty());
    assert!(
        calls
            .iter()
            .all(|(model, deny)| model == "ollama::llama3.2" && *deny),
        "{calls:?}"
    );
}

#[tokio::test]
async fn the_guard_refuses_direct_calls_without_benching_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(OptsRecorder::default());
    let (store, session, _capture) =
        zdr_session(dir.path(), Arc::clone(&provider), "ollama::llama3.2");
    let error = session
        .provider
        .complete_with(
            "groq::llama-3.1-8b-instant",
            &[Message::user("secret")],
            &[],
            &CompletionOptions::default(),
            &mut |_| {},
        )
        .await
        .unwrap_err();
    assert!(crate::zero_retention::is_refusal(&error), "{error}");
    assert!(provider.calls.lock().unwrap().is_empty());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    crate::model_failure_record::record_model_failure_in(
        &store,
        "groq::llama-3.1-8b-instant",
        &error,
        std::time::Duration::from_secs(60),
    );
    assert!(
        !store
            .benched_models(now)
            .unwrap()
            .is_benched("groq::llama-3.1-8b-instant"),
        "a policy refusal says nothing about the model and must not bench it for other sessions"
    );
}

#[test]
fn side_calls_under_a_pin_skip_retaining_candidates() {
    if std::env::var_os("OLLAMA_HOST").is_some() {
        return;
    }
    let mut config = Config::default();
    config.privacy.zero_retention = true;
    config.mesh.models.insert(
        TaskTier::Trivial.as_str().to_string(),
        forge_config::OneOrMany::Many(vec!["ollama::qwen3:4b".into()]),
    );
    let routed = forge_mesh::RoutingDecision {
        tier: TaskTier::Trivial,
        model: "openai::gpt-5.4".into(),
        rationale: String::new(),
        fallbacks: vec![],
        pinned: true,
        unroutable: false,
        effort: None,
    };
    let catalog = ModelCatalog::new(vec!["ollama::qwen3:4b".into()]);
    let pick = |config: &Config| {
        crate::btw_policy::side_call_model(
            config,
            &forge_types::ModelHealth::default(),
            None,
            Some(&catalog),
            &routed,
        )
    };
    assert_eq!(pick(&config), "ollama::qwen3:4b");
    config.privacy.retains.insert("ollama".into(), true);
    assert_eq!(
        pick(&config),
        "openai::gpt-5.4",
        "a candidate marked retaining is never chosen; the pin itself is then refused by the guard"
    );
}
