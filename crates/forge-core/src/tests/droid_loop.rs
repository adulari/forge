//! Factory Droid loop controls (`[harness]` flags, default off): stream-silence retry, shell_job
//! wait, append-only prefix, cache-break warnings and the compaction summary cap.

use super::harness_profile_tests::{harness_config, say, transcript_hits, Run, Script};
use super::pinned_context_tests::{quiet_config, start, workspace, Recorder};
use super::*;
use forge_config::HarnessProfile;
use forge_provider::wire_fingerprint::fingerprint;

/// Goes quiet on its first `silent` calls (no event, no return), then answers.
struct SilentThenAnswer {
    silent: usize,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for SilentThenAnswer {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n < self.silent {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
        Ok(forge_provider::ModelResponse {
            content: "answered".into(),
            ..Default::default()
        })
    }
}

/// Streams reasoning for longer than the idle window, then answers.
struct LongReasoning;

#[async_trait::async_trait]
impl Provider for LongReasoning {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        for _ in 0..5 {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            on_event(StreamEvent::Reasoning("thinking".into()));
        }
        Ok(forge_provider::ModelResponse {
            content: "reasoned".into(),
            ..Default::default()
        })
    }
}

/// The silence floor is process-wide, so the tests that move it take turns.
static SILENCE_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn silence_run(provider: Arc<dyn Provider>, on: bool, idle: u64, floor: u64) -> Run {
    crate::droid_loop::SILENCE_FLOOR_OVERRIDE.store(floor, std::sync::atomic::Ordering::Relaxed);
    let mut config = harness_config(HarnessProfile::Guarded, |h| h.stream_silence_retry = on);
    config.mesh.stream_idle_timeout_secs = idle;
    config.mesh.first_event_timeout_secs = 0;
    config.mesh.pin_outage_wait_secs = 0;
    Run::new(provider, config, test_workspace())
}

#[tokio::test]
async fn a_silent_stream_is_retried_on_the_same_model_when_the_flag_is_on() {
    let _turn = SILENCE_TURN.lock().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut run = silence_run(
        Arc::new(SilentThenAnswer {
            silent: 1,
            calls: calls.clone(),
        }),
        true,
        1,
        1,
    );
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run.session.run_turn("go"),
    )
    .await
    .expect("no hang")
    .expect("the retry answers");
    assert_eq!(out, "answered");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(run.warnings("retrying direct::scripted (1/2)"), 1);
}

