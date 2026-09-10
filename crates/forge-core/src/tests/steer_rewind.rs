//! Mid-turn steering (`steer.rs`) and the state `rewind_to` has to re-derive.

use super::*;

/// Call 0: one cheap tool call, and — while "the model is working" — the user queues a prompt.
/// Call 1: a final answer. The queued prompt must reach the model between the two.
struct ToolThenFinal {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    inbox: crate::steer::SteerInbox,
    steer_on_call: usize,
    tool_on_first: bool,
}

#[async_trait::async_trait]
impl Provider for ToolThenFinal {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == self.steer_on_call {
            self.inbox.push("also rename the helper");
        }
        let tool_calls = if n == 0 && self.tool_on_first {
            vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "list_dir".into(),
                args: serde_json::json!({"path": "."}),
            }]
        } else {
            Vec::new()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            content: if n == 0 && self.tool_on_first {
                String::new()
            } else {
                format!("answer after call {n}")
            },
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

type SteerFixture = (
    Session,
    Arc<Mutex<Vec<PresenterEvent>>>,
    Arc<Store>,
    Arc<std::sync::atomic::AtomicUsize>,
);

fn steer_session(tool_on_first: bool, steer_on_call: usize) -> SteerFixture {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let inbox = crate::steer::SteerInbox::default();
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(ToolThenFinal {
            calls: Arc::clone(&calls),
            inbox: inbox.clone(),
            steer_on_call,
            tool_on_first,
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    // The provider's inbox IS the session's inbox (what a surface holds via `steer_handle`).
    session.steer = inbox;
    (session, events, store, calls)
}

fn steered(events: &Mutex<Vec<PresenterEvent>>) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Steered(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_prompt_queued_during_a_tool_step_joins_the_turn_after_that_step() {
    let (mut session, events, store, calls) = steer_session(true, 0);
    session.run_turn("tidy the module").await.unwrap();
    assert_eq!(steered(&events), vec!["also rename the helper".to_string()]);
    let roles: Vec<(Role, String)> = session
        .transcript
        .iter()
        .map(|m| (m.role, m.content.chars().take(24).collect()))
        .collect();
    let tool_idx = roles
        .iter()
        .position(|(r, _)| *r == Role::Tool)
        .expect("the tool result is in the transcript");
    let steer_idx = roles
        .iter()
        .position(|(r, c)| *r == Role::User && c.starts_with("also rename"))
        .expect("the queued prompt became a user message");
    assert!(
        steer_idx > tool_idx,
        "queued prompt lands after the tool result: {roles:?}"
    );
    assert!(
        roles[steer_idx + 1..]
            .iter()
            .any(|(r, _)| *r == Role::Assistant),
        "the model answered AFTER seeing the queued prompt: {roles:?}"
    );
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    let persisted = store.load_messages(&session.id).unwrap();
    assert!(
        persisted
            .iter()
            .any(|m| m.role == Role::User && m.content == "also rename the helper"),
        "the steer is persisted like any user message"
    );
}

#[tokio::test]
async fn a_prompt_queued_during_the_final_answer_continues_the_same_turn() {
    // No tool call at all: the only boundary is where the turn would have ended.
    let (mut session, events, _store, calls) = steer_session(false, 0);
    session.run_turn("summarize").await.unwrap();
    let dbg: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .map(|e| format!("{e:?}").chars().take(90).collect())
        .collect();
    let roles: Vec<(Role, String)> = session
        .transcript
        .iter()
        .map(|m| (m.role, m.content.chars().take(30).collect()))
        .collect();
    assert_eq!(
        steered(&events),
        vec!["also rename the helper".to_string()],
        "calls={} roles={roles:?} events={dbg:#?}",
        calls.load(std::sync::atomic::Ordering::SeqCst)
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "one continuation call for the queued prompt, then done"
    );
    let last = session.transcript.last().expect("transcript not empty");
    assert_eq!(last.role, Role::Assistant);
    assert_eq!(last.content, "answer after call 1");
}

#[tokio::test]
async fn a_leftover_from_a_previous_turn_is_not_replayed_into_the_next() {
    let (mut session, events, _store, _calls) = steer_session(false, 99);
    // Simulate a prompt that was queued after the turn's last boundary: still in the inbox when
    // the next turn starts. The surface will submit it as its own turn; the inbox must not.
    session.steer.push("stale leftover");
    session.run_turn("first").await.unwrap();
    assert!(
        steered(&events).is_empty(),
        "nothing injected: {:?}",
        steered(&events)
    );
    assert!(
        !session
            .transcript
            .iter()
            .any(|m| m.content == "stale leftover"),
        "the leftover was cleared at turn start"
    );
}

#[tokio::test]
async fn rewind_rederives_turn_seq_epoch_and_injection_latches() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = fresh_session(Arc::clone(&store), Config::default());
    session.run_turn("first prompt").await.unwrap();
    let first_turn_seq = session.current_turn_seq;
    session.run_turn("second prompt").await.unwrap();
    let second_turn_seq = session.current_turn_seq;
    assert!(second_turn_seq > first_turn_seq);
    let epoch_before = session.checkpoint_context().epoch;

    // State that only makes sense for the turns about to be removed. The guidance message is
    // persisted like a real injection so the seq ↔ transcript-index invariant holds.
    session.pending_hints.push("stale hint".into());
    session.steer.push("stale steer");
    let gseq = session.next_seq();
    store
        .add_message(
            &session.id,
            gseq,
            Role::System,
            workflow::WHITEHOT_GUIDANCE,
            None,
        )
        .unwrap();
    session
        .transcript
        .push(Message::system(workflow::WHITEHOT_GUIDANCE));
    session.whitehot_guidance_injected = true;

    let outcome = session.rewind_to(second_turn_seq).unwrap();
    assert_eq!(outcome.rewound_prompt.as_deref(), Some("second prompt"));
    assert_eq!(
        session.current_turn_seq, first_turn_seq,
        "the newest surviving prompt is the current turn again"
    );
    assert_eq!(
        session.checkpoint_context().epoch,
        epoch_before + 1,
        "a rewind is a new history epoch (the bridge must not resume its old session)"
    );
    assert!(session.pending_hints.is_empty());
    assert!(session.steer.is_empty());
    assert!(
        !session.whitehot_guidance_injected,
        "the guidance message was removed, so the latch re-arms"
    );
    assert!(
        session.agents_md_fingerprint.is_none(),
        "the AGENTS.md refresh looks again on the next turn"
    );
    // Chained undo now lands on the first turn, not on a stale seq.
    assert!(session.undo().unwrap().is_some());
    assert!(store.load_messages(&session.id).unwrap().is_empty());
}

#[tokio::test]
async fn rewind_into_compacted_history_reactivates_it_first() {
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
    // Fold seq 0-5 into a summary; seq 6-9 stay verbatim.
    store.compact_session_store(&sid, "summary", 4).unwrap();
    let mut session = Session::resume(
        Arc::clone(&store),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(Config::default())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(HeadlessPresenter::new(false)),
        Config::default(),
        &sid,
    )
    .unwrap();
    assert!(session.was_compacted());
    assert_eq!(session.transcript.len(), 5, "summary + 4 survivors");

    // Rewind to the prompt at seq 2 — inside the folded-away part.
    let outcome = session.rewind_to(2).unwrap();
    assert_eq!(outcome.rewound_prompt.as_deref(), Some("msg 2"));
    assert!(!session.was_compacted(), "the summary is gone");
    let active = store.load_messages(&sid).unwrap();
    assert_eq!(
        active
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["msg 0", "msg 1"],
        "seq 0-1 are live again, seq 2+ are rewound away"
    );
    assert_eq!(
        session
            .transcript
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["msg 0", "msg 1"]
    );
    assert_eq!(session.seq, 2);
}

#[test]
fn inject_steers_persists_and_echoes_each_prompt() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(Config::default())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        Config::default(),
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    assert!(!session.inject_steers(), "empty inbox injects nothing");
    let handle = session.steer_handle();
    handle.push("one");
    handle.push("two");
    assert!(session.inject_steers());
    assert_eq!(steered(&events), vec!["one".to_string(), "two".to_string()]);
    let persisted = store.load_messages(&session.id).unwrap();
    assert_eq!(persisted.len(), 2);
    assert!(persisted.iter().all(|m| m.role == Role::User));
    assert!(session.steer.is_empty());
}
