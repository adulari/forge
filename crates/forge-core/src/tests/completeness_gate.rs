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

/// A CLI bridge as the session sees one: the tools run inside the subprocess (here: the provider
/// itself writes the files and reports each as a stream event), and only text comes back.
struct ScriptedBridge {
    dir: std::path::PathBuf,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    /// One entry per `complete` call: files to write first, tools to report, then the reply. The
    /// last entry repeats.
    steps: Vec<BridgeStep>,
}

#[derive(Clone, Default)]
struct BridgeStep {
    writes: Vec<(&'static str, &'static str)>,
    tools: Vec<(&'static str, &'static str)>,
    reply: &'static str,
}

fn step(reply: &'static str) -> BridgeStep {
    BridgeStep {
        reply,
        ..BridgeStep::default()
    }
}

#[async_trait::async_trait]
impl Provider for ScriptedBridge {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_provider::StreamEvent;
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let step = self.steps[n.min(self.steps.len() - 1)].clone();
        for (path, content) in &step.writes {
            let full = self.dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, content).unwrap();
        }
        for (name, args) in &step.tools {
            on_event(StreamEvent::ToolStarted {
                name: (*name).into(),
                args: (*args).into(),
            });
            on_event(StreamEvent::ToolFinished {
                name: (*name).into(),
                ok: true,
                summary: String::new(),
            });
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: step.reply.to_string(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

const BRIDGES: [&str; 2] = ["claude-cli::opus", "codex-cli::gpt-6-luna"];

struct BridgeRun {
    session: Session,
    store: Arc<Store>,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    dir: tempfile::TempDir,
}

impl BridgeRun {
    /// A repository holding `a.txt` (committed as "typo"), a session on `model`, and a bridge that
    /// plays `steps`. `prepare` runs on the repository before the session starts.
    fn new(model: &str, steps: Vec<BridgeStep>, prepare: impl FnOnce(&std::path::Path)) -> Self {
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
        prepare(dir.path());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = Config {
            permission_mode: forge_types::PermissionMode::Bypass,
            ..Config::default()
        };
        config.mesh.verify_completeness = true;
        let session = Session::start(
            Arc::clone(&store),
            Arc::new(ScriptedBridge {
                dir: dir.path().to_path_buf(),
                calls: Arc::clone(&calls),
                steps,
            }),
            Arc::new(FixedRouter {
                model: model.into(),
                fallbacks: vec![],
            }),
            ToolRegistry::with_core_tools_in(dir.path()),
            Box::new(capture),
            config,
            dir.path().to_str().unwrap(),
        )
        .unwrap();
        Self {
            session,
            store,
            events,
            calls,
            dir,
        }
    }

    fn warned(&self, needle: &str) -> bool {
        self.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains(needle)))
    }

    fn reviewed(&self) -> bool {
        self.warned("completeness check")
    }

    fn model_calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The answer the transcript and the replay keep for the latest turn.
    fn stored_answer(&self) -> String {
        self.store
            .load_replay(&self.session.id)
            .unwrap()
            .into_iter()
            .rev()
            .find(|entry| entry.role == Role::Assistant && !entry.content.is_empty())
            .map(|entry| entry.content)
            .unwrap()
    }
}

fn edit(path: &'static str) -> BridgeStep {
    BridgeStep {
        writes: vec![(path, "fixed")],
        tools: vec![("edit_file", r#"{"path":"x"}"#)],
        reply: "",
    }
}

#[tokio::test]
async fn both_bridges_pass_through_the_same_review_gate() {
    for model in BRIDGES {
        let mut small = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    reply: "Fixed the typo.",
                    ..edit("a.txt")
                },
                step("never reached"),
            ],
            |_| {},
        );
        small
            .session
            .run_turn("fix the typo in a.txt")
            .await
            .unwrap();
        assert!(
            !small.reviewed(),
            "{model}: a small checked-free edit is not reviewed"
        );
        assert_eq!(small.model_calls(), 1, "{model}");

        let mut hedged = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    reply: "Fixed it, the docs are still TODO.",
                    ..edit("a.txt")
                },
                step("Review: nothing to add."),
            ],
            |_| {},
        );
        hedged
            .session
            .run_turn("fix the typo in a.txt")
            .await
            .unwrap();
        assert!(hedged.reviewed(), "{model}: a hedged answer still earns it");
        assert_eq!(hedged.model_calls(), 2, "{model}");
    }
}

#[tokio::test]
async fn test_run_byproducts_do_not_turn_a_one_line_fix_into_a_large_change() {
    for model in BRIDGES {
        let mut run = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    writes: vec![
                        ("a.txt", "fixed"),
                        ("pkg/__pycache__/a.cpython-312.pyc", "x"),
                        ("pkg/__pycache__/b.cpython-312.pyc", "x"),
                        ("node_modules/left-pad/index.js", "x"),
                    ],
                    tools: vec![("edit_file", r#"{"path":"a.txt"}"#)],
                    reply: "Fixed the typo.",
                },
                step("never reached"),
            ],
            |_| {},
        );
        run.session.run_turn("fix the typo in a.txt").await.unwrap();
        assert!(
            !run.reviewed(),
            "{model}: four untracked cache files are not four edits"
        );
    }
}

