//! `[mesh] harness_profile` through real turns: `minimal` silences every advisory nudge while each
//! safety guard (permission broker, the progress ledger's stops, phantom-edit gate, an explicit
//! step limit, spend cap) still fires; the profile follows the model driving each step; and the
//! empty-reply nudge is spent at most once per turn.

use super::*;
use forge_config::HarnessProfile;

/// Replays `(content, tool_calls)`, repeating the last entry once the script runs out.
struct Script {
    steps: Vec<(String, Vec<forge_types::ToolCall>)>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl Script {
    fn new(steps: Vec<(String, Vec<forge_types::ToolCall>)>) -> Self {
        Self {
            steps,
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Script {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, tool_calls) = self
            .steps
            .get(n)
            .or_else(|| self.steps.last())
            .cloned()
            .unwrap_or_default();
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

fn call(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: forge_types::new_id(),
        name: name.into(),
        args,
    }
}

fn say(text: &str) -> (String, Vec<forge_types::ToolCall>) {
    (text.to_string(), Vec::new())
}

fn config(profile: HarnessProfile) -> Config {
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.mesh.rate_limit_wait_secs = 0;
    config.shell.explain_errors = false;
    config.mesh.harness_profile = profile;
    config
}

struct Run {
    session: Session,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
}

impl Run {
    fn new(provider: Arc<dyn Provider>, config: Config, root: &std::path::Path) -> Self {
        Self::routed(provider, config, root, "direct::scripted", vec![])
    }

    fn routed(
        provider: Arc<dyn Provider>,
        config: Config,
        root: &std::path::Path,
        model: &str,
        fallbacks: Vec<String>,
    ) -> Self {
        let capture = CapturePresenter {
            attended: true,
            ..Default::default()
        };
        let events = capture.events.clone();
        let session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            provider,
            Arc::new(FixedRouter {
                model: model.into(),
                fallbacks,
            }),
            ToolRegistry::with_core_tools_in(root),
            Box::new(capture),
            config,
            root.to_str().unwrap(),
        )
        .unwrap();
        Self { session, events }
    }

    fn warnings(&self, needle: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| {
                matches!(e, PresenterEvent::Warning(w) | PresenterEvent::Error(w) if w.contains(needle))
            })
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

#[test]
fn the_profile_follows_the_model_and_defaults_to_guarded() {
    let mut cfg = config(HarnessProfile::Guarded);
    cfg.mesh
        .harness_profiles
        .insert("bedrock::*".into(), HarnessProfile::Minimal);
    let mut run = Run::new(Arc::new(Script::new(vec![])), cfg, test_workspace());
    assert!(run.session.advisory_nudges_on(), "guarded before any turn");
    run.session
        .note_harness_model("bedrock::global.anthropic.claude");
    assert!(!run.session.advisory_nudges_on());
    assert!(!run.session.commit_nudges_on());
    run.session.note_harness_model("ollama::qwen");
    assert!(run.session.advisory_nudges_on());
}

#[tokio::test]
async fn minimal_keeps_the_progress_line_but_drops_the_silence_ask() {
    let mut steps: Vec<_> = (0..30)
        .map(|n| {
            (
                String::new(),
                vec![call(
                    "shell",
                    serde_json::json!({ "command": format!("echo step-{n}") }),
                )],
            )
        })
        .collect();
    steps.push(say("Done."));
    let mut run = Run::new(
        Arc::new(Script::new(steps)),
        config(HarnessProfile::Minimal),
        test_workspace(),
    );
    run.session.run_turn("inspect the build").await.unwrap();
    assert_eq!(run.system_notes("without a word to the user"), 0);
    let progress = run
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PresenterEvent::Progress(_)))
        .count();
    assert_eq!(progress, 2, "the user still sees the stretch summary");
}

fn rereads() -> Vec<(String, Vec<forge_types::ToolCall>)> {
    (0..40)
        .map(|n| {
            (
                "You're right — I looped. Answering the question from evidence, then finishing \
                 the revert I left half-done."
                    .to_string(),
                vec![call(
                    "read_file",
                    serde_json::json!({"path": "Cargo.toml", "limit": 5 + n}),
                )],
            )
        })
        .collect()
}

#[tokio::test]
async fn minimal_sends_no_stall_nudge_but_still_halts_the_loop() {
    let provider = Arc::new(Script::new(rereads()));
    let calls = Arc::clone(&provider.calls);
    let mut run = Run::new(provider, config(HarnessProfile::Minimal), test_workspace());
    run.session.run_turn("why does it loop?").await.unwrap();
    assert_eq!(run.system_notes(crate::stall_guard::STALL_NUDGE), 0);
    assert_eq!(run.system_notes("Progress check from Forge"), 0);
    assert_eq!(run.warnings("nudging"), 0);
    assert_eq!(run.warnings("Stopped:"), 1);
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) < 40);
}