#[tokio::test]
async fn silence_retries_are_bounded_and_the_turn_then_fails() {
    let _turn = SILENCE_TURN.lock().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut run = silence_run(
        Arc::new(SilentThenAnswer {
            silent: usize::MAX,
            calls: calls.clone(),
        }),
        true,
        1,
        1,
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run.session.run_turn("go"),
    )
    .await
    .expect("no hang");
    assert!(result.is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn without_the_flag_a_stall_is_not_retried_on_the_same_model() {
    let _turn = SILENCE_TURN.lock().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut run = silence_run(
        Arc::new(SilentThenAnswer {
            silent: 1,
            calls: calls.clone(),
        }),
        false,
        1,
        1,
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run.session.run_turn("go"),
    )
    .await
    .expect("no hang");
    assert!(result.is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_long_reasoning_stream_is_never_cut_by_the_silence_window() {
    let _turn = SILENCE_TURN.lock().await;
    let mut run = silence_run(Arc::new(LongReasoning), true, 1, 2);
    let out = run.session.run_turn("think hard").await.unwrap();
    assert_eq!(out, "reasoned");
    assert_eq!(run.warnings("retrying"), 0);
}

#[tokio::test]
async fn three_empty_replies_stop_the_turn_under_stop_retries() {
    let provider = Arc::new(Script::new(vec![say("")]));
    let calls = Arc::clone(&provider.calls);
    let mut run = Run::new(
        provider,
        harness_config(HarnessProfile::Guarded, |h| h.stop_retries = true),
        test_workspace(),
    );
    let _ = run.session.run_turn("do it").await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert_eq!(
        transcript_hits(&run, crate::stop_guards::EMPTY_STOP_NUDGE),
        3
    );
}

#[tokio::test]
async fn identical_calls_are_nudged_at_the_third_and_stopped_by_the_ledger() {
    let steps: Vec<_> = (0..20)
        .map(|_| {
            (
                String::new(),
                vec![super::harness_profile_tests::call(
                    "shell",
                    serde_json::json!({"command": "echo same"}),
                )],
            )
        })
        .collect();
    let provider = Arc::new(Script::new(steps));
    let calls = Arc::clone(&provider.calls);
    let mut run = Run::new(
        provider,
        harness_config(HarnessProfile::Guarded, |_| {}),
        test_workspace(),
    );
    run.session.run_turn("check it").await.unwrap();
    let n = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!((4..=6).contains(&n), "stopped after {n} identical calls");
}

fn task(title: &str, status: forge_types::TodoStatus) -> forge_types::TodoItem {
    forge_types::TodoItem {
        title: title.into(),
        status,
        assignee: None,
    }
}

/// Two turns with a task list that changes between them; how much of turn one's last request
/// survives at the front of turn two's first request.
async fn reuse_across_a_changed_task_list(on: bool) -> (bool, Vec<Message>) {
    let dir = workspace(Some("# rules\nbe terse\n"));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut config = quiet_config();
    config.harness.append_only_prefix = on;
    let mut session = start(&store, &dir, &provider, config);
    session.tasks = vec![task("write it", forge_types::TodoStatus::InProgress)];
    session.run_turn("first").await.unwrap();
    let before = provider.requests().len();
    session.tasks = vec![
        task("write it", forge_types::TodoStatus::Done),
        task("test it", forge_types::TodoStatus::InProgress),
    ];
    session.run_turn("second").await.unwrap();
    let requests = provider.requests();
    let prev = fingerprint(&requests[before - 1], &[]);
    let next = fingerprint(&requests[before], &[]);
    let _ = std::fs::remove_dir_all(&dir);
    (
        next.reuse_against(&prev).is_append(),
        requests[before].clone(),
    )
}

#[tokio::test]
async fn a_changed_task_list_breaks_the_cached_prefix_only_without_the_flag() {
    let (off_append, _) = reuse_across_a_changed_task_list(false).await;
    let (on_append, request) = reuse_across_a_changed_task_list(true).await;
    assert!(
        !off_append,
        "the pinned block moved in front of the transcript"
    );
    assert!(on_append, "the transcript only grew");
    let block = request
        .iter()
        .find(|m| m.kind == forge_types::MessageKind::Pinned)
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(
        block.contains("be terse"),
        "instructions stay pinned: {block}"
    );
    assert!(!block.contains("## Task list"), "{block}");
    let snapshot = request
        .iter()
        .filter(|m| {
            m.content
                .starts_with(crate::droid_loop::TASK_SNAPSHOT_PREFIX)
        })
        .count();
    assert_eq!(snapshot, 2, "one snapshot per distinct list");
    assert!(request
        .iter()
        .any(|m| m.content.contains("- [in progress] test it")));
}

#[tokio::test]
async fn an_unchanged_task_list_adds_no_second_snapshot() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut config = quiet_config();
    config.harness.append_only_prefix = true;
    let mut session = start(&store, &dir, &provider, config);
    session.tasks = vec![task("write it", forge_types::TodoStatus::InProgress)];
    session.run_turn("first").await.unwrap();
    session.run_turn("second").await.unwrap();
    let n = provider
        .last()
        .iter()
        .filter(|m| {
            m.content
                .starts_with(crate::droid_loop::TASK_SNAPSHOT_PREFIX)
        })
        .count();
    assert_eq!(n, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_cache_break_warns_at_once_with_its_cause_only_under_the_monitor() {
    async fn warnings(monitor: bool) -> usize {
        let dir = workspace(Some("# rules\nv1\n"));
        let store = Arc::new(Store::open_in_memory().unwrap());
        let provider = Arc::new(Recorder::default());
        let mut config = quiet_config();
        config.harness.cache_monitor = monitor;
        let capture = CapturePresenter {
            attended: true,
            ..Default::default()
        };
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            provider.clone(),
            Arc::new(FixedRouter {
                model: "m::x".into(),
                fallbacks: vec![],
            }),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(capture),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        session.tasks = vec![task("a", forge_types::TodoStatus::Pending)];
        session.run_turn("one").await.unwrap();
        session.tasks = vec![task("b", forge_types::TodoStatus::Pending)];
        session.run_turn("two").await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let events = events.lock().unwrap();
        events
            .iter()
            .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("prompt cache broken") && w.contains("system message")))
            .count()
    }
    assert_eq!(warnings(false).await, 0);
    assert_eq!(warnings(true).await, 1);
}

#[test]
fn the_wait_action_is_advertised_only_under_the_flag() {
    let run = |on: bool| {
        let mut config = harness_config(HarnessProfile::Guarded, |h| h.wait_job = on);
        config.mesh.direct_lean = false;
        let run = Run::new(
            Arc::new(Script::new(vec![say("ok")])),
            config,
            test_workspace(),
        );
        run.session
            .full_tool_specs()
            .into_iter()
            .find(|s| s.name == "shell_job")
            .expect("shell_job")
    };
    let off = run(false);
    let on = run(true);
    assert!(!off.schema.to_string().contains("wait"));
    assert!(on.schema.to_string().contains("\"wait\""));
    assert!(on.schema.to_string().contains("timeout_secs"));
    assert!(on.description.starts_with("Wait for a background job"));
}
