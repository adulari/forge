//! Stalled tasks (`task_staleness.rs`) through real turns: a task nobody moves is escalated once,
//! then removed from the list so the completion gate stops re-driving the session over it.

use super::*;

/// Turn 1 opens one task and leaves it In progress; every turn after that only talks about it,
/// which is exactly the shape of the session that motivated the guard.
struct Talker {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for Talker {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            return Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                content: String::new(),
                tool_calls: vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "update_tasks".into(),
                    args: serde_json::json!({
                        "tasks": [{"title": "Strip live-reuse, keep perf wins",
                                   "status": "in_progress"}]
                    }),
                }],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            });
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            content: "Still working out what that task means.".into(),
            tool_calls: vec![],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn talking_session() -> (tempfile::TempDir, Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.git.commit_nudge = false;
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(Talker {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
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
    (dir, session, events)
}

fn task_notes(session: &Session) -> Vec<String> {
    session
        .transcript
        .iter()
        .filter(|m| m.role == Role::System && m.content.starts_with("[tasks]"))
        .map(|m| m.content.clone())
        .collect()
}

#[tokio::test]
async fn a_task_nobody_moves_is_escalated_once_and_then_dropped() {
    let (_dir, mut session, _events) = talking_session();
    session.run_turn("start").await.unwrap();
    assert_eq!(session.tasks().len(), 1, "the model opened one task");

    // Turns 2 and 3 leave it exactly as it was: still under the escalation threshold.
    session.run_turn("carry on").await.unwrap();
    session.run_turn("carry on").await.unwrap();
    assert!(task_notes(&session).is_empty(), "not stale yet");

    // Third unchanged turn: name it and demand a decision, but leave it on the list.
    session.run_turn("carry on").await.unwrap();
    let notes = task_notes(&session);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("Strip live-reuse, keep perf wins"),
        "{}",
        notes[0]
    );
    assert!(notes[0].contains("ask_user"), "{}", notes[0]);
    assert_eq!(session.tasks().len(), 1, "escalation does not remove it");

    // The model ignores the escalation too, so the harness takes the task off the list itself.
    session.run_turn("carry on").await.unwrap();
    session.run_turn("carry on").await.unwrap();
    let notes = task_notes(&session);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[1].contains("removed"), "{}", notes[1]);
    assert!(
        session.tasks().is_empty(),
        "the stalled task is gone, so the completion gate stops re-driving: {:?}",
        session.tasks()
    );
}

#[tokio::test]
async fn dropping_a_task_is_reported_to_the_user_not_just_the_model() {
    let (_dir, mut session, events) = talking_session();
    for _ in 0..6 {
        session.run_turn("carry on").await.unwrap();
    }
    let warned =
        events.lock().unwrap().iter().any(
            |e| matches!(e, PresenterEvent::Warning(w) if w.contains("dropped 1 stalled task")),
        );
    assert!(warned, "the user is told their task list was edited");
}

#[tokio::test]
async fn a_task_the_model_keeps_working_on_is_left_alone() {
    let (_dir, mut session, _events) = talking_session();
    session.run_turn("start").await.unwrap();
    for status in [
        forge_types::TodoStatus::Pending,
        forge_types::TodoStatus::InProgress,
        forge_types::TodoStatus::Pending,
        forge_types::TodoStatus::InProgress,
        forge_types::TodoStatus::Pending,
    ] {
        // Status moves every turn, which is what "being worked on" looks like to the tracker.
        session.tasks[0].status = status;
        session.run_turn("carry on").await.unwrap();
    }
    assert!(task_notes(&session).is_empty());
    assert_eq!(session.tasks().len(), 1);
}
