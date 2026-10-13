//! The statusline accounting is live: every model call in a turn reports the session's spend,
//! tokens and context fill, instead of one snapshot when the turn ends.

use super::*;

struct MeteredSteps {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for MeteredSteps {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tool_calls = if n < 2 {
            vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "shell".into(),
                args: serde_json::json!({ "command": format!("echo metered-{n}") }),
            }]
        } else {
            Vec::new()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if n < 2 { String::new() } else { "Done.".into() },
            tool_calls,
            usage: forge_types::Usage {
                input_tokens: 1_000 * (n as u64 + 1),
                output_tokens: 10,
                ..forge_types::Usage::default()
            },
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn every_model_call_reports_spend_tokens_and_context_before_the_turn_ends() {
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
        Arc::new(MeteredSteps {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }),
        Arc::new(FixedRouter {
            model: "ollama::metered".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("run the checks").await.unwrap();

    let events = events.lock().unwrap();
    let done_at = events
        .iter()
        .position(|e| matches!(e, PresenterEvent::Done { .. }))
        .expect("turn finished");
    let live: Vec<(u64, u64, u64)> = events[..done_at]
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Cost {
                session_in,
                session_out,
                context_tokens,
                ..
            } => Some((*session_in, *session_out, *context_tokens)),
            _ => None,
        })
        .collect();
    // Three calls, one live snapshot each, plus the terminal one right before Done.
    assert!(live.len() >= 4, "{live:?}");
    assert_eq!(live[0], (1_000, 10, 1_000), "{live:?}");
    assert_eq!(live[1], (3_000, 20, 2_000), "{live:?}");
    assert_eq!(live[2], (6_000, 30, 3_000), "{live:?}");
}
