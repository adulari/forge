//! Stalled tasks (`task_staleness.rs`) through real turns: a task nobody moves is escalated to the
//! model once, then — if it still does not move — the USER is asked what to do with it. The harness
//! never removes a task on its own.

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

/// `answer` is what the (attended) presenter returns when the harness asks the user about a stalled
/// task. `""` there means the presenter picks its first option — "Keep working on it".
fn talking_session(answer: &str) -> (tempfile::TempDir, Session, Arc<Mutex<Vec<PresenterEvent>>>) {
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
        ask_answer: answer.to_string(),
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
async fn a_task_nobody_moves_is_escalated_then_the_user_is_asked_never_auto_removed() {
    // The user is attended and answers "Keep working on it", so the task stays on the list.
    let (_dir, mut session, _events) = talking_session("Keep working on it");
    session.run_turn("start").await.unwrap();
    assert_eq!(session.tasks().len(), 1, "the model opened one task");

    // Turns 2 and 3 leave it exactly as it was: still under the escalation threshold.
    session.run_turn("carry on").await.unwrap();
    session.run_turn("carry on").await.unwrap();
    assert!(task_notes(&session).is_empty(), "not stale yet");

    // Third unchanged turn: name it and demand a decision from the model, but leave it on the list.
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

    // Two more unchanged turns: the harness asks the USER (not the model), who says keep it. The
    // task is STILL on the list — nothing is auto-removed.
    session.run_turn("carry on").await.unwrap();
    session.run_turn("carry on").await.unwrap();
    let notes = task_notes(&session);
    assert!(
        notes
            .iter()
            .any(|n| n.contains("asked the user") && n.contains("keep working")),
        "the user was asked and their keep decision recorded: {notes:?}"
    );
    assert_eq!(
        session.tasks().len(),
        1,
        "the task is kept — the harness never removes it on its own: {:?}",
        session.tasks()
    );
}

#[tokio::test]
async fn the_user_can_choose_to_remove_a_stalled_task() {
    // The one path that takes a task off the list is the user explicitly choosing to.
    let (_dir, mut session, events) = talking_session("Remove it");
    for _ in 0..6 {
        session.run_turn("carry on").await.unwrap();
    }
    assert!(
        session
            .tasks()
            .iter()
            .all(|t| t.title != "Strip live-reuse, keep perf wins"),
        "the user chose Remove, so it is gone: {:?}",
        session.tasks()
    );
    let told = task_notes(&session)
        .iter()
        .any(|n| n.contains("removed it from the list"));
    assert!(told, "the model is told the user removed it");
    // The removal is the user's decision, surfaced as a task update, not a silent harness edit.
    let updated = events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, PresenterEvent::Tasks(_)));
    assert!(updated, "the new list is emitted to the surface");
}

#[tokio::test]
async fn nobody_present_keeps_the_task_rather_than_removing_it() {
    // Unattended: there is nobody to answer, so the task stays exactly as it is — never deleted.
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
        attended: false,
        ..Default::default()
    };
    let mut session = Session::start(
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
    for _ in 0..6 {
        session.run_turn("carry on").await.unwrap();
    }
    assert_eq!(
        session.tasks().len(),
        1,
        "with nobody to answer, the task is kept, not removed: {:?}",
        session.tasks()
    );
    assert!(
        task_notes(&session)
            .iter()
            .any(|n| n.contains("stays on the list")),
        "the model is told it stays open: {:?}",
        task_notes(&session)
    );
}

#[tokio::test]
async fn a_task_the_model_keeps_working_on_is_left_alone() {
    let (_dir, mut session, _events) = talking_session("");
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
