//! Adaptive effort end to end: the request after a successful read-only round goes out at low
//! effort, a write restores the configured effort, and a provider not opted in is never touched.

use super::*;

/// Reads twice, writes once, then answers; records the effort of every main-loop request.
#[derive(Default)]
struct EffortRecorder {
    calls: Mutex<usize>,
    efforts: Mutex<Vec<Option<forge_types::EffortLevel>>>,
}

#[async_trait::async_trait]
impl Provider for EffortRecorder {
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
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        opts: &CompletionOptions,
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_types::{new_id, ToolCall, Usage};
        if !messages
            .first()
            .is_some_and(|m| m.content.starts_with("You are"))
        {
            return Ok(forge_provider::ModelResponse {
                content: "ok".into(),
                ..Default::default()
            });
        }
        self.efforts.lock().unwrap().push(opts.effort);
        let n = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls - 1
        };
        let call = |name: &str, args: serde_json::Value| ToolCall {
            id: new_id(),
            name: name.into(),
            args,
        };
        let tool_calls = match n {
            0 | 1 => vec![call("read_file", serde_json::json!({"path": "a.txt"}))],
            2 => vec![call(
                "write_file",
                serde_json::json!({"path": "b.txt", "content": "x"}),
            )],
            _ => Vec::new(),
        };
        Ok(forge_provider::ModelResponse {
            content: if tool_calls.is_empty() {
                "Done.".into()
            } else {
                String::new()
            },
            tool_calls,
            usage: Usage {
                reasoning_tokens: Some(500),
                ..Usage::default()
            },
            ..Default::default()
        })
    }
}

async fn run(providers: &[&str]) -> Vec<Option<forge_types::EffortLevel>> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.adaptive_effort_providers = providers.iter().map(|p| p.to_string()).collect();
    let provider = Arc::new(EffortRecorder::default());
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::clone(&provider) as Arc<dyn Provider>,
        Arc::new(FixedRouter {
            model: "ollama::llama3.2".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(HeadlessPresenter::new(false)),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.pinned_effort = Some(forge_types::EffortLevel::High);
    session
        .run_turn("look around, then write b.txt")
        .await
        .unwrap();
    let efforts = provider.efforts.lock().unwrap().clone();
    efforts
}

#[tokio::test]
async fn a_readonly_round_lowers_the_next_request_and_a_write_restores_it() {
    use forge_types::EffortLevel::{High, Low};
    let efforts = run(&["ollama"]).await;
    assert_eq!(
        efforts.get(..4),
        Some(&[Some(High), Some(Low), Some(Low), Some(High)][..]),
        "{efforts:?}"
    );
}

#[tokio::test]
async fn a_provider_not_opted_in_keeps_its_effort() {
    let efforts = run(&["openai"]).await;
    assert!(!efforts.is_empty());
    assert!(
        efforts
            .iter()
            .all(|e| *e == Some(forge_types::EffortLevel::High)),
        "{efforts:?}"
    );
}