#[tokio::test]
async fn uncommitted_work_from_an_earlier_turn_is_not_this_turns_change() {
    for model in BRIDGES {
        let mut run = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    reply: "Fixed the typo.",
                    ..edit("a.txt")
                },
                step("never reached"),
            ],
            |root| {
                // Three files an earlier turn changed and left uncommitted, and one it created.
                for name in ["one.txt", "two.txt", "three.txt"] {
                    std::fs::write(root.join(name), "v1\n").unwrap();
                }
                let git = |args: &[&str]| {
                    std::process::Command::new("git")
                        .arg("-C")
                        .arg(root)
                        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                        .args(args)
                        .output()
                        .unwrap()
                };
                git(&["add", "-A"]);
                git(&["commit", "-qm", "earlier work"]);
                for name in ["one.txt", "two.txt", "three.txt"] {
                    std::fs::write(root.join(name), "v2\nmore\n").unwrap();
                }
                std::fs::write(root.join("notes.md"), "scratch").unwrap();
            },
        );
        run.session.run_turn("fix the typo in a.txt").await.unwrap();
        assert!(!run.reviewed(), "{model}");
    }
}

#[tokio::test]
async fn a_review_that_changes_nothing_keeps_the_answer_to_the_user() {
    for model in BRIDGES {
        let answer = "Fixed it, the docs are still TODO.";
        let mut run = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    reply: answer,
                    ..edit("a.txt")
                },
                BridgeStep {
                    tools: vec![("shell", r#"{"command":"git status --short"}"#)],
                    reply: "Review: every requirement is handled.",
                    ..BridgeStep::default()
                },
            ],
            |_| {},
        );
        let outcome = run.session.run_turn("fix the typo in a.txt").await.unwrap();
        assert!(run.reviewed(), "{model}");
        assert_eq!(outcome.text, answer, "{model}");
        assert_eq!(run.stored_answer(), answer, "{model}");
    }
}

#[tokio::test]
async fn a_review_that_only_runs_the_tests_keeps_the_answer() {
    for model in BRIDGES {
        let answer = "Fixed it, the docs are still TODO.";
        let mut run = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    reply: answer,
                    ..edit("a.txt")
                },
                BridgeStep {
                    writes: vec![("pkg/__pycache__/m.cpython-312.pyc", "x")],
                    tools: vec![("shell", r#"{"command":"python3 -m unittest"}"#)],
                    reply: "The test suite passes.",
                },
            ],
            |_| {},
        );
        let outcome = run.session.run_turn("fix the typo in a.txt").await.unwrap();
        assert!(run.reviewed(), "{model}");
        assert_eq!(
            outcome.text, answer,
            "{model}: the cache the run left is not an edit"
        );
    }
}

#[tokio::test]
async fn a_review_that_edits_shows_the_answer_and_then_what_it_did() {
    for model in BRIDGES {
        let answer = "Fixed it, the docs are still TODO.";
        let mut run = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    reply: answer,
                    ..edit("a.txt")
                },
                BridgeStep {
                    reply: "The review also fixed the docs.",
                    ..edit("README.md")
                },
            ],
            |_| {},
        );
        let outcome = run.session.run_turn("fix the typo in a.txt").await.unwrap();
        let both = format!("{answer}\n\nThe review also fixed the docs.");
        assert_eq!(outcome.text, both, "{model}");
        assert_eq!(run.stored_answer(), both, "{model}");
    }
}

#[tokio::test]
async fn a_question_after_a_finished_plan_is_answered_without_a_verification_turn() {
    for model in BRIDGES {
        let answer = "The project is called Lighthouse; the rules were stdlib only and 20 lines.";
        let mut run = BridgeRun::new(
            model,
            vec![step(answer), step("Verification completed.")],
            |_| {},
        );
        // The plan an earlier turn finished: every task Done.
        run.store
            .set_tasks(
                &run.session.id,
                &[forge_types::TodoItem {
                    title: "add the report command".into(),
                    status: forge_types::TodoStatus::Done,
                    assignee: None,
                }],
            )
            .unwrap();
        let outcome = run
            .session
            .run_turn(
                "Memory check, answer from our conversation without running any commands: \
                 what is the project called?",
            )
            .await
            .unwrap();
        assert_eq!(outcome.text, answer, "{model}");
        assert_eq!(
            run.model_calls(),
            1,
            "{model}: nothing to verify, nothing re-driven"
        );
        assert!(!run.warned("verifying with a real state check"), "{model}");
        assert_eq!(run.stored_answer(), answer, "{model}");
    }
}

#[tokio::test]
async fn a_turn_that_edited_still_has_its_finished_plan_verified() {
    for model in BRIDGES {
        let mut run = BridgeRun::new(
            model,
            vec![
                BridgeStep {
                    reply: "Fixed a.txt: the typo is gone.",
                    ..edit("a.txt")
                },
                step("Verification completed."),
            ],
            |_| {},
        );
        run.store
            .set_tasks(
                &run.session.id,
                &[forge_types::TodoItem {
                    title: "fix a.txt".into(),
                    status: forge_types::TodoStatus::Done,
                    assignee: None,
                }],
            )
            .unwrap();
        let outcome = run.session.run_turn("fix the typo in a.txt").await.unwrap();
        assert!(run.warned("verifying with a real state check"), "{model}");
        assert_eq!(outcome.text, "Fixed a.txt: the typo is gone.", "{model}");
        let _ = &run.dir;
    }
}
