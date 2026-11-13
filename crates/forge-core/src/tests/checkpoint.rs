//! Checkpoint compaction at a landed slice, through real turns with a scripted provider: no model
//! writes the summary, and nothing folds unless a commit followed a passing check.

use super::*;
use crate::compaction_checkpoint::{fold_superseded_reads, CheckpointTracker, CHECKPOINT_HEADING};
use crate::compaction_episodes::EPISODES_HEADING;

fn call(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: forge_types::new_id(),
        name: name.into(),
        args,
    }
}

/// Answers each main-loop request with the next scripted batch of calls, then "done". Counts the
/// summarizer calls, which a checkpoint must never make.
struct Scripted {
    script: Mutex<std::collections::VecDeque<Vec<forge_types::ToolCall>>>,
    requests: Mutex<Vec<Vec<Message>>>,
    summarizer_calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.requests.lock().unwrap().push(messages.to_vec());
        let calls = self.script.lock().unwrap().pop_front().unwrap_or_default();
        Ok(forge_provider::ModelResponse {
            content: if calls.is_empty() { "done" } else { "" }.into(),
            tool_calls: calls,
            ..Default::default()
        })
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        opts: &forge_provider::CompletionOptions,
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let key = opts.prompt_cache_key.as_deref().unwrap_or_default();
        if key.ends_with(":compact") {
            self.summarizer_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if key.contains(':') {
            return Ok(forge_provider::ModelResponse {
                content: "NONE".into(),
                ..Default::default()
            });
        }
        self.complete(model, messages, tools, on_event).await
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Check {
    Passes,
    Fails,
    Skipped,
}

struct Scenario {
    check: Check,
    commit: bool,
    enabled: bool,
    min_tokens: u64,
    a_words: usize,
    /// Run the check and the commit as `tools.shell` calls inside a `run_code` script.
    via_run_code: bool,
    /// Run the check and the commit in one `&&` chain instead of two steps.
    chained: bool,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            check: Check::Passes,
            commit: true,
            enabled: true,
            min_tokens: 3_000,
            a_words: 1_500,
            via_run_code: false,
            chained: false,
        }
    }
}

struct Outcome {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    store: Arc<Store>,
    session: Session,
    provider: Arc<Scripted>,
    warnings: Vec<String>,
}

