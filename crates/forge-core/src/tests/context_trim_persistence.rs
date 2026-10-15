//! Trim, bloat and persistence correctness: harness messages are tagged and survive a resume,
//! the store keeps exactly the tail memory kept, `/uncompact` does not resurrect undone rows, and
//! a compaction that gives up still closes its progress band.

use super::*;
use forge_types::{MessageKind, ToolCall};

fn resumed(store: &Arc<Store>, sid: &str, provider: Arc<dyn Provider>) -> Session {
    Session::resume(
        Arc::clone(store),
        provider,
        Arc::new(HeuristicRouter::new(Config::default())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(HeadlessPresenter::new(false)),
        Config::default(),
        sid,
    )
    .unwrap()
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "read_file".into(),
        args: serde_json::json!({ "path": "a.rs" }),
    }
}

fn contents(session: &Session) -> Vec<(Role, String)> {
    session
        .transcript
        .iter()
        .map(|m| (m.role, m.content.clone()))
        .collect()
}

#[tokio::test]
async fn harness_nudges_and_notes_keep_their_kind_across_a_resume() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = fixed_session_in(
        Arc::clone(&store),
        Arc::new(SummarizingProvider),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
    );
    session.transcript.push(Message::user("real task"));
    session.push_nudge("Your last response was empty.").unwrap();
    session
        .push_note(
            MessageKind::Clock,
            "[time] Friday 2026-10-16 10:00:00 +02:00",
        )
        .unwrap();
    let kinds: Vec<MessageKind> = session.transcript.iter().map(|m| m.kind).collect();
    assert_eq!(
        kinds,
        vec![MessageKind::Normal, MessageKind::Nudge, MessageKind::Clock]
    );
    assert!(
        !session.transcript[1].is_real_user(),
        "a nudge is not the task"
    );
    assert!(session.transcript[0].is_real_user());

    let sid = session.id.clone();
    let back = resumed(&store, &sid, Arc::new(SummarizingProvider));
    let kinds: Vec<(Role, MessageKind)> =
        back.transcript.iter().map(|m| (m.role, m.kind)).collect();
    // The "real task" row was never persisted (pushed to memory only); the harness rows were.
    assert_eq!(
        kinds,
        vec![
            (Role::User, MessageKind::Nudge),
            (Role::System, MessageKind::Clock)
        ]
    );
}

#[tokio::test]
async fn the_store_keeps_exactly_the_tail_memory_kept_so_resume_matches_live() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let sid = store.create_session("/tmp", "default").unwrap();
    let mut seq = 0i64;
    let mut add = |role: Role, content: &str, calls: &[ToolCall], tool_call_id: Option<&str>| {
        store
            .add_message_full(&sid, seq, role, content, None, calls, tool_call_id)
            .unwrap();
        seq += 1;
    };
    for i in 0..10 {
        add(Role::User, &format!("question {i}"), &[], None);
        add(Role::Assistant, &format!("answer {i}"), &[], None);
    }
    // A last turn of three tool rounds: the keep-6 boundary falls on the first round's RESULT.
    add(Role::User, "now read three files", &[], None);
    for id in ["a", "b", "c"] {
        add(Role::Assistant, "", &[call(id)], None);
        add(Role::Tool, &format!("contents of {id}"), &[], Some(id));
    }
    add(Role::Assistant, "read them all", &[], None);

    let mut live = resumed(&store, &sid, Arc::new(SummarizingProvider));
    let (before, after) = live.compact(false).await.unwrap();
    assert!(after < before, "something was folded");

    let kept_first = live.transcript[1].clone();
    assert!(
        kept_first.role != Role::Tool,
        "the kept tail never opens on a tool result"
    );

    let again = resumed(&store, &sid, Arc::new(SummarizingProvider));
    assert_eq!(
        contents(&again),
        contents(&live),
        "a resume reproduces the post-compaction transcript exactly"
    );
    for (i, m) in again.transcript.iter().enumerate() {
        if m.role == Role::Tool {
            assert!(
                again.transcript[..i].iter().any(|c| c
                    .tool_calls
                    .iter()
                    .any(|t| Some(&t.id) == m.tool_call_id.as_ref())),
                "orphan tool result after resume: {}",
                m.content
            );
        }
    }
}

#[tokio::test]
async fn reloading_the_full_context_does_not_resurrect_undone_rows() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let sid = store.create_session("/tmp", "default").unwrap();
    for i in 0..10i64 {
        let role = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        store
            .add_message(&sid, i, role, &format!("msg {i}"), None)
            .unwrap();
    }
    store.compact_session_store(&sid, "summary", 6).unwrap();
    // `/undo` rewinds the last turn (seq 8-9) after the compaction.
    store.deactivate_messages_from(&sid, 8).unwrap();

    let mut session = resumed(&store, &sid, Arc::new(SummarizingProvider));
    session.reload_full_context().unwrap();

    let texts: Vec<String> = session
        .transcript
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        texts,
        (0..8).map(|i| format!("msg {i}")).collect::<Vec<_>>(),
        "compacted rows come back, undone rows stay gone"
    );
}

