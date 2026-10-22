//! What a tool call looks like to the person (and to the model) when it did not go well: a shell
//! command that exited non-zero is drawn as a failure, and a call the person declined says so
//! instead of blaming a policy that never forbade it.

use super::*;

/// One shell call, then a final answer.
struct ShellOnce {
    command: &'static str,
    calls: std::sync::atomic::AtomicUsize,
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
        let step = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tool_calls = if step == 0 {
            vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "shell".into(),
                args: serde_json::json!({ "command": self.command }),
            }]
        } else {
            vec![]
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if step == 0 {
                String::new()
            } else {
                "done".into()
            },
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

/// Run one turn that issues `command` and return the shell tool's result plus every presenter
/// event.
async fn run(command: &'static str, mode: PermissionMode) -> (String, Vec<PresenterEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut config = Config {
        permission_mode: mode,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.git.commit_nudge = false;
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(ShellOnce {
            command,
            calls: Default::default(),
        }),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(CapturePresenter {
            events: events.clone(),
            attended: true,
            ..Default::default()
        }),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("run it").await.unwrap();
    let id = session.session_id().to_string();
    let result = store
        .load_messages(&id)
        .unwrap()
        .into_iter()
        .find(|m| m.role == Role::Tool)
        .map(|m| m.content)
        .unwrap_or_default();
    let captured = events.lock().unwrap().clone();
    (result, captured)
}

fn shell_result_ok(events: &[PresenterEvent]) -> Option<bool> {
    events.iter().find_map(|e| match e {
        PresenterEvent::ToolResult { name, ok, .. } if name == "shell" => Some(*ok),
        _ => None,
    })
}

#[tokio::test]
async fn a_shell_command_that_exits_nonzero_is_not_drawn_as_a_success() {
    let (result, events) = run("exit 3", PermissionMode::Bypass).await;
    assert!(result.starts_with("shell: exit 3"), "{result}");
    assert_eq!(shell_result_ok(&events), Some(false));

    let (result, events) = run("true", PermissionMode::Bypass).await;
    assert!(result.starts_with("shell: exit 0"), "{result}");
    assert_eq!(shell_result_ok(&events), Some(true));
}

#[tokio::test]
async fn a_call_the_person_declined_does_not_blame_policy() {
    // `CapturePresenter` answers every confirmation with Deny, standing in for the person. The
    // command writes: Ask runs a read-only one (`echo hi`) without ever asking.
    let (result, events) = run("touch declined_probe.txt", PermissionMode::Default).await;
    assert!(result.starts_with("permission denied"), "{result}");
    assert!(result.contains("by the user"), "{result}");
    assert!(!result.contains("policy"), "{result}");
    assert_eq!(shell_result_ok(&events), Some(false));
}
