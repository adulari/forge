//! `!command` from the composer: runs through the shell tool without a model call, keeps the
//! safety floor, draws a shell card and joins the conversation as a user message.

use super::*;

struct Counting {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    last_user: Arc<Mutex<String>>,
}

#[async_trait::async_trait]
impl Provider for Counting {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(m) = messages.iter().rev().find(|m| m.role == Role::User) {
            *self.last_user.lock().unwrap() = m.content.clone();
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "the command printed a greeting".into(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

struct Harness {
    session: Session,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    last_user: Arc<Mutex<String>>,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
    _ws: tempfile::TempDir,
}

fn harness(mode: PermissionMode) -> Harness {
    let ws = tempfile::tempdir().unwrap();
    let mut config = Config {
        permission_mode: mode,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let last_user = Arc::new(Mutex::new(String::new()));
    let presenter = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = Arc::clone(&presenter.events);
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(Counting {
            calls: Arc::clone(&calls),
            last_user: Arc::clone(&last_user),
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(ws.path()),
        Box::new(presenter),
        config,
        ws.path().to_str().unwrap(),
    )
    .unwrap();
    Harness {
        session,
        calls,
        last_user,
        events,
        _ws: ws,
    }
}

fn shell_cards(events: &Arc<Mutex<Vec<PresenterEvent>>>) -> Vec<(bool, String)> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::ToolResult {
                name, ok, summary, ..
            } if name == "shell" => Some((*ok, summary.clone())),
            _ => None,
        })
        .collect()
}

#[cfg(unix)]
#[tokio::test]
async fn a_bang_command_runs_in_the_workspace_without_asking_or_calling_the_model() {
    // Ask mode: a model's shell call would prompt; the user's own command does not.
    let mut h = harness(PermissionMode::Default);
    std::fs::write(h._ws.path().join("marker.txt"), "x").unwrap();
    let run = h
        .session
        .run_user_shell("echo hello-from-bang && ls")
        .await
        .unwrap();
    assert!(run.ok && !run.refused, "{run:?}");
    assert!(run.output.contains("hello-from-bang"), "{}", run.output);
    assert!(
        run.output.contains("marker.txt"),
        "ran in the workspace: {}",
        run.output
    );
    assert_eq!(h.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    let starts = h
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PresenterEvent::ToolStart { name, args } if name == "shell" && args.contains("hello-from-bang")))
        .count();
    assert_eq!(starts, 1);
    assert_eq!(shell_cards(&h.events).len(), 1);
    assert!(shell_cards(&h.events)[0].0);
}

#[cfg(unix)]
#[tokio::test]
async fn a_failing_command_draws_a_failed_card() {
    let mut h = harness(PermissionMode::AcceptEdits);
    let run = h
        .session
        .run_user_shell("echo oops >&2; exit 3")
        .await
        .unwrap();
    assert!(!run.ok && !run.refused, "{run:?}");
    assert!(run.output.contains("oops"), "{}", run.output);
    assert!(!shell_cards(&h.events)[0].0);
}

#[tokio::test]
async fn the_safety_floor_still_refuses_privilege_escalation_and_catastrophic_deletes() {
    let mut h = harness(PermissionMode::Bypass);
    let sudo = h.session.run_user_shell("sudo ls /root").await.unwrap();
    assert!(sudo.refused && !sudo.ok);
    assert!(
        sudo.output.starts_with("Forge never runs `sudo`"),
        "{}",
        sudo.output
    );
    assert!(sudo.output.contains("sudo ls /root"), "{}", sudo.output);
    let wipe = h.session.run_user_shell("rm -rf ~").await.unwrap();
    assert!(wipe.refused, "{wipe:?}");
    assert!(wipe.output.contains("safety floor"), "{}", wipe.output);
    let cards = shell_cards(&h.events);
    assert_eq!(
        cards[0],
        (false, "needs root — run it in your own terminal".into())
    );
    assert_eq!(cards[1], (false, "refused by the safety floor".into()));
    assert_eq!(h.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn recording_adds_the_run_to_context_and_the_next_turn_sees_it() {
    let mut h = harness(PermissionMode::AcceptEdits);
    let run = h
        .session
        .run_user_shell("echo context-probe")
        .await
        .unwrap();
    h.session.record_user_shell(&run).unwrap();
    let last = h.session.transcript.last().unwrap();
    assert_eq!(last.role, Role::User);
    assert!(last
        .content
        .contains("<bash-input>echo context-probe</bash-input>"));
    assert!(
        last.content.contains("context-probe\n</bash-stdout>"),
        "{}",
        last.content
    );
    assert_eq!(h.calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    // Responding is an ordinary turn whose prompt is the same message.
    let again = h
        .session
        .run_user_shell("echo respond-probe")
        .await
        .unwrap();
    h.session.run_turn(&again.context_message()).await.unwrap();
    assert_eq!(h.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let seen = h.last_user.lock().unwrap().clone();
    assert!(
        seen.contains("<bash-input>echo respond-probe</bash-input>"),
        "{seen}"
    );
    assert!(seen.contains("ran this shell command themselves"), "{seen}");
}
