//! Progress supervision through real turns: a turn that is making progress is never interrupted
//! by a count, a turn that is not is nudged once and then stopped cleanly, and the explicit
//! limits and spend ceilings the user sets keep working.

use super::*;
use forge_config::ProgressGuardMode;

type Plan = Box<dyn Fn(usize) -> Option<(String, forge_types::ToolCall)> + Send + Sync>;

/// Calls what `plan` says for each model call; once it returns `None` the model answers in text.
struct Scripted {
    plan: Plan,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl Scripted {
    fn new(plan: Plan) -> Self {
        Self {
            plan,
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, tool_calls) = match (self.plan)(n) {
            Some((text, call)) => (text, vec![call]),
            None => ("All done.".to_string(), Vec::new()),
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

fn tool(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: forge_types::new_id(),
        name: name.into(),
        args,
    }
}

struct Harness {
    session: Session,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
    dir: tempfile::TempDir,
}

impl Harness {
    fn new(
        provider: impl Provider + 'static,
        mode: PermissionMode,
        attended: bool,
        tune: impl FnOnce(&mut Config),
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let capture = CapturePresenter {
            attended,
            ..Default::default()
        };
        let events = capture.events.clone();
        let mut config = Config {
            permission_mode: mode,
            ..Config::default()
        };
        config.recap.enabled = false;
        config.suggest.enabled = false;
        config.mesh.auto_memory = false;
        config.mesh.verify_completeness = false;
        config.mesh.nudge_empty_diff = false;
        config.shell.explain_errors = false;
        tune(&mut config);
        let session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(provider),
            Arc::new(FixedRouter {
                model: "direct::scripted".into(),
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
            events,
            dir,
        }
    }

    fn texts(&self, error: bool, needle: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| match e {
                PresenterEvent::Warning(w) => !error && w.contains(needle),
                PresenterEvent::Error(w) => error && w.contains(needle),
                _ => false,
            })
            .count()
    }

    fn warnings(&self, needle: &str) -> usize {
        self.texts(false, needle)
    }

    fn errors(&self, needle: &str) -> usize {
        self.texts(true, needle)
    }

    fn model_calls(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::ProviderRequest { .. }))
            .count()
    }

    fn system_notes(&self, needle: &str) -> usize {
        self.session
            .transcript
            .iter()
            .filter(|m| m.role == Role::System && m.content.contains(needle))
            .count()
    }
}

/// A file of 400 lines, each different, to read one line at a time.
fn write_novel_file(dir: &std::path::Path) {
    let text: String = (0..400)
        .map(|i| format!("unique line number {i}\n"))
        .collect();
    std::fs::write(dir.join("big.txt"), text).unwrap();
}

fn novel_read(n: usize) -> (String, forge_types::ToolCall) {
    (
        format!("Reading the next stretch ({n}) to see what else is in there."),
        tool(
            "read_file",
            serde_json::json!({"path": "big.txt", "start_line": n + 1, "end_line": n + 1}),
        ),
    )
}

fn productive_plan(steps: usize) -> Plan {
    Box::new(move |n| {
        (n < steps).then(|| {
            (
                "Now let me update the module so the next case is covered as well.".to_string(),
                tool(
                    "write_file",
                    serde_json::json!({"path": format!("case_{n}.txt"), "content": format!("{n}\n")}),
                ),
            )
        })
    })
}

#[tokio::test]
async fn three_hundred_productive_steps_are_never_interrupted_attended() {
    let provider = Scripted::new(productive_plan(300));
    let calls = Arc::clone(&provider.calls);
    let mut h = Harness::new(provider, PermissionMode::AcceptEdits, true, |_| {});
    let out = h.session.run_turn("add the cases").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 301);
    assert!(h.dir.path().join("case_299.txt").exists());
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.warnings("Stopped:"), 0);
    assert_eq!(h.warnings("step limit"), 0);
    assert_eq!(h.errors(""), 0);
}

#[tokio::test]
async fn three_hundred_productive_steps_are_never_interrupted_unattended() {
    let provider = Scripted::new(productive_plan(300));
    let calls = Arc::clone(&provider.calls);
    let mut h = Harness::new(provider, PermissionMode::Bypass, true, |_| {});
    let out = h.session.run_turn("add the cases").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 301);
    assert_eq!(
        h.warnings("soft step cap"),
        0,
        "there is no soft cap any more"
    );
    assert_eq!(
        h.errors(""),
        0,
        "an unattended productive turn raises no error"
    );
}

