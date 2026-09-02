//! A bridged turn that answers with prose while tracked tasks stay open.
//!
//! Observed 2026-09-02 in a headless `forge run --mode bypass --resume …` routed to
//! claude-cli::opus: the model replied with a self-review essay, called no tool, changed no file —
//! and Forge printed "Send `continue` to resume" to a session nobody was attached to, then exited
//! 0 with the outcome recorded as a completed turn.

use super::*;

/// Registers two tasks in_progress on call 0, then answers with prose forever: no tool call, no
/// task closed — the exact stall shape of the incident.
struct ProseOnlyBridge {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for ProseOnlyBridge {
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
                id: forge_types::new_id(),
                name: "update_tasks".into(),
                args: serde_json::json!({"tasks": [
                    {"title": "rework the failover chain", "status": "in_progress"},
                    {"title": "add a regression test", "status": "pending"}
                ]}),
            }]
        } else {
            Vec::new()
        };
        Ok(forge_provider::ModelResponse {
            content: "Claim: the earlier change is correct. Verification summary: reviewed."
                .to_string(),
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn bridge_session(attended: bool) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>, Arc<Store>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter {
        attended,
        ..Default::default()
    };
    let events = capture.events.clone();
    let session = Session::start(
        Arc::clone(&store),
        Arc::new(ProseOnlyBridge {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        Config::default(),
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    (session, events, store)
}

fn warnings(events: &Mutex<Vec<PresenterEvent>>) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(w) => Some(w.clone()),
            _ => None,
        })
        .collect()
}

fn errors(events: &Mutex<Vec<PresenterEvent>>) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Error(e) => Some(e.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_prose_only_bridge_stall_gets_exactly_one_stronger_nudge_naming_the_task() {
    let (mut session, events, store) = bridge_session(false);

    session.run_turn("rework it").await.unwrap();

    let escalations = warnings(&events)
        .iter()
        .filter(|w| w.contains("without calling any tool"))
        .count();
    assert_eq!(
        escalations, 1,
        "the stronger nudge must be sent exactly once, not on every stall"
    );

    let nudges: Vec<String> = store
        .load_messages(&session.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.content)
        .filter(|c| c.contains("prose only"))
        .collect();
    assert_eq!(nudges.len(), 1, "exactly one escalation reached the model");
    assert!(
        nudges[0].contains("rework the failover chain"),
        "the nudge names the first unfinished task verbatim: {}",
        nudges[0]
    );
    assert!(
        nudges[0].contains("Do NOT restate"),
        "the nudge forbids another restatement: {}",
        nudges[0]
    );
}

#[tokio::test]
async fn an_unattended_stall_fails_the_turn_with_an_error_naming_the_open_work() {
    let (mut session, events, _store) = bridge_session(false);

    let outcome = session.run_turn("rework it").await.unwrap();

    assert_eq!(
        outcome.stop_reason,
        StopReason::TasksUnfinished,
        "an unfinished plan must never be recorded as a completed turn"
    );
    assert_eq!(StopReason::TasksUnfinished.outcome(), "failed");
    let errors = errors(&events);
    let error = errors
        .iter()
        .find(|e| e.starts_with("ERROR: unattended turn ended with"))
        .unwrap_or_else(|| panic!("expected an unattended ERROR line; got {errors:?}"));
    assert!(error.contains("rework the failover chain"), "{error}");
    assert!(error.contains("add a regression test"), "{error}");
    assert!(
        outcome.text.starts_with("ERROR:"),
        "the failure text is what a headless caller exits on: {}",
        outcome.text
    );
    assert!(
        !warnings(&events)
            .iter()
            .any(|w| w.contains("Send `continue`")),
        "nobody is attached to type `continue`"
    );
}

#[tokio::test]
async fn an_attended_stall_still_pauses_for_the_user() {
    let (mut session, events, _store) = bridge_session(true);

    let outcome = session.run_turn("rework it").await.unwrap();

    assert!(
        warnings(&events)
            .iter()
            .any(|w| w.contains("still unfinished") && w.contains("Send `continue`")),
        "an attended session keeps the resume prompt: {:?}",
        warnings(&events)
    );
    assert!(
        errors(&events).is_empty(),
        "an attended pause is not a hard failure: {:?}",
        errors(&events)
    );
    assert_eq!(
        outcome.stop_reason,
        StopReason::TasksUnfinished,
        "the store still records the turn as incomplete"
    );
}
