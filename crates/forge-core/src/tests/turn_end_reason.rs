//! Every turn leaves a `turn` row with the reason it ended, and a turn that did not end in an
//! answer leaves a note saying why (docs/features/session-replay.md, "Turn endings").

use super::*;

enum Step {
    Reply(&'static str, Vec<forge_types::ToolCall>),
    Fail(forge_provider::ProviderError),
    Hang,
}

struct Script {
    steps: Mutex<Vec<Step>>,
    steer: Option<(usize, crate::steer::SteerInbox)>,
    calls: std::sync::atomic::AtomicUsize,
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
        if let Some((at, inbox)) = &self.steer {
            if *at == n {
                inbox.push("also rename the helper");
            }
        }
        let step = {
            let mut steps = self.steps.lock().unwrap();
            match steps.first() {
                Some(Step::Reply(t, c)) if steps.len() == 1 => Step::Reply(t, c.clone()),
                Some(Step::Hang) | None => Step::Hang,
                Some(_) => steps.remove(0),
            }
        };
        match step {
            Step::Reply(content, tool_calls) => Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: content.to_string(),
                tool_calls,
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            }),
            Step::Fail(error) => Err(error),
            Step::Hang => std::future::pending().await,
        }
    }
}

fn list_dir() -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: forge_types::new_id(),
        name: "list_dir".into(),
        args: serde_json::json!({"path": "."}),
    }
}

fn fixture(steps: Vec<Step>, steer_at: Option<usize>) -> (Session, Arc<Store>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let inbox = crate::steer::SteerInbox::default();
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(Script {
            steps: Mutex::new(steps),
            steer: steer_at.map(|at| (at, inbox.clone())),
            calls: Default::default(),
        }),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    session.steer = inbox;
    (session, store)
}

fn turns(store: &Store, session: &Session) -> Vec<forge_store::TurnRow> {
    store.recent_turns(&session.id, 20).unwrap()
}

fn newest_ui_note(store: &Store, session: &Session) -> Option<String> {
    store
        .load_messages(&session.id)
        .unwrap()
        .into_iter()
        .rev()
        .find(|m| m.role == Role::System)
        .map(|m| m.content)
}

#[tokio::test]
async fn a_final_answer_is_recorded_without_a_note() {
    let (mut session, store) = fixture(vec![Step::Reply("All done.", vec![])], None);
    session.run_turn("do it").await.unwrap();
    let rows = turns(&store, &session);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].end_reason.as_deref(), Some("final_answer"));
    assert_eq!(rows[0].last_role_shape.as_deref(), Some("finished"));
    assert_eq!(rows[0].prompt_seq, 0);
    assert!(!newest_ui_note(&store, &session)
        .unwrap_or_default()
        .contains("Turn stopped"));
}

#[tokio::test]
async fn an_answer_that_announces_unfinished_work_is_flagged() {
    let (mut session, store) = fixture(
        vec![Step::Reply("Let me check the config now.", vec![])],
        None,
    );
    session.run_turn("do it").await.unwrap();
    let reason = turns(&store, &session)[0].end_reason.clone().unwrap();
    assert!(
        reason == "final_answer_mid_intent" || reason == "final_answer",
        "{reason}"
    );
    assert!(crate::mid_intent::ends_mid_intent(
        "Let me check the config now."
    ));
}

#[tokio::test]
async fn an_answer_that_left_a_step_undone_leaves_one_stop_note_and_line() {
    let (mut session, store) = fixture(
        vec![Step::Reply("Let me check the config now.", vec![])],
        None,
    );
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    session.presenter = Box::new(capture);
    session.run_turn("do it").await.unwrap();
    assert_eq!(
        turns(&store, &session)[0].end_reason.as_deref(),
        Some("final_answer_mid_intent")
    );
    let note = newest_ui_note(&store, &session).unwrap_or_default();
    assert!(
        note.starts_with("⚠ Turn stopped (answer left a step undone)"),
        "{note}"
    );
    let lines = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PresenterEvent::TurnStopped { .. }))
        .count();
    assert_eq!(lines, 1);
}

#[tokio::test]
async fn the_surface_hears_the_stored_end_reason_before_done() {
    let (mut session, store) = fixture(
        vec![Step::Reply("Let me check the config now.", vec![])],
        None,
    );
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let heard = capture.end_reasons.clone();
    session.presenter = Box::new(capture);
    session.run_turn("do it").await.unwrap();
    let stored = turns(&store, &session)[0].end_reason.clone().unwrap();
    assert_eq!(heard.lock().unwrap().last(), Some(&stored));
}

#[tokio::test]
async fn a_provider_error_after_a_tool_step_closes_the_row_and_leaves_a_note() {
    let (mut session, store) = fixture(
        vec![
            Step::Reply("", vec![list_dir()]),
            Step::Fail(forge_provider::ProviderError::Request("bad request".into())),
        ],
        None,
    );
    assert!(session.run_turn("list it").await.is_err());
    let row = &turns(&store, &session)[0];
    assert_eq!(row.end_reason.as_deref(), Some("provider_error"));
    assert_eq!(
        row.last_role_shape.as_deref(),
        Some("unanswered_tool_results")
    );
    let detail: serde_json::Value =
        serde_json::from_str(row.detail_json.as_deref().unwrap()).unwrap();
    assert_eq!(detail["error_kind"], "invalid_request");
    let note = newest_ui_note(&store, &session).unwrap();
    assert!(note.starts_with("⚠ Turn failed:"), "{note}");
}