#[tokio::test]
async fn an_attended_read_only_investigation_is_nudged_but_never_stopped() {
    let provider = Scripted::new(Box::new(|n| (n < 20).then(|| novel_read(n))));
    let mut h = Harness::new(provider, PermissionMode::AcceptEdits, true, |config| {
        config.mesh.idle_nudge_steps = 6;
        config.mesh.idle_stop_steps = 8;
    });
    write_novel_file(h.dir.path());
    let out = h.session.run_turn("what is in big.txt?").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(
        h.system_notes("steps without changing a file or closing a task"),
        1,
        "one idle nudge, however long the investigation runs"
    );
    assert_eq!(
        h.warnings("Stopped:"),
        0,
        "an attended turn is never stopped for idleness"
    );
}

#[tokio::test]
async fn an_unattended_read_only_turn_is_stopped_with_a_stalled_outcome_and_a_way_back() {
    let provider = Scripted::new(Box::new(|n| Some(novel_read(n))));
    let mut h = Harness::new(provider, PermissionMode::Bypass, true, |config| {
        config.mesh.idle_nudge_steps = 6;
        config.mesh.idle_stop_steps = 12;
    });
    write_novel_file(h.dir.path());
    let out = h.session.run_turn("read everything").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert_eq!(
        h.errors("Stopped: no file change or finished task for 12 steps"),
        1
    );
    assert_eq!(h.errors("Resume: forge run --continue"), 1);
    assert!(h.model_calls() <= 13, "{} model calls", h.model_calls());
    assert!(h.events.lock().unwrap().iter().any(|e| matches!(
        e,
        PresenterEvent::Done {
            stop_reason: StopReason::Stalled,
            ..
        }
    )));
}

#[tokio::test]
async fn progress_guard_warn_reports_where_it_would_stop_and_lets_the_turn_finish() {
    let same = || {
        (
            "Reading it once more.".to_string(),
            tool(
                "read_file",
                serde_json::json!({"path": "big.txt", "start_line": 1, "end_line": 1}),
            ),
        )
    };
    let provider = Scripted::new(Box::new(move |n| (n < 12).then(same)));
    let mut h = Harness::new(provider, PermissionMode::AcceptEdits, true, |config| {
        config.mesh.progress_guard = ProgressGuardMode::Warn;
    });
    write_novel_file(h.dir.path());
    let out = h.session.run_turn("look at it").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("would have stopped here"), 1);
    assert_eq!(h.warnings("Stopped:"), 0);
    assert_eq!(h.system_notes("Progress check from Forge"), 1);
}

#[tokio::test]
async fn progress_guard_off_keeps_the_nudge_and_never_stops() {
    let same = || {
        (
            "Reading it once more.".to_string(),
            tool(
                "read_file",
                serde_json::json!({"path": "big.txt", "start_line": 1, "end_line": 1}),
            ),
        )
    };
    let provider = Scripted::new(Box::new(move |n| (n < 12).then(same)));
    let mut h = Harness::new(provider, PermissionMode::AcceptEdits, true, |config| {
        config.mesh.progress_guard = ProgressGuardMode::Off;
    });
    write_novel_file(h.dir.path());
    let out = h.session.run_turn("look at it").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("would have stopped here"), 0);
    assert_eq!(h.warnings("Stopped:"), 0);
    assert_eq!(h.system_notes("Progress check from Forge"), 1);
}

#[tokio::test]
async fn a_stopped_turn_tells_the_next_turn_why_and_a_repeat_after_continue_stops_at_once() {
    let same = || {
        (
            "Reading it once more.".to_string(),
            tool(
                "read_file",
                serde_json::json!({"path": "big.txt", "start_line": 1, "end_line": 1}),
            ),
        )
    };
    let provider = Scripted::new(Box::new(move |_| Some(same())));
    let mut h = Harness::new(provider, PermissionMode::AcceptEdits, true, |_| {});
    write_novel_file(h.dir.path());

    let first = h.session.run_turn("look at it").await.unwrap();
    assert_eq!(first.stop_reason, StopReason::Stalled);
    assert_eq!(h.warnings("Stopped:"), 1);
    assert_eq!(h.system_notes("Progress check from Forge"), 1);
    assert_eq!(h.system_notes("Forge stopped your previous turn"), 1);
    let calls_after_first = h.model_calls();

    let second = h.session.run_turn("continue").await.unwrap();
    assert_eq!(second.stop_reason, StopReason::Stalled);
    assert_eq!(h.warnings("Stopped:"), 2);
    assert_eq!(
        h.warnings("progress check"),
        1,
        "the repeat after a stop gets no second nudge"
    );
    assert!(
        h.model_calls() - calls_after_first <= 2,
        "the resumed turn was stopped on its first repeated call: {} calls",
        h.model_calls() - calls_after_first
    );
}

