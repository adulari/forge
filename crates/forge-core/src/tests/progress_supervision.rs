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
        // These tests count model calls; a compaction summary is one more. 300 steps under the
        // unknown model's 32k floor crossed the trigger once the base prompt grew.
        config
            .mesh
            .context_windows
            .insert("direct::scripted".into(), 1_000_000);
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
async fn three_hundred_productive_steps_are_never_interrupted_headless() {
    let provider = Scripted::new(productive_plan(300));
    let calls = Arc::clone(&provider.calls);
    let mut h = Harness::new(provider, PermissionMode::Bypass, false, |_| {});
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
async fn a_headless_read_only_turn_is_stopped_with_a_stalled_outcome_and_a_way_back() {
    let provider = Scripted::new(Box::new(|n| Some(novel_read(n))));
    let mut h = Harness::new(provider, PermissionMode::Bypass, false, |config| {
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
    let provider = Scripted::new(Box::new(move |n| (n < 20).then(wait)));
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

/// A harness over a git repository whose model runs `command(n)` as its n-th step, `steps` times.
fn git_harness_with(
    command: impl Fn(usize) -> String + Send + Sync + 'static,
    steps: usize,
    attended: bool,
) -> Harness {
    git_harness_plan(
        Box::new(move |n| {
            (n < steps).then(|| {
                (
                    "Running it again.".to_string(),
                    tool("shell", serde_json::json!({ "command": command(n) })),
                )
            })
        }),
        attended,
    )
}

/// A harness over a git repository holding one tracked file, `tracked.txt`.
fn git_harness_plan(plan: Plan, attended: bool) -> Harness {
    let h = Harness::new(
        Scripted::new(plan),
        PermissionMode::Bypass,
        attended,
        |_| {},
    );
    git(h.dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(h.dir.path().join("tracked.txt"), "start\n").unwrap();
    git(h.dir.path(), &["add", "tracked.txt"]);
    git(h.dir.path(), &["commit", "-q", "-m", "init"]);
    h
}

fn git_harness(command: &'static str, steps: usize) -> Harness {
    git_harness_with(move |_| command.to_string(), steps, true)
}

#[tokio::test]
async fn shell_driven_work_that_changes_the_tree_is_progress_and_is_never_nudged() {
    // Every step runs the same silent command, so a ledger that reads results sees a cycle; but
    // each run edits a tracked file into a state it never had, so the turn is doing something.
    let mut h = git_harness("echo more >> tracked.txt", 40);
    let out = h.session.run_turn("append to the file").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.errors("Stopped:"), 0);
    assert_eq!(h.warnings("progress check"), 0, "not even a nudge");
}

#[tokio::test]
async fn the_same_silent_command_that_changes_nothing_is_stopped() {
    let mut h = git_harness("true", 14);
    let out = h.session.run_turn("do nothing, repeatedly").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert_eq!(
        h.warnings("Stopped:"),
        1,
        "a person is watching: a warning, and the model writes a status"
    );
    assert_eq!(h.errors("Stopped:"), 0);
}

#[tokio::test]
async fn a_headless_stall_is_an_error_with_the_way_to_resume() {
    let mut h = git_harness_with(|_| "true".to_string(), 14, false);
    let out = h.session.run_turn("do nothing, repeatedly").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert_eq!(h.errors("Stopped:"), 1);
    assert_eq!(h.errors("Resume: forge run --continue"), 1);
}

/// A tracked file that goes through three contents and round again: the first three states are
/// new, every state after that is one it was already in.
#[tokio::test]
async fn a_tracked_file_rewritten_through_three_known_states_is_stopped() {
    let mut h = git_harness_with(
        |n| format!("printf 'state {}\\n' > tracked.txt", n % 3),
        60,
        true,
    );
    let out = h.session.run_turn("keep rewriting the file").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert!(
        h.model_calls() <= 24,
        "stopped after {} model calls",
        h.model_calls()
    );
    assert_eq!(h.warnings("progress check"), 1);
}

#[tokio::test]
async fn a_shell_migration_outside_git_is_progress_too() {
    let mut h = Harness::new(
        Scripted::new(Box::new(|n| {
            (n < 30).then(|| {
                (
                    "Converting the next file.".to_string(),
                    tool(
                        "shell",
                        serde_json::json!({ "command": format!("printf 'converted {n}\\n' > out_{n}.txt") }),
                    ),
                )
            })
        })),
        PermissionMode::AcceptEdits,
        true,
        |_| {},
    );
    let out = h.session.run_turn("convert the files").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert!(h.dir.path().join("out_29.txt").exists());
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.errors("Stopped:"), 0);
}

/// A shell loop that grows an untracked log and reports its length: new output every step and a
/// new tree state every step, so nothing is ever stale.
#[tokio::test]
async fn growing_an_untracked_file_from_the_shell_is_progress() {
    let mut h = git_harness("echo more >> log.txt; wc -l log.txt", 90);
    let out = h.session.run_turn("keep the log going").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.errors("Stopped:"), 0);
}

/// Silent commands that only write gitignored outputs leave git nothing to see; each one is a
/// command not run before, which is what the ledger can tell.
#[tokio::test]
async fn silent_commands_that_write_only_gitignored_outputs_are_not_idleness() {
    let mut h = git_harness_with(
        |n| format!("mkdir -p out && printf 'part {n}\\n' > out/part_{n}.txt"),
        30,
        true,
    );
    std::fs::write(h.dir.path().join(".gitignore"), "out/\n").unwrap();
    git(h.dir.path(), &["add", ".gitignore"]);
    git(h.dir.path(), &["commit", "-q", "-m", "ignore out"]);
    let out = h.session.run_turn("generate the parts").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.errors("Stopped:"), 0);
}

/// A command that is cut by its time limit and called again is how a run longer than the cap is
/// watched: it is a wait, however identical the answers.
#[tokio::test]
async fn a_watch_cut_by_the_shell_time_limit_and_repeated_is_a_wait() {
    let provider = Scripted::new(Box::new(|n| {
        (n < 9).then(|| {
            (
                "Watching the run again.".to_string(),
                tool(
                    "shell",
                    serde_json::json!({"command": "sleep 25", "timeout_secs": 1}),
                ),
            )
        })
    }));
    let mut h = Harness::new(provider, PermissionMode::Bypass, true, |_| {});
    let out = h.session.run_turn("watch the CI run").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.warnings("Stopped:"), 0);
}

