//! A detached `spawn_agents` call returns at once, but its children must still reach the agents
//! panel: Start at admission, then their outcome after the turn has moved on, through the
//! presenter's background sink — and what the panel shows must agree with `list_subagents`.

use super::*;

/// A presenter whose background sink feeds the same event log, like the TUI's channel presenter.
#[derive(Clone, Default)]
struct SinkPresenter {
    events: Arc<Mutex<Vec<PresenterEvent>>>,
}

impl Presenter for SinkPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn recap_sink(&self) -> Option<Box<dyn Presenter>> {
        Some(Box::new(self.clone()))
    }
    fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> forge_types::ConfirmOutcome {
        forge_types::ConfirmOutcome::Deny
    }
    fn ask(&mut self, _q: &str, _options: &[forge_types::QChoice], _allow_other: bool) -> String {
        String::new()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
}

/// A child model that never answers, so the child stays running until cancelled.
struct SilentProvider;

#[async_trait::async_trait]
impl forge_provider::Provider for SilentProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        std::future::pending().await
    }
}

fn session_with(
    dir: &std::path::Path,
    provider: Arc<dyn forge_provider::Provider>,
) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let presenter = SinkPresenter::default();
    let events = Arc::clone(&presenter.events);
    let config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(presenter),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    (session, events)
}

fn tool_call(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: format!("call-{name}"),
        name: name.into(),
        args,
    }
}

/// Fold every event so far into a TUI `App` — the state the TUI rail/band and the remote
/// snapshot (desktop + mobile Agents tab) are both built from.
fn panel(events: &Arc<Mutex<Vec<PresenterEvent>>>) -> forge_tui::App {
    let mut app = forge_tui::App::default();
    for event in events.lock().unwrap().iter().cloned() {
        app.apply(event);
    }
    app
}

async fn wait_for(
    events: &Arc<Mutex<Vec<PresenterEvent>>>,
    done: impl Fn(&PresenterEvent) -> bool,
) {
    for _ in 0..500 {
        if events.lock().unwrap().iter().any(&done) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!(
        "timed out waiting for the event; saw {:?}",
        events.lock().unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_detached_agent_reaches_the_panel_and_settles_when_it_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, events) = session_with(dir.path(), Arc::new(MockProvider));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "plan it", None)
        .unwrap();

    let admitted = session
        .invoke_tool(
            &msg_id,
            &tool_call(
                "spawn_agents",
                serde_json::json!({
                    "agents": [{ "agent": "plan", "task": "draft the plan" }],
                    "detached": true
                }),
            ),
        )
        .await
        .unwrap();
    assert!(admitted.contains("admitted 1 detached agent"), "{admitted}");

    // Visible from admission on, before the child has finished.
    let started = panel(&events).remote_snapshot();
    assert_eq!(started.subagents.len(), 1, "{:?}", events.lock().unwrap());
    assert_eq!(started.subagents[0].agent, "plan");
    assert!(started.subagents[0].model.is_some());
    let child_id = started.subagents[0].id.clone();

    // The child finishes after the call returned and reports through the background sink.
    wait_for(&events, |e| {
        matches!(e, PresenterEvent::SubagentResult { .. })
    })
    .await;
    let settled = panel(&events).remote_snapshot();
    assert_eq!(settled.subagents.len(), 1);
    assert!(settled.subagents[0].done, "{:?}", settled.subagents[0]);
    assert_eq!(settled.subagents[0].id, child_id);

    // `list_subagents` reads the same store row the panel was settled from.
    let listed = session
        .invoke_tool(&msg_id, &tool_call("list_subagents", serde_json::json!({})))
        .await
        .unwrap();
    let status = if settled.subagents[0].ok {
        "[done]"
    } else {
        "[failed]"
    };
    assert!(listed.contains(status), "{listed}");
    assert!(listed.contains(&child_id[..8]), "{listed}");

    // Delivery at the next turn boundary repeats the outcome; the panel keeps one settled row.
    session.deliver_pending_detached_results().unwrap();
    let delivered = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PresenterEvent::SubagentResult { id, .. } if *id == child_id))
        .count();
    assert_eq!(delivered, 2, "the child's own Done plus delivery");
    let after = panel(&events).remote_snapshot();
    assert_eq!(after.subagents.len(), 1);
    assert!(after.subagents[0].done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_detached_agent_settles_as_failed_in_the_panel() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, events) = session_with(dir.path(), Arc::new(SilentProvider));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "explore", None)
        .unwrap();
    session
        .invoke_tool(
            &msg_id,
            &tool_call(
                "spawn_agents",
                serde_json::json!({
                    "agents": [{ "agent": "explore", "task": "map the crate" }],
                    "detached": true
                }),
            ),
        )
        .await
        .unwrap();
    let running = panel(&events).remote_snapshot();
    assert_eq!(running.subagents.len(), 1);
    assert!(!running.subagents[0].done);

    let cancelled = session
        .invoke_tool(
            &msg_id,
            &tool_call("cancel_subagent", serde_json::json!({ "agent": "explore" })),
        )
        .await
        .unwrap();
    assert!(cancelled.starts_with("cancelled 'explore'"), "{cancelled}");

    let snap = panel(&events).remote_snapshot();
    assert!(
        snap.subagents[0].done && !snap.subagents[0].ok,
        "{:?}",
        snap.subagents[0]
    );
    let listed = session
        .invoke_tool(&msg_id, &tool_call("list_subagents", serde_json::json!({})))
        .await
        .unwrap();
    assert!(listed.contains("[cancelled]"), "{listed}");
}

