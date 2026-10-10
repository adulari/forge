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

/// Edits a file and marks the plan done, writes the real summary, then answers the completion
/// gate's verification request with a one-line "verified".
struct EditSummaryThenVerified {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for EditSummaryThenVerified {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let call = |name: &str, args: serde_json::Value| forge_types::ToolCall {
            id: forge_types::new_id(),
            name: name.into(),
            args,
        };
        let (content, tool_calls) = match n {
            0 => (
                String::new(),
                vec![
                    call(
                        "write_file",
                        serde_json::json!({ "path": "a.txt", "content": "fixed" }),
                    ),
                    call(
                        "update_tasks",
                        serde_json::json!({ "tasks": [{ "title": "fix a.txt", "status": "done" }] }),
                    ),
                ],
            ),
            1 => (
                "Fixed a.txt: the typo is gone and the file now reads `fixed`.".into(),
                vec![],
            ),
            _ => ("Verification completed.".into(), vec![]),
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
async fn verification_redrive_does_not_replace_the_real_answer() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "typo").unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(EditSummaryThenVerified {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        Arc::new(FixedRouter {
            model: "direct::edit-summary".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();

    let outcome = session.run_turn("fix the typo in a.txt").await.unwrap();

    let verified = events.lock().unwrap().iter().any(
        |e| matches!(e, PresenterEvent::Warning(w) if w.contains("verifying with a real state check")),
    );
    assert!(verified, "the gate must have asked for verification");
    assert_eq!(
        outcome.text,
        "Fixed a.txt: the typo is gone and the file now reads `fixed`."
    );
}

/// Writes `a.txt`, then gives `answer`.
struct WriteThenAnswer {
    calls: std::sync::atomic::AtomicUsize,
    answer: &'static str,
}

#[async_trait::async_trait]
impl Provider for WriteThenAnswer {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, tool_calls) = if n == 0 {
            (
                String::new(),
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "write_file".into(),
                    args: serde_json::json!({ "path": "a.txt", "content": "fixed" }),
                }],
            )
        } else {
            (self.answer.to_string(), Vec::new())
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

async fn review_fired_after_small_edit(request: &str, answer: &'static str) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(dir.path().join("a.txt"), "typo").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "init"]);
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.mesh.verify_completeness = true;
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(WriteThenAnswer {
            calls: std::sync::atomic::AtomicUsize::new(0),
            answer,
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
    session.run_turn(request).await.unwrap();
    let fired = events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("completeness check")));
    fired
}

#[tokio::test]
async fn single_small_edit_skips_the_review() {
    assert!(!review_fired_after_small_edit("fix the typo in a.txt", "Fixed the typo.").await);
}

#[tokio::test]
async fn hedging_answer_after_a_small_edit_still_gets_reviewed() {
    assert!(
        review_fired_after_small_edit("fix the typo in a.txt", "Fixed it, docs are still TODO.")
            .await
    );
}
