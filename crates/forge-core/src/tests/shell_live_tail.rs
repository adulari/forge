//! A running foreground shell call reports a rolling output tail to the presenter (UI only); the
//! model-facing result and transcript are exactly what the command produced.

use super::*;

struct ShellOnce {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    command: &'static str,
}

#[async_trait::async_trait]
impl Provider for ShellOnce {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let first = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if first { String::new() } else { "Done.".into() },
            tool_calls: if first {
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "shell".into(),
                    args: serde_json::json!({ "command": self.command }),
                }]
            } else {
                Vec::new()
            },
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

async fn run(command: &'static str) -> (Vec<PresenterEvent>, Vec<Message>) {
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
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(ShellOnce {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            command,
        }),
        Arc::new(FixedRouter {
            model: "ollama::live".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("run it").await.unwrap();
    let events = events.lock().unwrap().clone();
    (events, session.transcript.clone())
}

// Not on the read-only fast path (it redirects to a file), so it runs as a single serial call.
const SLOW: &str = "for i in 1 2 3; do echo line $i; sleep 0.4; done > /dev/stdout";

#[tokio::test]
async fn a_running_shell_call_streams_a_growing_tail_before_its_result() {
    let (events, transcript) = run(SLOW).await;
    let deltas: Vec<&Vec<String>> = events
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::ToolOutputDelta { name, lines, .. } if name == "shell" => Some(lines),
            _ => None,
        })
        .collect();
    assert!(deltas.len() >= 2, "expected live samples, got {deltas:?}");
    assert!(deltas.windows(2).all(|w| w[1].len() >= w[0].len()));
    assert_eq!(deltas.last().unwrap().last().unwrap(), "line 3");
    let last_delta = events
        .iter()
        .rposition(|e| matches!(e, PresenterEvent::ToolOutputDelta { .. }))
        .unwrap();
    let result = events
        .iter()
        .position(|e| matches!(e, PresenterEvent::ToolResult { name, .. } if name == "shell"))
        .unwrap();
    assert!(last_delta < result, "deltas end before the result");

    // The model sees the ordinary result and no trace of the deltas.
    let tool = transcript.iter().find(|m| m.role == Role::Tool).unwrap();
    assert!(
        tool.content.contains("line 1\nline 2\nline 3"),
        "{}",
        tool.content
    );
    assert!(transcript.iter().all(|m| !m.content.contains("elapsed_ms")));
}

#[tokio::test]
async fn the_tail_is_capped_to_the_last_lines() {
    let (events, _) = run("seq 1 500; sleep 0.6").await;
    let max = events
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::ToolOutputDelta { lines, .. } => Some(lines.len()),
            _ => None,
        })
        .max()
        .expect("at least one sample");
    assert!(max <= forge_tools::LIVE_TAIL_LINES);
    let last = events.iter().rev().find_map(|e| match e {
        PresenterEvent::ToolOutputDelta { lines, .. } => Some(lines.clone()),
        _ => None,
    });
    assert_eq!(last.unwrap().last().unwrap(), "500");
}
