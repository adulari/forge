//! The silence guard through a real turn: a model that runs many distinct, successful shell
//! commands without a word gets a progress ask after the twelfth silent step, and the user sees a
//! harness-written progress line, while the turn keeps running.

use super::*;

struct SilentShellRunner {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    steps: usize,
}

#[async_trait::async_trait]
impl Provider for SilentShellRunner {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, tool_calls) = if n < self.steps {
            (
                String::new(),
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "shell".into(),
                    args: serde_json::json!({ "command": format!("echo silent-step-{n}") }),
                }],
            )
        } else {
            ("Done.".to_string(), Vec::new())
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn a_silent_shell_stretch_gets_a_progress_ask_and_a_user_visible_line() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.shell.explain_errors = false;
    let mut session = Session::start(
        store,
        Arc::new(SilentShellRunner {
            calls: Arc::clone(&calls),
            steps: 30,
        }),
        Arc::new(FixedRouter {
            model: "ollama::silent".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("inspect the build").await.unwrap();

    assert!(
        calls.load(std::sync::atomic::Ordering::SeqCst) > 30,
        "the guard must not halt a productive turn"
    );
    let asks = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::System && m.content.contains("without a word to the user"))
        .count();
    assert_eq!(asks, 2, "30 silent steps ask at 12 and 24");
    let progress: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Progress(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(progress.len(), 2, "{progress:?}");
    assert!(progress[0].contains("12 shell"), "{progress:?}");
    assert!(progress[0].contains("echo silent-step-11"), "{progress:?}");
}
