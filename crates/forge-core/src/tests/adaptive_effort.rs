//! Adaptive effort end to end: the request after a successful read-only round goes out at low
//! effort, a write or a step with no tool call restores the configured effort, and a provider not
//! opted in is never touched.

use super::*;

#[derive(Clone, Copy)]
enum Step {
    Read,
    Write,
    /// No text and no tool call: the loop nudges the model and asks again.
    Empty,
}

/// Plays `script` one step per main-loop request, then answers; records the effort of each.
#[derive(Default)]
struct EffortRecorder {
    script: Vec<Step>,
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
        let step = self.script.get(n).copied();
        let tool_calls = match step {
            Some(Step::Read) => vec![call("read_file", serde_json::json!({"path": "a.txt"}))],
            Some(Step::Write) => vec![call(
                "write_file",
                serde_json::json!({"path": "b.txt", "content": "x"}),
            )],
            Some(Step::Empty) | None => Vec::new(),
        };
        Ok(forge_provider::ModelResponse {
            content: if step.is_none() {
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

async fn run(providers: &[&str], script: Vec<Step>) -> Vec<Option<forge_types::EffortLevel>> {
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
    let provider = Arc::new(EffortRecorder {
        script,
        ..EffortRecorder::default()
    });
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
    let efforts = run(&["ollama"], vec![Step::Read, Step::Read, Step::Write]).await;
    assert_eq!(
        efforts.get(..4),
        Some(&[Some(High), Some(Low), Some(Low), Some(High)][..]),
        "{efforts:?}"
    );
}

#[tokio::test]
async fn a_provider_not_opted_in_keeps_its_effort() {
    let efforts = run(&["openai"], vec![Step::Read, Step::Read, Step::Write]).await;
    assert!(!efforts.is_empty());
    assert!(
        efforts
            .iter()
            .all(|e| *e == Some(forge_types::EffortLevel::High)),
        "{efforts:?}"
    );
}

#[tokio::test]
async fn a_step_without_a_tool_call_restores_the_effort() {
    use forge_types::EffortLevel::{High, Low};
    let efforts = run(&["ollama"], vec![Step::Read, Step::Empty]).await;
    assert_eq!(
        efforts.get(..3),
        Some(&[Some(High), Some(Low), Some(High)][..]),
        "the request after the empty-reply nudge must not stay low: {efforts:?}"
    );
}
