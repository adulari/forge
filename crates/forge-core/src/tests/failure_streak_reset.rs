//! An edit → failing build → edit cycle is progress, not "failing the same way".

use super::*;

/// Alternates a failing shell command with a successful write three times, then answers.
struct EditCompileCycle {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for EditCompileCycle {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_types::{new_id, ToolCall, Usage};
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let call = |name: &str, args: serde_json::Value| ToolCall {
            id: new_id(),
            name: name.into(),
            args,
        };
        let (content, tool_calls) = match n {
            0 | 2 | 4 => (
                String::new(),
                vec![call(
                    "shell",
                    serde_json::json!({"command": format!("cat missing-{n}.txt")}),
                )],
            ),
            1 | 3 | 5 => (
                String::new(),
                vec![call(
                    "write_file",
                    serde_json::json!({"path": format!("fix-{n}.txt"), "content": "x"}),
                )],
            ),
            _ => ("Done.".to_string(), Vec::new()),
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn successful_edits_reset_the_failure_streak() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(EditCompileCycle {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();

    session.run_turn("fix the build").await.unwrap();

    let warnings: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(w) => Some(w.clone()),
            _ => None,
        })
        .collect();
    assert!(
        !warnings.iter().any(|w| w.contains("the same way")),
        "edits between failures are a change of approach; warnings: {warnings:?}"
    );
}
