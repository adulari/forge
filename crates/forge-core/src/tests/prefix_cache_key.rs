//! The main loop's prompt-cache key: shared across sessions with the same fixed prefix when the
//! provider treats it as a routing hint, chosen once per session, and never changed mid-session.

use super::*;

#[derive(Default)]
struct KeyRecorder {
    keys: Mutex<Vec<(String, Option<String>)>>,
}

#[async_trait::async_trait]
impl Provider for KeyRecorder {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        unreachable!("the main loop calls complete_with")
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        opts: &CompletionOptions,
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if messages
            .first()
            .is_some_and(|m| m.content.starts_with("You are"))
        {
            self.keys
                .lock()
                .unwrap()
                .push((model.to_string(), opts.prompt_cache_key.clone()));
        }
        Ok(forge_provider::ModelResponse {
            content: "Done.".into(),
            ..Default::default()
        })
    }
}

fn session(dir: &std::path::Path, provider: Arc<KeyRecorder>, config: Config) -> Session {
    let mut config = config;
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(FixedRouter {
            model: "ollama::llama3.2".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(HeadlessPresenter::new(false)),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn sessions_with_the_same_prefix_share_one_key_that_never_moves() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(KeyRecorder::default());
    let mut first = session(dir.path(), Arc::clone(&provider), Config::default());
    first.run_turn("say hi").await.unwrap();
    first.run_turn("say hi again").await.unwrap();
    let mut second = session(dir.path(), Arc::clone(&provider), Config::default());
    second.run_turn("say hi").await.unwrap();

    let keys = provider.keys.lock().unwrap().clone();
    assert!(keys.len() >= 3, "{keys:?}");
    let key = keys[0].1.clone().expect("the main loop always sends a key");
    assert!(key.starts_with("forge-prefix-"), "{key}");
    assert!(
        keys.iter().all(|(_, k)| k.as_deref() == Some(key.as_str())),
        "both sessions, every request: {keys:?}"
    );
    assert_ne!(first.id, second.id);
}

#[tokio::test]
async fn the_flag_off_keeps_the_per_session_key() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(KeyRecorder::default());
    let mut config = Config::default();
    config.mesh.cross_session_cache_key = false;
    let mut s = session(dir.path(), Arc::clone(&provider), config);
    s.run_turn("say hi").await.unwrap();
    let keys = provider.keys.lock().unwrap().clone();
    assert!(!keys.is_empty());
    assert!(
        keys.iter()
            .all(|(_, k)| k.as_deref() == Some(s.id.as_str())),
        "{keys:?}"
    );
}

#[test]
fn the_first_request_decides_and_later_ones_inherit_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(
        dir.path(),
        Arc::new(KeyRecorder::default()),
        Config::default(),
    );
    let tools = |name: &str| {
        vec![ToolSpec {
            name: name.into(),
            description: String::new(),
            schema: serde_json::json!({}),
        }]
    };
    let shared = s.main_loop_cache_key("openai::gpt-5.4", &tools("a"), "sid");
    assert!(shared.starts_with("forge-prefix-"));
    assert_eq!(
        s.main_loop_cache_key("openai::gpt-5.5", &tools("b"), "sid"),
        shared
    );
    assert_eq!(
        s.main_loop_cache_key("nvidia::glm", &tools("a"), "sid"),
        shared
    );

    let mut nim_first = session(
        dir.path(),
        Arc::new(KeyRecorder::default()),
        Config::default(),
    );
    assert_eq!(
        nim_first.main_loop_cache_key("nvidia::glm", &[], "sid"),
        "sid"
    );
    assert_eq!(
        nim_first.main_loop_cache_key("openai::gpt-5.4", &[], "sid"),
        "sid",
        "a failover onto a key-sharing provider must not switch keys mid-session"
    );
}

#[tokio::test]
async fn new_and_resume_pick_the_key_again_for_the_session_they_switch_to() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(KeyRecorder::default());
    let mut config = Config::default();
    config.mesh.cross_session_cache_key = false;
    let mut s = session(dir.path(), Arc::clone(&provider), config);
    s.run_turn("say hi").await.unwrap();
    let first_id = s.id.clone();
    s.reset_fresh(dir.path().to_str().unwrap()).unwrap();
    s.run_turn("say hi").await.unwrap();
    let fresh_id = s.id.clone();
    s.reset_resumed(&first_id).unwrap();
    s.run_turn("say hi again").await.unwrap();

    let keys: Vec<Option<String>> = provider
        .keys
        .lock()
        .unwrap()
        .iter()
        .map(|(_, k)| k.clone())
        .collect();
    assert_ne!(first_id, fresh_id);
    assert_eq!(
        keys.first().cloned().flatten().as_deref(),
        Some(first_id.as_str())
    );
    assert!(keys.contains(&Some(fresh_id.clone())), "{keys:?}");
    assert_eq!(
        keys.last().cloned().flatten().as_deref(),
        Some(first_id.as_str())
    );
}