#[tokio::test]
async fn a_cancelled_turn_is_closed_as_interrupted_by_the_drop_guard() {
    let (mut session, store) = fixture(vec![Step::Hang], None);
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        session.run_turn("never answered"),
    )
    .await;
    assert!(cancelled.is_err(), "the turn was cut off");
    let rows = turns(&store, &session);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].end_reason.as_deref(), Some("interrupted"));
    assert!(rows[0].ended_at.is_some());
    assert_eq!(
        rows[0].last_role_shape.as_deref(),
        Some("unanswered_prompt")
    );
}

#[tokio::test]
async fn a_steer_joins_the_turn_instead_of_opening_one() {
    let (mut session, store) = fixture(
        vec![
            Step::Reply("", vec![list_dir()]),
            Step::Reply("done", vec![]),
        ],
        Some(0),
    );
    session.run_turn("tidy").await.unwrap();
    assert_eq!(turns(&store, &session).len(), 1);
}

#[test]
fn stop_notes_name_the_reason_and_the_way_out() {
    let note = crate::stop_note("stalled");
    assert!(note.starts_with("⚠ Turn stopped (no progress): "), "{note}");
    assert!(note.contains("continue"));
    assert!(crate::stop_note("some_new_reason").contains("some new reason"));
}

#[tokio::test]
async fn a_stalled_ok_end_gets_a_stop_note() {
    let (mut session, store) = fixture(vec![Step::Reply("done", vec![])], None);
    session.run_turn("go").await.unwrap();
    let id = store.open_turn(&session.id, 99).unwrap();
    session
        .turn_row
        .store(id, std::sync::atomic::Ordering::SeqCst);
    session.finish_turn_row(&Ok(LoopOutcome::stalled(String::new())));
    let last = store.last_closed_turn(&session.id).unwrap().unwrap();
    assert_eq!(last.end_reason.as_deref(), Some("stalled"));
    assert!(newest_ui_note(&store, &session)
        .unwrap()
        .starts_with("⚠ Turn stopped (no progress)"));
}

#[test]
fn only_the_recoverable_ends_offer_continue() {
    for reason in [
        "stalled",
        "tasks_unfinished",
        "provider_down",
        "provider_error",
        "interrupted",
        "daemon_restart_or_crash",
        "final_answer_mid_intent",
    ] {
        assert!(crate::is_resumable(reason), "{reason}");
    }
    for reason in [
        "final_answer",
        "budget_exhausted",
        "pinned_rejected",
        "review_blocked",
    ] {
        assert!(!crate::is_resumable(reason), "{reason}");
    }
    assert!(crate::end_fails_run("final_answer_mid_intent"));
    assert!(crate::end_fails_run("provider_down"));
    assert!(!crate::end_fails_run("final_answer"));
}

#[test]
fn every_end_reason_has_a_plain_label() {
    for reason in [
        "final_answer",
        "final_answer_mid_intent",
        "final_answer_open_tasks",
        "daemon_restart_or_crash",
        "daemon_shutdown",
        "interrupted",
        "abandoned",
        "unknown_finished",
    ] {
        let label = crate::end_label(reason);
        assert!(!label.contains('_'), "{reason} -> {label}");
    }
    assert_eq!(crate::end_label("final_answer"), "finished");
}

#[tokio::test]
async fn a_stop_note_reaches_the_presenter_once_and_an_error_end_adds_none() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = Arc::clone(&capture.events);
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(Script {
            steps: Mutex::new(vec![Step::Reply("done", vec![])]),
            steer: None,
            calls: Default::default(),
        }),
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
    session.run_turn("go").await.unwrap();
    let stopped = |events: &Arc<Mutex<Vec<PresenterEvent>>>| {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::TurnStopped { .. }))
            .count()
    };
    assert_eq!(stopped(&events), 0, "a final answer says nothing");
    let open = |session: &mut Session| {
        let id = store.open_turn(&session.id, 99).unwrap();
        session
            .turn_row
            .store(id, std::sync::atomic::Ordering::SeqCst);
    };
    open(&mut session);
    session.finish_turn_row(&Ok(LoopOutcome::stalled(String::new())));
    assert_eq!(stopped(&events), 1);
    let text = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match e {
            PresenterEvent::TurnStopped { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap();
    assert!(text.starts_with("Turn stopped (no progress): "), "{text}");
    open(&mut session);
    session.finish_turn_row(&Ok(LoopOutcome::stalled(String::new())));
    assert_eq!(stopped(&events), 1, "the note is already the newest row");
    open(&mut session);
    session.finish_turn_row(&Err(CoreError::Internal("boom".into())));
    assert_eq!(stopped(&events), 1, "an error end has its own Error line");
}
