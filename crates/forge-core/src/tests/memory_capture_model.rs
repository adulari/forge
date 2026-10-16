//! Memory capture is an optional side call: under a session pin it must use the same filtered
//! model as recap/suggestion, not the routed (pinned) model.

use super::*;

struct RecordingProvider {
    memory_models: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl Provider for RecordingProvider {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if messages
            .first()
            .is_some_and(|m| m.content == Session::MEMORY_CAPTURE_SYSTEM)
        {
            self.memory_models.lock().unwrap().push(model.to_string());
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "nothing durable".into(),
            tool_calls: vec![],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn memory_capture_under_a_pin_uses_the_filtered_side_call_model() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.mesh.auto_memory = true;
    config.mesh.models.insert(
        TaskTier::Trivial.as_str().to_string(),
        forge_config::OneOrMany::Many(vec!["ollama::qwen3:4b".into()]),
    );
    let memory_models = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(RecordingProvider {
            memory_models: memory_models.clone(),
        }),
        Arc::new(FixedRouter {
            model: "codex-oauth::gpt-5.6-sol".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(HeadlessPresenter::new(false)),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.pin_model(Some("codex-oauth::gpt-5.6-sol".into()));
    session.set_catalog(Some(ModelCatalog::new(vec!["ollama::qwen3:4b".into()])));

    let handle = session
        .capture_memories("remember this", "noted")
        .await
        .expect("a usable side-call model exists");
    handle.await.unwrap();
    assert_eq!(
        *memory_models.lock().unwrap(),
        vec!["ollama::qwen3:4b".to_string()],
        "the pinned subscription model must not serve the optional memory call"
    );

    session.set_catalog(Some(ModelCatalog::new(vec![])));
    session.pin_model(Some("claude-cli::sonnet".into()));
    assert!(
        session.capture_memories("again", "noted").await.is_none(),
        "a bridge-only pin leaves no suitable side-call model, so memory capture is skipped"
    );
}

/// Answers the memory-capture call with one fact that quotes a credential.
struct LeakyExtractor;

#[async_trait::async_trait]
impl Provider for LeakyExtractor {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let is_capture = messages
            .first()
            .is_some_and(|m| m.content == Session::MEMORY_CAPTURE_SYSTEM);
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if is_capture {
                "preference: always deploy with key sk-abcdefghijklmnopqrstuvwxyz1234567890".into()
            } else {
                "ok".into()
            },
            tool_calls: vec![],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn an_auto_extracted_memory_never_stores_a_credential() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = Config::default();
    config.mesh.auto_memory = true;
    config.mesh.models.insert(
        TaskTier::Trivial.as_str().to_string(),
        forge_config::OneOrMany::Many(vec!["ollama::qwen3:4b".into()]),
    );
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(LeakyExtractor),
        Arc::new(FixedRouter {
            model: "ollama::qwen3:4b".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(HeadlessPresenter::new(false)),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.set_catalog(Some(ModelCatalog::new(vec!["ollama::qwen3:4b".into()])));
    session
        .capture_memories("how do I deploy?", "use the deploy key")
        .await
        .expect("a side-call model exists")
        .await
        .unwrap();
    let stored = store
        .list_memories(&crate::memory_scope_at(dir.path()))
        .unwrap();
    assert_eq!(stored.len(), 1, "the preference is kept: {stored:?}");
    assert!(
        !stored[0].text.contains("sk-abcdef"),
        "the secret must be masked before it is stored: {}",
        stored[0].text
    );
    assert!(stored[0].text.contains("always deploy"));
}