#[tokio::test]
async fn guarded_still_sends_the_stall_nudge() {
    let mut run = Run::new(
        Arc::new(Script::new(rereads())),
        config(HarnessProfile::Guarded),
        test_workspace(),
    );
    run.session.run_turn("why does it loop?").await.unwrap();
    assert_eq!(run.system_notes(crate::stall_guard::STALL_NUDGE), 1);
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

async fn git_notes_after_two_turns(profile: HarnessProfile) -> usize {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("README.md"), "hi\n").unwrap();
    git(dir.path(), &["add", "README.md"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    let mut steps: Vec<_> = (0..3)
        .map(|n| {
            (
                String::new(),
                vec![call(
                    "write_file",
                    serde_json::json!({"path": "notes.md", "content": format!("edit {n}\n")}),
                )],
            )
        })
        .collect();
    steps.push(say("done"));
    let mut cfg = config(profile);
    cfg.git.commit_nudge_edits = 1;
    let mut run = Run::new(Arc::new(Script::new(steps)), cfg, dir.path());
    run.session.run_turn("take notes").await.unwrap();
    run.session.run_turn("more").await.unwrap();
    run.session
        .transcript
        .iter()
        .filter(|m| m.role == Role::System && m.content.starts_with("[git]"))
        .count()
}

#[tokio::test]
async fn minimal_drops_both_commit_reminders() {
    assert!(git_notes_after_two_turns(HarnessProfile::Guarded).await > 1);
    assert_eq!(git_notes_after_two_turns(HarnessProfile::Minimal).await, 0);
}

async fn orchestration_notes(profile: HarnessProfile) -> usize {
    let mut cfg = config(profile);
    cfg.mesh.auto_orchestrate = true;
    let mut run = Run::new(
        Arc::new(Script::new(vec![say("Planned.")])),
        cfg,
        test_workspace(),
    );
    run.session
        .run_turn(
            "Split the session loop into request, tool and persistence modules. Add regression \
             tests for each of them. Update the feature docs to describe the new layout.",
        )
        .await
        .unwrap();
    run.system_notes("Forge auto-orchestrate is active")
}

#[tokio::test]
async fn minimal_skips_the_auto_orchestrate_prompt() {
    assert_eq!(orchestration_notes(HarnessProfile::Guarded).await, 1);
    assert_eq!(orchestration_notes(HarnessProfile::Minimal).await, 0);
}

async fn mid_intent(profile: HarnessProfile) -> (usize, String) {
    let mut run = Run::new(
        Arc::new(Script::new(vec![say(
            "Now let me look at the workspace layout:",
        )])),
        config(profile),
        test_workspace(),
    );
    let outcome = run.session.run_turn("look around").await.unwrap();
    (run.warnings("announced another action"), outcome.text)
}

#[tokio::test]
async fn minimal_accepts_a_mid_intent_ending_without_a_nudge() {
    assert_eq!(mid_intent(HarnessProfile::Guarded).await.0, 2);
    let (nudges, text) = mid_intent(HarnessProfile::Minimal).await;
    assert_eq!(nudges, 0);
    assert_eq!(text, "Now let me look at the workspace layout:");
}

fn open_task(title: &str) -> forge_types::TodoItem {
    forge_types::TodoItem {
        title: title.into(),
        status: forge_types::TodoStatus::Pending,
        assignee: None,
    }
}

async fn continue_nudges(profile: HarnessProfile) -> (usize, String) {
    let mut run = Run::new(
        Arc::new(Script::new(vec![say(
            "That covers the first part of the plan.",
        )])),
        config(profile),
        test_workspace(),
    );
    run.session.tasks = vec![open_task("write the parser"), open_task("wire it in")];
    let outcome = run.session.run_turn("work through the plan").await.unwrap();
    (run.warnings("continuing it (nudge"), outcome.text)
}

#[tokio::test]
async fn minimal_sends_no_continue_nudge_for_open_tasks() {
    assert!(continue_nudges(HarnessProfile::Guarded).await.0 > 0);
    let (nudges, text) = continue_nudges(HarnessProfile::Minimal).await;
    assert_eq!(nudges, 0);
    assert_eq!(text, "That covers the first part of the plan.");
}

#[tokio::test]
async fn minimal_still_halts_a_doom_loop() {
    let mut run = Run::new(
        Arc::new(Script::new(vec![(
            "let me read it again".into(),
            vec![call("read_file", serde_json::json!({"path": "Cargo.toml"}))],
        )])),
        config(HarnessProfile::Minimal),
        test_workspace(),
    );
    run.session.run_turn("read the file").await.unwrap();
    assert!(run.warnings("Stopped: the same calls returning the same results") > 0);
    assert_eq!(run.warnings("progress check"), 0, "minimal sends no nudge");
}

#[tokio::test]
async fn minimal_still_halts_a_failure_loop() {
    let steps = (0..60)
        .map(|n| {
            (
                "let me try a different file".to_string(),
                vec![call(
                    "read_file",
                    serde_json::json!({"path": format!("does-not-exist-{n}.rs")}),
                )],
            )
        })
        .collect();
    let mut run = Run::new(
        Arc::new(Script::new(steps)),
        config(HarnessProfile::Minimal),
        test_workspace(),
    );
    run.session.run_turn("find the config").await.unwrap();
    assert!(run.warnings("Stopped: the same calls failing") > 0);
}

#[tokio::test]
async fn minimal_still_runs_the_phantom_edit_gate_even_with_open_tasks() {
    let provider = Arc::new(Script::new(vec![say(
        "I've created the file and updated the workflow.",
    )]));
    let calls = Arc::clone(&provider.calls);
    let mut run = Run::new(provider, config(HarnessProfile::Minimal), test_workspace());
    run.session.tasks = vec![open_task("update CI")];
    run.session
        .run_turn("create a file and update CI")
        .await
        .unwrap();
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 1);
    assert!(run.warnings("no mutating tool ran") > 0);
}

#[tokio::test]
async fn minimal_still_stops_at_the_step_cap() {
    let mut cfg = config(HarnessProfile::Minimal);
    cfg.permission_mode = PermissionMode::Default;
    cfg.mesh.max_steps = 2;
    cfg.mesh.max_steps_unattended = 5;
    let (provider, calls) = EndlessToolProvider::new(7);
    let mut run = Run::new(Arc::new(provider), cfg, test_workspace());
    run.session.run_turn("keep reading").await.unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert!(run.warnings("2-step limit") > 0);
}

#[tokio::test]
async fn minimal_still_enforces_the_turn_spend_cap() {
    const MODEL: &str = "ollama::spend-test";
    let mut cfg = config(HarnessProfile::Minimal);
    cfg.mesh.max_steps = 12;
    cfg.mesh.max_steps_unattended = 12;
    cfg.mesh.max_turn_input_tokens = 0;
    cfg.mesh.pricing.insert(
        MODEL.into(),
        forge_config::PriceOverride {
            input_per_1k: 1.0,
            output_per_1k: 0.0,
            cache_read_per_1k: None,
        },
    );
    cfg.mesh.budget.turn_warn_usd = 2.0;
    cfg.mesh.budget.turn_cap_usd = 4.0;
    cfg.mesh.budget.session_warn_usd = 0.0;
    cfg.mesh.budget.session_cap_usd = 0.0;
    let (provider, calls) = EndlessToolProvider::new(1000);
    let mut run = Run::routed(Arc::new(provider), cfg, test_workspace(), MODEL, vec![]);
    run.session
        .set_catalog(Some(ModelCatalog::new(vec![MODEL.into()])));
    run.session.run_turn("keep reading").await.unwrap();
    assert!(run.warnings("turn spend cap") > 0);
    assert!(calls.load(std::sync::atomic::Ordering::Relaxed) < 12);
}

#[tokio::test]
async fn minimal_still_asks_the_permission_broker() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(HarnessProfile::Minimal);
    cfg.permission_mode = PermissionMode::Default;
    let mut run = Run::new(
        Arc::new(Script::new(vec![
            (
                String::new(),
                vec![call(
                    "write_file",
                    serde_json::json!({"path": "out.txt", "content": "x"}),
                )],
            ),
            say("Tried."),
        ])),
        cfg,
        dir.path(),
    );
    run.session.run_turn("write out.txt").await.unwrap();
    assert!(
        !dir.path().join("out.txt").exists(),
        "the presenter denied the write; minimal must not bypass the broker"
    );
}