#[tokio::test]
async fn a_compaction_that_gives_up_still_closes_the_progress_band() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(SummarizingProvider),
        Arc::new(HeuristicRouter::new(Config::default())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        Config::default(),
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    // One long tool round: the round-aligned split falls before COMPACT_MIN_OLDER.
    session.transcript.push(Message::user("start"));
    session.transcript.push(Message::assistant("ok"));
    session.transcript.push(Message::user("read"));
    let ids: Vec<String> = (0..7).map(|i| format!("c{i}")).collect();
    session.transcript.push(Message::assistant_tool_calls(
        "",
        ids.iter().map(|id| call(id)).collect(),
    ));
    for id in &ids {
        session
            .transcript
            .push(Message::tool_result(id.clone(), "data"));
    }

    let (before, after) = session.compact(true).await.unwrap();
    assert_eq!(before, after, "nothing was folded");

    let events = events.lock().unwrap();
    let started = events
        .iter()
        .filter(|e| matches!(e, PresenterEvent::CompactionStarted { .. }))
        .count();
    let finished = events
        .iter()
        .filter(|e| matches!(e, PresenterEvent::CompactionFinished { .. }))
        .count();
    assert_eq!((started, finished), (1, 1), "every start has a finish");
}

/// A router that calls every model free, so the free-model ceiling is the one in force.
struct FreeRouter(FixedRouter);

#[async_trait::async_trait]
impl Router for FreeRouter {
    async fn route(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &forge_types::ModelHealth,
        quota: &forge_types::SubscriptionQuota,
        effort: Option<forge_types::EffortLevel>,
        project: &forge_types::ProjectContext,
    ) -> forge_mesh::RoutingDecision {
        self.0
            .route(prompt, has_images, budget, health, quota, effort, project)
            .await
    }

    fn model_is_free(&self, _model: &str) -> bool {
        true
    }
}

#[test]
fn the_idle_auto_compact_trigger_is_the_sessions_own_for_the_pinned_model() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = Config::default();
    config.mesh.compact_cap_tokens = 50_000;
    config.mesh.free_model_cap_tokens = 300_000;
    config
        .mesh
        .context_windows
        .insert("free::big".to_string(), 1_000_000);
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(SummarizingProvider),
        Arc::new(FreeRouter(FixedRouter {
            model: "free::big".into(),
            fallbacks: vec![],
        })),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(HeadlessPresenter::new(false)),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();

    // No model has run yet: the global (paid) ceiling.
    assert_eq!(session.idle_compaction_trigger(1_000_000), 50_000);

    session.pin_model(Some("free::big".to_string()));
    assert_eq!(
        session.idle_compaction_trigger(1_000_000),
        session.compaction_trigger("free::big"),
        "the idle check and the turn loop agree"
    );
    assert_eq!(
        session.idle_compaction_trigger(1_000_000),
        300_000,
        "a free model compacts at the free ceiling, not the spend cap"
    );
}

#[tokio::test]
async fn a_window_trim_that_would_drop_history_compacts_first_and_warns_once() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut config = Config::default();
    config
        .mesh
        .context_windows
        .insert("direct::scripted".to_string(), 30_000);
    config.mesh.max_output_tokens = 1_000;
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(SummarizingProvider),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    let filler = "alpha bravo charlie delta echo foxtrot ".repeat(100);
    for i in 0..24 {
        session
            .transcript
            .push(Message::user(format!("question {i} {filler}")));
        session
            .transcript
            .push(Message::assistant(format!("answer {i} {filler}")));
    }

    let before = session.transcript_with_preamble_and_tools("direct::scripted", &[]);
    assert!(session.trim_calls_for_compaction(), "history was dropped");
    assert!(before.len() < session.transcript.len());

    assert!(session.compact_instead_of_trim("direct::scripted").await);
    let after = session.transcript_with_preamble_and_tools("direct::scripted", &[]);
    assert!(
        !session.trim_calls_for_compaction(),
        "the summary made the request fit without dropping history"
    );
    assert!(after.iter().any(|m| m.content.contains("SUMMARY:")));

    // The notice names the trim once per turn, however many requests hit it.
    session.trim_watch.last.set(context_floor::TrimDrop {
        messages: 5,
        history: 3,
    });
    session.note_window_trim();
    session.note_window_trim();
    let notices = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("context window full")))
        .count();
    assert_eq!(notices, 1);
}

#[tokio::test]
async fn a_compaction_that_folds_nothing_is_not_retried_for_every_request() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = Config::default();
    config
        .mesh
        .context_windows
        .insert("direct::scripted".to_string(), 4_000);
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(SummarizingProvider),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(HeadlessPresenter::new(false)),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    // Too few messages to compact, but each is far too big for the window.
    let filler = "alpha bravo charlie delta echo foxtrot ".repeat(400);
    for i in 0..4 {
        session
            .transcript
            .push(Message::user(format!("question {i} {filler}")));
        session
            .transcript
            .push(Message::assistant(format!("answer {i} {filler}")));
    }
    session.transcript_with_preamble_and_tools("direct::scripted", &[]);
    assert!(session.trim_calls_for_compaction());

    assert!(!session.compact_instead_of_trim("direct::scripted").await);
    session.transcript_with_preamble_and_tools("direct::scripted", &[]);
    assert!(
        !session.trim_calls_for_compaction(),
        "a stalled compaction is not asked for again this turn"
    );
}