/// `sleep`, then the job's log, alternately: the sleep always answers the same and the log keeps
/// growing, which is two calls that look alike only to a ledger that ignores what they are for.
#[tokio::test(flavor = "multi_thread")]
async fn following_a_job_log_with_a_sleep_between_reads_is_a_wait() {
    let _only_one = JOB_TESTS.lock().await;
    let state = tempfile::tempdir().unwrap();
    std::env::set_var("FORGE_STATE_DIR", state.path());
    let id = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let plan_id = Arc::clone(&id);
    let mut h = Harness::new(
        Scripted::new(Box::new(move |n| {
            (n < 16).then(|| {
                if n % 2 == 0 {
                    (
                        "Giving the build a moment.".to_string(),
                        tool("shell", serde_json::json!({"command": "sleep 0.3"})),
                    )
                } else {
                    (
                        "Reading the log again.".to_string(),
                        tool(
                            "shell_job",
                            serde_json::json!({"action": "log", "id": plan_id.load(std::sync::atomic::Ordering::SeqCst)}),
                        ),
                    )
                }
            })
        })),
        PermissionMode::AcceptEdits,
        true,
        |_| {},
    );
    let job = start_job(
        &h,
        "i=0; while true; do i=$((i+1)); echo \"building crate $i\"; sleep 0.1; done",
    )
    .await;
    id.store(job, std::sync::atomic::Ordering::SeqCst);
    let out = h.session.run_turn("wait for the build").await;
    stop_job(&h, job).await;
    let out = out.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.warnings("Stopped:"), 0);
}

/// Different edits, then the same failing test, again and again: the edits do not touch what
/// fails. No edit repeats, so only the check's own verdict gives it away.
#[tokio::test]
async fn different_edits_between_the_same_failing_test_run_are_nudged_then_stopped() {
    let mut h = Harness::new(
        Scripted::new(Box::new(|n| {
            (n < 60).then(|| {
                if n % 2 == 0 {
                    (
                        "Trying another fix.".to_string(),
                        tool(
                            "write_file",
                            serde_json::json!({"path": "fix.txt", "content": format!("attempt {n}\n")}),
                        ),
                    )
                } else {
                    (
                        "Running the tests.".to_string(),
                        tool("shell", serde_json::json!({"command": "sh run_tests.sh"})),
                    )
                }
            })
        })),
        PermissionMode::AcceptEdits,
        true,
        |_| {},
    );
    std::fs::write(
        h.dir.path().join("run_tests.sh"),
        "echo 'FAILED tests::parses_the_header'\necho 'assertion failed: left == right'\nexit 1\n",
    )
    .unwrap();
    let out = h.session.run_turn("make the tests pass").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert_eq!(h.warnings("progress check"), 1);
    assert!(
        h.system_notes("failed 6 times with the same output") >= 1,
        "the nudge names the check"
    );
    assert!(h.model_calls() <= 26, "{} model calls", h.model_calls());
}

/// Break a tracked file, read it, repair it, read it: the same four calls with the same results,
/// on a file git tracks. It is stopped wherever in the cycle the nudge lands.
#[tokio::test]
async fn breaking_and_repairing_a_tracked_file_in_a_cycle_is_stopped() {
    let plan: Plan = Box::new(|n| {
        (n < 80).then(|| match n % 4 {
            0 => (
                "Trying a change.".to_string(),
                tool(
                    "edit_file",
                    serde_json::json!({"path": "tracked.txt", "old": "start", "new": "broken"}),
                ),
            ),
            2 => (
                "That did not help, putting it back.".to_string(),
                tool(
                    "edit_file",
                    serde_json::json!({"path": "tracked.txt", "old": "broken", "new": "start"}),
                ),
            ),
            _ => (
                "Looking at the file.".to_string(),
                tool("read_file", serde_json::json!({"path": "tracked.txt"})),
            ),
        })
    });
    let mut h = git_harness_plan(plan, true);
    let out = h.session.run_turn("fix the file").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert!(h.model_calls() <= 24, "{} model calls", h.model_calls());
}