fn words(n: usize, tag: &str) -> String {
    (0..n).map(|i| format!("{tag}{i} ")).collect()
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// Read two files, rewrite one, run a check, commit; then one more request.
async fn run(scenario: Scenario) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    git(&root, &["config", "user.name", "T"]);
    std::fs::write(root.join("a.txt"), words(scenario.a_words, "alpha")).unwrap();
    std::fs::write(root.join("b.txt"), words(1_500, "beta")).unwrap();
    let mut script: Vec<Vec<forge_types::ToolCall>> = vec![
        vec![
            call("read_file", serde_json::json!({"path": "a.txt"})),
            call("read_file", serde_json::json!({"path": "b.txt"})),
            call(
                "update_tasks",
                serde_json::json!({"tasks": [
                    {"title": "add the regression test", "status": "pending"}
                ]}),
            ),
        ],
        vec![call(
            "write_file",
            serde_json::json!({"path": "a.txt", "content": "// the reworked file\n"}),
        )],
    ];
    let step = |command: &str| {
        if scenario.via_run_code {
            call(
                "run_code",
                serde_json::json!({"code": format!(
                    "const r = await tools.shell({{command: {}}});\nreturn r;",
                    serde_json::json!(command)
                )}),
            )
        } else {
            call("shell", serde_json::json!({ "command": command }))
        }
    };
    let check = match scenario.check {
        Check::Passes => Some("echo tests pass"),
        Check::Fails => Some("false # tests"),
        Check::Skipped => None,
    };
    let commit = "git add -A && git commit -q -m 'feat: rework a' -m 'because of the body line'";
    match (scenario.chained, check, scenario.commit) {
        (true, Some(check), true) => script.push(vec![step(&format!("{check} && {commit}"))]),
        _ => {
            if let Some(check) = check {
                script.push(vec![step(check)]);
            }
            if scenario.commit {
                script.push(vec![step(commit)]);
            }
        }
    }
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Scripted {
        script: Mutex::new(script.into()),
        requests: Mutex::default(),
        summarizer_calls: Default::default(),
    });
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.git.commit_nudge = false;
    config.compact.checkpoint = scenario.enabled;
    config.compact.checkpoint_min_tokens = scenario.min_tokens;
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let mut session = Session::start(
        Arc::clone(&store),
        provider.clone(),
        Arc::new(FixedRouter {
            model: "routed::session".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(&root),
        Box::new(capture),
        config,
        root.to_str().unwrap(),
    )
    .unwrap();
    session
        .run_turn("Rework a.txt and commit it")
        .await
        .unwrap();
    let warnings = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    Outcome {
        _dir: dir,
        root,
        store,
        session,
        provider,
        warnings,
    }
}

fn request_tokens(request: &[Message]) -> usize {
    request
        .iter()
        .map(|m| crate::tokens::count_message(&m.content))
        .sum()
}

fn checkpoint_lines(outcome: &Outcome) -> Vec<&String> {
    outcome
        .warnings
        .iter()
        .filter(|w| w.starts_with("checkpoint: "))
        .collect()
}

fn summary_of(outcome: &Outcome) -> Option<String> {
    outcome
        .session
        .transcript
        .iter()
        .find(|m| {
            m.content
                .starts_with(crate::compaction_summary::SUMMARY_PREFIX)
        })
        .map(|m| m.content.clone())
}

#[tokio::test]
async fn a_landed_slice_is_folded_once_and_the_next_step_starts_lean() {
    let outcome = run(Scenario::default()).await;
    let lines = checkpoint_lines(&outcome);
    assert_eq!(lines.len(), 1, "{:?}", outcome.warnings);
    let sha = {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&outcome.root)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    assert!(
        lines[0].starts_with(&format!("checkpoint: folded the slice behind {sha} (")),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains("k -> ") && lines[0].ends_with("k)"),
        "{}",
        lines[0]
    );
    assert_eq!(
        outcome
            .provider
            .summarizer_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no model writes a checkpoint"
    );

    let requests = outcome.provider.requests.lock().unwrap().clone();
    assert!(
        requests.len() >= 5,
        "three scripted steps, the commit, the answer"
    );
    let (commit_step, answer_step) = (&requests[3], &requests[4]);
    assert!(
        request_tokens(answer_step) < request_tokens(commit_step),
        "{} -> {}",
        request_tokens(commit_step),
        request_tokens(answer_step)
    );
    assert!(
        !answer_step.iter().any(|m| m.content.contains("beta1400")),
        "the folded read is gone from the prompt"
    );

    let summary = summary_of(&outcome).expect("the checkpoint summary");
    for needle in [
        CHECKPOINT_HEADING,
        &format!("Commit {sha}: feat: rework a"),
        "because of the body line",
        "### Diffstat",
        "a.txt",
        "`echo tests pass`",
        "[pending] add the regression test",
        EPISODES_HEADING,
        "`recall`",
        "Rework a.txt and commit it",
    ] {
        assert!(summary.contains(needle), "missing {needle:?} in\n{summary}");
    }
}

#[tokio::test]
async fn a_slice_landed_through_run_code_or_one_chained_command_is_folded() {
    for (label, via_run_code, chained) in [
        ("run_code, two scripts", true, false),
        ("one chained shell command", false, true),
        ("run_code, one chained script", true, true),
    ] {
        let outcome = run(Scenario {
            via_run_code,
            chained,
            ..Default::default()
        })
        .await;
        assert_eq!(
            checkpoint_lines(&outcome).len(),
            1,
            "{label}: {:?}",
            outcome.warnings
        );
        let summary = summary_of(&outcome).unwrap_or_default();
        assert!(summary.contains("`echo tests pass`"), "{label}\n{summary}");
    }
}

#[tokio::test]
async fn a_failed_check_in_a_chain_or_a_script_lands_nothing() {
    for (label, via_run_code, chained) in [
        ("run_code", true, false),
        ("chained", false, true),
        ("run_code chained", true, true),
    ] {
        let outcome = run(Scenario {
            check: Check::Fails,
            via_run_code,
            chained,
            ..Default::default()
        })
        .await;
        assert!(
            checkpoint_lines(&outcome).is_empty() && summary_of(&outcome).is_none(),
            "{label}: {:?}",
            outcome.warnings
        );
    }
}

#[tokio::test]
async fn recall_still_reaches_what_the_checkpoint_folded() {
    let outcome = run(Scenario::default()).await;
    assert!(summary_of(&outcome).is_some());
    let (text, ok) = crate::recall::run(
        &outcome.store,
        &outcome.session.id,
        &serde_json::json!({"action": "search", "query": "beta1400"}),
    );
    assert!(ok && text.contains("seq"), "{text}");
}

#[tokio::test]
async fn nothing_folds_without_a_passing_check_a_commit_room_or_the_switch() {
    for (label, scenario) in [
        (
            "failed check",
            Scenario {
                check: Check::Fails,
                ..Default::default()
            },
        ),
        (
            "no check",
            Scenario {
                check: Check::Skipped,
                ..Default::default()
            },
        ),
        (
            "no commit",
            Scenario {
                commit: false,
                ..Default::default()
            },
        ),
        (
            "below the threshold",
            Scenario {
                min_tokens: 500_000,
                ..Default::default()
            },
        ),
        (
            "disabled",
            Scenario {
                enabled: false,
                ..Default::default()
            },
        ),
    ] {
        let outcome = run(scenario).await;
        assert!(
            checkpoint_lines(&outcome).is_empty() && summary_of(&outcome).is_none(),
            "{label}: {:?}",
            outcome.warnings
        );
    }
}

#[tokio::test]
async fn superseded_reads_are_folded_first_and_may_be_enough() {
    let outcome = run(Scenario {
        min_tokens: 4_500,
        ..Default::default()
    })
    .await;
    let lines = checkpoint_lines(&outcome);
    assert_eq!(lines.len(), 1, "{:?}", outcome.warnings);
    assert!(lines[0].contains("superseded reads"), "{}", lines[0]);
    assert!(summary_of(&outcome).is_none(), "the slice itself stays");
    let answer = outcome.provider.requests.lock().unwrap().pop().unwrap();
    assert!(
        answer
            .iter()
            .any(|m| m.content.contains("[superseded: a.txt")),
        "the stale read says why it is gone"
    );
    assert!(
        answer.iter().any(|m| m.content.contains("beta1400")),
        "a read of a file nobody changed stays"
    );
}

fn shell(command: &str) -> forge_types::ToolCall {
    call("shell", serde_json::json!({ "command": command }))
}

#[test]
fn the_tracker_needs_a_pass_after_the_last_edit_before_the_commit() {
    let write = call(
        "write_file",
        serde_json::json!({"path": "src/a.rs", "content": "x"}),
    );
    let mut t = CheckpointTracker::default();
    t.observe(&write, true);
    t.observe(&shell("cargo test -p x"), true);
    t.observe(&shell("git add -A && git commit -m 'add tests'"), true);
    assert!(
        t.take_landed().is_some(),
        "a message that says tests is still a commit"
    );

    t.observe(&shell("cargo test"), true);
    t.observe(&write, true);
    t.observe(&shell("git commit -m x"), true);
    assert!(t.take_landed().is_none(), "the edit made the pass stale");

    t.observe(&write, true);
    t.observe(&shell("cargo test"), true);
    t.observe(&shell("cargo clippy"), false);
    t.observe(&shell("git commit -m x"), true);
    assert!(t.take_landed().is_none(), "a failing check is outstanding");

    t.observe(&shell("cargo test"), true);
    t.observe(&shell("git commit --dry-run"), true);
    t.observe(&shell("git commit -m x"), false);
    assert!(
        t.take_landed().is_none(),
        "dry runs and failed commits land nothing"
    );
}

#[test]
fn a_later_edit_unlands_a_commit() {
    let write = call(
        "write_file",
        serde_json::json!({"path": "a", "content": "x"}),
    );
    let mut t = CheckpointTracker::default();
    t.observe(&write, true);
    t.observe(&shell("pytest"), true);
    t.observe(&shell("git -C . commit -m x"), true);
    t.observe(&write, true);
    assert!(t.take_landed().is_none());
}

#[test]
fn only_reads_of_files_changed_later_are_folded() {
    let big = "x".repeat(2_000);
    let read = |id: &str, path: &str| {
        let mut call = call("read_file", serde_json::json!({"path": path}));
        call.id = id.into();
        Message::assistant_tool_calls("", vec![call])
    };
    let mut messages = vec![
        read("r1", "a.rs"),
        Message::tool_result("r1", &big),
        read("r2", "b.rs"),
        Message::tool_result("r2", &big),
        Message::assistant_tool_calls(
            "",
            vec![call("edit_file", serde_json::json!({"path": "./a.rs"}))],
        ),
        Message::tool_result("e1", "ok"),
        read("r3", "a.rs"),
        Message::tool_result("r3", &big),
    ];
    let released = fold_superseded_reads(&mut messages);
    assert!((1_500..2_000).contains(&released), "{released}");
    assert!(messages[1].content.starts_with("[superseded: a.rs"));
    assert_eq!(messages[3].content, big, "b.rs was never changed");
    assert_eq!(
        messages[7].content, big,
        "the read after the edit is current"
    );
}

#[test]
fn a_second_checkpoint_keeps_the_first_slice_and_the_model_written_summary_beneath() {
    use crate::compaction_checkpoint::{render_summary, Facts};
    let facts = |sha: &'static str, subject: &'static str, previous: Option<&str>| {
        render_summary(&Facts {
            sha,
            subject,
            body: "",
            diffstat: "",
            files: vec![("src/a.rs".into(), Some("the parser".into()))],
            checks: &["cargo test".to_string()],
            open_tasks: vec![],
            previous,
        })
    };
    let model_written = format!(
        "{}\n## 1. Primary request\nbuild the uploader",
        crate::compaction_summary::SUMMARY_PREFIX
    );
    let first = facts("aaa1111", "feat: first", Some(&model_written));
    assert!(first.contains("### Earlier summary (carried forward)\n## 1. Primary request"));
    assert!(first.contains("- src/a.rs: the parser"));
    let second = facts("bbb2222", "feat: second", Some(&first));
    assert!(second.contains("Commit bbb2222: feat: second"));
    assert!(second.contains("### Landed earlier this session\n- aaa1111 feat: first"));
    assert!(second.contains("build the uploader"), "{second}");
    assert_eq!(second.matches(CHECKPOINT_HEADING).count(), 1);
}

#[test]
fn a_check_chained_before_the_commit_counts_only_when_nothing_can_mask_it() {
    let write = call(
        "write_file",
        serde_json::json!({"path": "a.py", "content": "x"}),
    );
    let landed = |command: &str, ok: bool| {
        let mut t = CheckpointTracker::default();
        t.observe(&write, true);
        t.observe(&shell(command), ok);
        t.take_landed().is_some()
    };
    assert!(landed("pytest && git commit -m x", true));
    assert!(landed("cd w && python3 -m unittest tests.test_progress && git add -A && git commit -q -m 'slice done'", true));
    assert!(!landed("pytest && git commit -m x", false));
    assert!(!landed("git commit -m x", true), "no check at all");
    assert!(
        !landed("git commit -m x && pytest", true),
        "the check came after the commit"
    );
    assert!(
        !landed("pytest | tail -3 && git commit -m x", true),
        "a pipe hides the status"
    );
    assert!(!landed("pytest || true && git commit -m x", true));
    assert!(!landed("pytest; git commit -m x", true));
    assert!(!landed("git commit -m 'run the tests' && echo x", true));
}

#[test]
fn a_commit_made_inside_run_code_is_found_by_its_outer_call() {
    use crate::compaction_checkpoint::owns_call;
    assert!(owns_call("call_9", "call_9"));
    assert!(owns_call("call_9", "call_9.3"));
    assert!(!owns_call("call_9", "call_93"));
    assert!(!owns_call("call_9", "call_9.x"));
    assert!(!owns_call("call_9.1", "call_9"));
}