/// Admit `explore` and `plan` as detached children that never answer.
async fn two_silent_children(session: &mut Session, msg_id: &str) {
    let admitted = session
        .invoke_tool(
            msg_id,
            &tool_call(
                "spawn_agents",
                serde_json::json!({
                    "agents": [
                        { "agent": "explore", "task": "map the crate" },
                        { "agent": "plan", "task": "draft the plan" }
                    ],
                    "detached": true
                }),
            ),
        )
        .await
        .unwrap();
    assert!(admitted.contains("admitted 2 detached agent"), "{admitted}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_for_subagents_any_returns_on_the_first_finisher_and_names_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _events) = session_with(dir.path(), Arc::new(SilentProvider));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "fan out", None)
        .unwrap();
    two_silent_children(&mut session, &msg_id).await;

    session
        .invoke_tool(
            &msg_id,
            &tool_call("cancel_subagent", serde_json::json!({ "agent": "explore" })),
        )
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let waited = session
        .invoke_tool(
            &msg_id,
            &tool_call("wait_for_subagents", serde_json::json!({ "mode": "any" })),
        )
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "any must not wait for the silent child"
    );
    assert!(waited.contains("[explore ("), "{waited}");
    assert!(waited.contains("cancelled]"), "{waited}");
    assert!(waited.contains("Still running: plan ("), "{waited}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_for_subagents_all_times_out_and_says_who_is_still_running() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _events) = session_with(dir.path(), Arc::new(SilentProvider));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "fan out", None)
        .unwrap();
    two_silent_children(&mut session, &msg_id).await;

    let waited = session
        .invoke_tool(
            &msg_id,
            &tool_call(
                "wait_for_subagents",
                serde_json::json!({ "mode": "all", "timeout_secs": 1 }),
            ),
        )
        .await
        .unwrap();
    assert!(waited.contains("timed out after 1s"), "{waited}");
    assert!(
        waited.contains("explore (") && waited.contains("plan ("),
        "{waited}"
    );

    let missing = session
        .invoke_tool(
            &msg_id,
            &tool_call(
                "wait_for_subagents",
                serde_json::json!({ "agents": ["nobody"] }),
            ),
        )
        .await
        .unwrap();
    assert!(
        missing.contains("no detached agent matches 'nobody'"),
        "{missing}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steer_subagent_redirects_a_child_and_returns_its_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _events) = session_with(dir.path(), Arc::new(MockProvider));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "plan it", None)
        .unwrap();
    session
        .invoke_tool(
            &msg_id,
            &tool_call(
                "spawn_agents",
                serde_json::json!({
                    "agents": [{ "agent": "plan", "task": "draft the plan" }],
                    "detached": true
                }),
            ),
        )
        .await
        .unwrap();
    // Whether the child is still running or already done, steering ends in its answer.
    let steered = session
        .invoke_tool(
            &msg_id,
            &tool_call(
                "steer_subagent",
                serde_json::json!({ "agent": "plan", "message": "focus on the API only" }),
            ),
        )
        .await
        .unwrap();
    assert!(steered.starts_with("[plan ("), "{steered}");

    let bad = session
        .invoke_tool(
            &msg_id,
            &tool_call("steer_subagent", serde_json::json!({ "agent": "plan" })),
        )
        .await
        .unwrap();
    assert!(bad.contains("needs both `agent`"), "{bad}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delegation_off_withholds_spawn_agents_for_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _events) = session_with(dir.path(), Arc::new(MockProvider));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "go", None)
        .unwrap();
    session.set_delegation_allowed(false);
    let refused = session
        .invoke_tool(
            &msg_id,
            &tool_call(
                "spawn_agents",
                serde_json::json!({ "agents": [{ "task": "anything" }] }),
            ),
        )
        .await
        .unwrap();
    assert!(refused.contains("delegation is switched off"), "{refused}");
    session.set_delegation_allowed(true);
    assert!(session.delegation_allowed());
}
