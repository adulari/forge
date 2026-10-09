//! The completeness review runs only when the turn changed the working tree.

use super::*;

/// Runs one harmless shell command, then answers.
struct ShellThenAnswer {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for ShellThenAnswer {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tool_calls = if n == 0 {
            vec![forge_types::ToolCall {
                id: "1".into(),
                name: "shell".into(),
                args: serde_json::json!({ "command": "echo built" }),
            }]
        } else {
            Vec::new()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if n == 0 {
                String::new()
            } else {
                "It printed: built".into()
            },
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn shell_only_turn_in_a_clean_repo_skips_the_review() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(dir.path().join("a.txt"), "a").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-qm",
        "init",
    ]);

    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.mesh.verify_completeness = true;
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(ShellThenAnswer {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();

    let outcome = session.run_turn("run the build").await.unwrap();

    let reviewed = events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("completeness check")));
    assert!(
        !reviewed,
        "a turn that changed nothing must not be reviewed"
    );
    assert_eq!(outcome.text, "It printed: built");
}
