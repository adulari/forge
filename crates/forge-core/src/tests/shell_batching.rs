//! Shell batching through the session: provably read-only shell calls in one response run as a
//! concurrent batch, anything that writes stays serial, and a `commands` batch is judged by the
//! permission broker entry by entry.

use super::*;

struct OneStep {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    commands: Vec<&'static str>,
}

#[async_trait::async_trait]
impl Provider for OneStep {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let first = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        let tool_calls = if first {
            self.commands
                .iter()
                .map(|cmd| forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "shell".into(),
                    args: serde_json::json!({ "command": cmd }),
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if first { String::new() } else { "Done.".into() },
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

/// The order of tool starts (`S`) and results (`R`) a one-step turn produced.
async fn tool_event_order(commands: Vec<&'static str>) -> String {
    tool_event_order_with_hooks(commands, Vec::new()).await
}

fn hook(event: forge_config::HookEvent) -> forge_config::HookConfig {
    forge_config::HookConfig {
        event,
        matcher: None,
        command: "true".into(),
        timeout_secs: 5,
        cc_compat: false,
    }
}

async fn tool_event_order_with_hooks(
    commands: Vec<&'static str>,
    hooks: Vec<forge_config::HookConfig>,
) -> String {
    run_one_step(commands, hooks).await.0
}

/// Run a one-step turn; returns the tool event order and the final transcript.
async fn run_one_step(
    commands: Vec<&'static str>,
    hooks: Vec<forge_config::HookConfig>,
) -> (String, Vec<Message>) {
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
    config.hooks = hooks;
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(OneStep {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            commands,
        }),
        Arc::new(FixedRouter {
            model: "ollama::batch".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("check the tree").await.unwrap();
    let events = events.lock().unwrap();
    let order = events
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::ToolStart { name, .. } if name == "shell" => Some('S'),
            PresenterEvent::ToolResult { name, .. } if name == "shell" => Some('R'),
            _ => None,
        })
        .collect();
    (order, session.transcript.clone())
}

#[tokio::test]
async fn a_failed_read_only_call_in_a_concurrent_batch_is_still_diagnosed() {
    let (order, transcript) = run_one_step(
        vec!["ls forge-definitely-missing-dir", "echo a"],
        Vec::new(),
    )
    .await;
    assert_eq!(order, "SSRR");
    assert!(
        transcript
            .iter()
            .any(|m| m.role == Role::System && m.content.starts_with("[shell diagnosis]")),
        "the failing ls got a diagnosis hint"
    );
}

#[tokio::test]
async fn read_only_shell_calls_in_one_response_run_as_a_concurrent_batch() {
    assert_eq!(tool_event_order(vec!["echo a", "ls"]).await, "SSRR");
}

/// A session-start or prompt hook never runs around a tool call, so it must not cost the
/// concurrent path (it did: any configured hook disabled it). A tool hook still does.
#[tokio::test]
async fn only_tool_hooks_force_the_serial_path() {
    let session_hook = vec![hook(forge_config::HookEvent::SessionStart)];
    assert_eq!(
        tool_event_order_with_hooks(vec!["echo a", "ls"], session_hook).await,
        "SSRR"
    );
    let tool_hook = vec![hook(forge_config::HookEvent::PreToolUse)];
    assert_eq!(
        tool_event_order_with_hooks(vec!["echo a", "ls"], tool_hook).await,
        "SRSR"
    );
}

#[tokio::test]
async fn a_shell_call_that_writes_keeps_the_whole_step_serial() {
    assert_eq!(
        tool_event_order(vec!["echo a", "rm -f /tmp/forge-shell-batching-absent"]).await,
        "SRSR"
    );
}

#[test]
fn a_batch_with_one_risky_entry_asks_in_auto_mode() {
    let args = serde_json::json!({ "commands": ["ls", "rm -rf build"] });
    assert_eq!(
        permission::decide(
            forge_types::PermissionMode::Auto,
            forge_types::SideEffect::Shell,
            "shell",
            &args,
            &[],
        ),
        PermissionDecision::Ask
    );
}

#[test]
fn a_batch_with_one_secret_read_is_denied_even_in_full_mode() {
    let args = serde_json::json!({ "commands": ["ls", "cat ~/.ssh/id_rsa"] });
    assert_eq!(
        permission::decide(
            forge_types::PermissionMode::Bypass,
            forge_types::SideEffect::Shell,
            "shell",
            &args,
            &forge_config::builtin_deny_rules(),
        ),
        PermissionDecision::Deny
    );
}

#[test]
fn read_only_mode_allows_a_batch_only_when_every_entry_reads() {
    let decide = |commands: serde_json::Value| {
        permission::decide(
            forge_types::PermissionMode::Plan,
            forge_types::SideEffect::Shell,
            "shell",
            &serde_json::json!({ "commands": commands }),
            &[],
        )
    };
    assert_eq!(
        decide(serde_json::json!(["ls", "git log -1"])),
        PermissionDecision::Allow
    );
    assert_eq!(
        decide(serde_json::json!(["ls", "touch x"])),
        PermissionDecision::Deny
    );
}
