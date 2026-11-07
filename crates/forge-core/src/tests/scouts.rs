//! The `scout` tool, through a real Session: only the brief returns, the reading stays in the
//! child session, and the scrollback gets the summary line.

use super::*;

/// A scout child that lists the directory twice, then answers with a cited brief.
struct ScoutProvider;

#[async_trait::async_trait]
impl forge_provider::Provider for ScoutProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let reads = messages.iter().filter(|m| m.role == Role::Tool).count();
        let (content, tool_calls) = if reads < 2 {
            (
                String::new(),
                vec![forge_types::ToolCall {
                    id: format!("r{reads}"),
                    name: "list_dir".into(),
                    args: serde_json::json!({"path": "."}),
                }],
            )
        } else {
            (
                "Retry lives in net.rs.\n- backoff: net.rs:10-20".to_string(),
                Vec::new(),
            )
        };
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

fn session(
    dir: &std::path::Path,
    mode: forge_config::ScoutsMode,
) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.harness.scouts = mode;
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(ScoutProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(capture),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    (session, events)
}

fn scout(question: &str) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: "call-scout".into(),
        name: "scout".into(),
        args: serde_json::json!({ "question": question }),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scout_returns_its_brief_and_keeps_the_reading_in_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, events) = session(dir.path(), forge_config::ScoutsMode::Tool);
    assert!(session.tool_specs().iter().any(|s| s.name == "scout"));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "go", None)
        .unwrap();
    let out = session
        .invoke_tool(
            &msg_id,
            &session.resolve_model_call(&scout("where is retry?")),
        )
        .await
        .unwrap();
    assert!(out.contains("net.rs:10-20"), "{out}");
    assert!(!out.contains("list_dir"), "{out}");
    let children = session.store.child_sessions(session.id()).unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(
        session.store.session_tool_call_count(&children[0]).unwrap(),
        2
    );
    assert_eq!(session.scouts_this_turn(), Some(1));
    let events = events.lock().unwrap();
    let line = events.iter().find_map(|e| match e {
        PresenterEvent::ToolResult { name, summary, .. } if name == "scout" => {
            Some(summary.clone())
        }
        _ => None,
    });
    let line = line.expect("a scout summary line");
    assert!(line.starts_with("scout: 2 reads -> "), "{line}");
    assert!(line.contains("saved ~"), "{line}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn off_does_not_advertise_scout() {
    let dir = tempfile::tempdir().unwrap();
    let (session, _events) = session(dir.path(), forge_config::ScoutsMode::Off);
    assert!(!session.tool_specs().iter().any(|s| s.name == "scout"));
}