#[tokio::test]
async fn a_poll_mode_wait_that_is_not_ready_is_never_a_loop() {
    let wait = || {
        (
            "Waiting for the build.".to_string(),
            tool(
                "shell",
                serde_json::json!({
                    "command": "false",
                    "poll_until_exit_zero": true,
                    "poll_interval_secs": 1,
                    "timeout_secs": 1
                }),
            ),
        )
    };
    let provider = Scripted::new(Box::new(move |n| (n < 25).then(wait)));
    let mut h = Harness::new(provider, PermissionMode::Bypass, true, |_| {});
    let out = h
        .session
        .run_turn("wait for the release build")
        .await
        .unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.errors("Stopped:"), 0);
}

fn git(root: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

fn git_harness(command: &'static str, steps: usize) -> Harness {
    let run = move || {
        (
            "Running it again.".to_string(),
            tool("shell", serde_json::json!({ "command": command })),
        )
    };
    let h = Harness::new(
        Scripted::new(Box::new(move |n| (n < steps).then(run))),
        PermissionMode::Bypass,
        true,
        |_| {},
    );
    git(h.dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(h.dir.path().join("tracked.txt"), "start\n").unwrap();
    git(h.dir.path(), &["add", "tracked.txt"]);
    git(h.dir.path(), &["commit", "-q", "-m", "init"]);
    h
}

#[tokio::test]
async fn shell_driven_work_that_changes_the_tree_vetoes_the_stop() {
    // Every step runs the same silent command, so the ledger sees a cycle; but each run edits a
    // tracked file, so the turn is doing something and must not be stopped.
    let mut h = git_harness("echo more >> tracked.txt", 14);
    let out = h.session.run_turn("append to the file").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.errors("Stopped:"), 0);
    assert!(
        h.system_notes("Progress check from Forge") >= 1,
        "it was nudged"
    );
}

#[tokio::test]
async fn the_same_silent_command_that_changes_nothing_is_stopped() {
    let mut h = git_harness("true", 14);
    let out = h.session.run_turn("do nothing, repeatedly").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert_eq!(h.errors("Stopped:"), 1);
}

#[tokio::test]
async fn an_explicit_step_limit_counts_the_turn_not_each_re_drive_pass() {
    // The empty-diff gate re-drives the loop once after the first pass. With an explicit limit
    // of 5 the second pass must start with no allowance left, not a fresh five.
    let provider = Scripted::new(Box::new(|n| Some(novel_read(n))));
    let mut h = Harness::new(provider, PermissionMode::AcceptEdits, true, |config| {
        config.mesh.max_steps = 5;
        config.mesh.nudge_empty_diff = true;
    });
    write_novel_file(h.dir.path());
    h.session.set_expect_code_change(true);
    let out = h.session.run_turn("fix the bug in big.txt").await.unwrap();
    // A turn that never produced a final answer is reported as having produced nothing; either
    // way it is a failed turn, never a success.
    assert!(!out.stop_reason.is_success(), "{:?}", out.stop_reason);
    assert_eq!(h.model_calls(), 5, "{} model calls", h.model_calls());
    assert_eq!(h.warnings("5-step limit"), 1);
}

#[tokio::test]
async fn the_default_config_has_no_step_limit_at_all() {
    let defaults = Config::default();
    assert_eq!(defaults.mesh.max_steps, 0);
    assert_eq!(defaults.mesh.max_steps_unattended, 0);
    assert_eq!(defaults.mesh.progress_guard, ProgressGuardMode::Enforce);
    assert_eq!(defaults.mesh.idle_nudge_steps, 60);
    assert_eq!(defaults.mesh.idle_stop_steps, 200);
    assert_eq!(crate::explicit_step_limit(0, 0, true), None);
    assert_eq!(crate::explicit_step_limit(0, 0, false), None);
    assert_eq!(crate::explicit_step_limit(7, 0, true), Some(7));
    assert_eq!(crate::explicit_step_limit(7, 9, true), Some(9));
    assert_eq!(crate::explicit_step_limit(7, 9, false), Some(7));
    assert_eq!(crate::explicit_step_limit(0, 9, false), None);
}
