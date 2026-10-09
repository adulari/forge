//! Plan mode through a real turn: `shell` is advertised with the read-only note, a read-only
//! command runs, and a writer is refused with the plan-mode explanation.

use super::*;

struct ShellProbe {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    seen_tools: Arc<Mutex<Vec<ToolSpec>>>,
    commands: Vec<&'static str>,
}

#[async_trait::async_trait]
impl Provider for ShellProbe {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            *self.seen_tools.lock().unwrap() = tools.to_vec();
        }
        let tool_calls = self
            .commands
            .get(n)
            .map(|cmd| {
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "shell".into(),
                    args: serde_json::json!({ "command": cmd }),
                }]
            })
            .unwrap_or_default();
        let content = if tool_calls.is_empty() { "done" } else { "" };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: content.into(),
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn plan_mode_advertises_shell_and_runs_only_read_only_commands() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut config = Config {
        permission_mode: PermissionMode::Plan,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(ShellProbe {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            seen_tools: Arc::clone(&seen),
            commands: vec!["wc -l Cargo.toml", "touch plan_shell_probe.txt"],
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session
        .run_turn("inspect the repo file Cargo.toml and count its lines")
        .await
        .unwrap();

    let specs = seen.lock().unwrap();
    let shell = specs.iter().find(|s| s.name == "shell").unwrap_or_else(|| {
        panic!(
            "shell advertised; got {:?}",
            specs.iter().map(|s| &s.name).collect::<Vec<_>>()
        )
    });
    assert!(shell
        .description
        .contains("In plan mode only read-only commands run"));

    let results: Vec<&str> = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.as_str())
        .collect();
    assert!(!results[0].starts_with("permission denied"), "{results:?}");
    assert!(
        results[1].starts_with("permission denied by policy: In plan mode"),
        "{results:?}"
    );
    assert!(!test_workspace().join("plan_shell_probe.txt").exists());
}