/// The person fixes whatever the probe was failing on and says `continue`: the model runs the
/// same probe again, and this time it works.
#[tokio::test]
async fn a_continue_after_the_cause_was_fixed_may_repeat_the_call_that_failed() {
    let second_turn = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls_this_turn = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (flag, count) = (Arc::clone(&second_turn), Arc::clone(&calls_this_turn));
    let probe = || {
        (
            "Probing the service again.".to_string(),
            tool("shell", serde_json::json!({"command": "sh probe.sh"})),
        )
    };
    let mut h = Harness::new(
        Scripted::new(Box::new(move |_| {
            let k = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                (k < 1).then(probe)
            } else {
                Some(probe())
            }
        })),
        PermissionMode::AcceptEdits,
        true,
        |_| {},
    );
    std::fs::write(
        h.dir.path().join("probe.sh"),
        "if test -f ready.flag; then echo 'service is up'; else echo 'probe failed: connection refused'; exit 1; fi\n",
    )
    .unwrap();
    let first = h.session.run_turn("check the service").await.unwrap();
    assert_eq!(first.stop_reason, StopReason::Stalled);

    std::fs::write(h.dir.path().join("ready.flag"), "").unwrap();
    second_turn.store(true, std::sync::atomic::Ordering::SeqCst);
    calls_this_turn.store(0, std::sync::atomic::Ordering::SeqCst);
    let second = h
        .session
        .run_turn("I started the service, continue")
        .await
        .unwrap();
    assert_eq!(second.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("Stopped:"), 1, "only the first turn was stopped");
}

#[tokio::test]
async fn bypass_mode_with_a_person_attending_is_not_stopped_for_idleness() {
    let provider = Scripted::new(Box::new(|n| (n < 20).then(|| novel_read(n))));
    let mut h = Harness::new(provider, PermissionMode::Bypass, true, |config| {
        config.mesh.idle_nudge_steps = 6;
        config.mesh.idle_stop_steps = 8;
    });
    write_novel_file(h.dir.path());
    let out = h.session.run_turn("read it all").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("Stopped:"), 0);
    assert_eq!(h.errors("Stopped:"), 0);
}

/// The jobs registry lives under a process-wide state directory, so tests that start jobs take
/// turns.
static JOB_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn start_job(h: &Harness, command: &str) -> u32 {
    let shell = h.session.tools.get("shell").expect("shell tool");
    let out = shell
        .run(&serde_json::json!({"command": command, "background": true}))
        .await
        .unwrap();
    out.split("started background job ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("no job id in: {out}"))
}

async fn stop_job(h: &Harness, id: u32) {
    let job = h.session.tools.get("shell_job").expect("shell_job tool");
    let _ = job
        .run(&serde_json::json!({"action": "stop", "id": id}))
        .await;
    // The id is the pid of a process this test started.
    let _ = std::process::Command::new("kill")
        .arg(id.to_string())
        .status();
}

/// Waits on a job that is still running, polling it with `shell_job status` once per step the
/// way the harness tells the model to.
async fn poll_a_running_job(polls: usize, attended: bool) -> (Harness, LoopOutcome) {
    let _only_one = JOB_TESTS.lock().await;
    let state = tempfile::tempdir().unwrap();
    std::env::set_var("FORGE_STATE_DIR", state.path());
    let id = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let plan_id = Arc::clone(&id);
    let mut h = Harness::new(
        Scripted::new(Box::new(move |n| {
            (n < polls).then(|| {
                (
                    "The build is still running; checking on it again.".to_string(),
                    tool(
                        "shell_job",
                        serde_json::json!({"action": "status", "id": plan_id.load(std::sync::atomic::Ordering::SeqCst)}),
                    ),
                )
            })
        })),
        PermissionMode::AcceptEdits,
        attended,
        |_| {},
    );
    let job = start_job(&h, "sleep 600").await;
    id.store(job, std::sync::atomic::Ordering::SeqCst);
    let out = h.session.run_turn("wait for the release build").await;
    stop_job(&h, job).await;
    (h, out.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn polling_a_running_background_job_is_waiting_not_looping() {
    let (h, out) = poll_a_running_job(14, true).await;
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.warnings("progress check"), 0);
    assert_eq!(h.warnings("Stopped:"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wait_that_goes_on_is_nudged_once_attended_and_never_stopped() {
    let (h, out) = poll_a_running_job(40, true).await;
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert_eq!(h.system_notes("waited 24 steps"), 1);
    assert_eq!(h.warnings("Stopped:"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wait_that_goes_on_is_stopped_when_nobody_is_watching() {
    let (h, out) = poll_a_running_job(200, false).await;
    assert_eq!(out.stop_reason, StopReason::Stalled);
    let said: Vec<String> = h
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Error(text) | PresenterEvent::Warning(text) => {
                Some(text.chars().take(160).collect())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        h.errors("still waiting for something that is not ready"),
        1,
        "{said:#?}"
    );
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