/// Empty for every `empty::` model, a mid-intent ending for anything else.
struct EmptyThenMidIntent;

#[async_trait::async_trait]
impl Provider for EmptyThenMidIntent {
    async fn complete(
        &self,
        model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let content = if model.starts_with("empty::") {
            String::new()
        } else {
            "Now let me look at the workspace layout:".to_string()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn the_profile_is_resolved_again_after_a_failover() {
    let mut cfg = config(HarnessProfile::Guarded);
    cfg.mesh
        .harness_profiles
        .insert("good::*".into(), HarnessProfile::Minimal);
    let mut run = Run::routed(
        Arc::new(EmptyThenMidIntent),
        cfg,
        test_workspace(),
        "empty::model",
        vec!["good::model".into()],
    );
    let outcome = run.session.run_turn("look around").await.unwrap();
    assert_eq!(outcome.text, "Now let me look at the workspace layout:");
    assert_eq!(
        run.warnings("announced another action"),
        0,
        "the fallback model is minimal, so its mid-intent ending stands"
    );
    assert_eq!(run.warnings("model returned an empty response"), 1);
}

#[tokio::test]
async fn the_empty_reply_nudge_re_arms_on_the_next_turn() {
    let mut run = Run::new(
        Arc::new(Script::new(vec![say("")])),
        config(HarnessProfile::Guarded),
        test_workspace(),
    );
    run.session.run_turn("do the task").await.unwrap();
    assert_eq!(run.warnings("model returned an empty response"), 1);
    run.session.run_turn("try again").await.unwrap();
    assert_eq!(
        run.warnings("model returned an empty response"),
        2,
        "a new turn gets its own nudge"
    );
    let nudges = run
        .session
        .transcript
        .iter()
        .filter(|m| m.content == crate::empty_reply::EMPTY_REPLY_NUDGE)
        .count();
    assert_eq!(nudges, 2);
}

#[tokio::test]
async fn tool_progress_re_arms_the_empty_reply_nudge() {
    let mut run = Run::new(
        Arc::new(Script::new(vec![
            say(""),
            (
                String::new(),
                vec![call("list_dir", serde_json::json!({"path": "."}))],
            ),
            say(""),
            say("Done."),
        ])),
        config(HarnessProfile::Guarded),
        test_workspace(),
    );
    let outcome = run.session.run_turn("do the task").await.unwrap();
    assert_eq!(outcome.text, "Done.");
    assert_eq!(
        run.warnings("model returned an empty response"),
        2,
        "a transient empty reply after real work gets its own nudge instead of a bench"
    );
    assert_eq!(run.warnings("pinned"), 0);
}

#[tokio::test]
async fn an_empty_reply_right_after_the_nudge_goes_to_the_honest_failure_path() {
    let mut run = Run::new(
        Arc::new(Script::new(vec![say(""), say(""), say("Late answer.")])),
        config(HarnessProfile::Guarded),
        test_workspace(),
    );
    let outcome = run.session.run_turn("do the task").await.unwrap();
    assert_eq!(run.warnings("model returned an empty response"), 1);
    assert_ne!(
        outcome.text, "Late answer.",
        "the second empty reply in a row stops the pinned-equivalent turn"
    );
}
