//! `spawn_agents` with an `output_schema`, through a real Session.

use super::*;

/// A child that submits the result its task asks for: "good" is valid, anything else is not.
struct SubmittingProvider;

#[async_trait::async_trait]
impl forge_provider::Provider for SubmittingProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let good = messages
            .iter()
            .any(|m| m.role == Role::User && m.content.contains("good"));
        let args = if good {
            serde_json::json!({"line": 3})
        } else {
            serde_json::json!({"line": "three"})
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: String::new(),
            tool_calls: vec![forge_types::ToolCall {
                id: format!("s{}", messages.len()),
                name: "submit_result".into(),
                args,
            }],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn session(dir: &std::path::Path, flag: bool) -> Session {
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.harness.subagent_output_schema = flag;
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(SubmittingProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::<CapturePresenter>::default(),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

fn spawn(args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: "call-spawn".into(),
        name: "spawn_agents".into(),
        args,
    }
}

const SCHEMA: &str =
    r#"{"type":"object","properties":{"line":{"type":"integer"}},"required":["line"]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_schema_batch_returns_json_and_one_invalid_child_makes_the_call_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path(), true);
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "go", None)
        .unwrap();
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let out = session
        .invoke_tool(
            &msg_id,
            &spawn(serde_json::json!({
                "output_schema": schema,
                "agents": [{"task": "good one"}, {"task": "bad one"}]
            })),
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(v["agents"][0]["status"], "valid", "{out}");
    assert_eq!(v["agents"][0]["data"]["line"], 3);
    assert_eq!(v["agents"][1]["status"], "invalid", "{out}");
    assert!(v["agents"][1]["errors"][0]
        .as_str()
        .unwrap()
        .contains("$.line"));
    let rows = session.store.tool_call_rows(&msg_id).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.status == "error" && r.tool_name == "spawn_agents"),
        "{rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_schema_is_refused_before_any_child_session_exists() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path(), true);
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "go", None)
        .unwrap();
    let out = session
        .invoke_tool(
            &msg_id,
            &spawn(serde_json::json!({
                "agents": [{"task": "good", "output_schema": {"type": "array"}}]
            })),
        )
        .await
        .unwrap();
    assert!(out.starts_with("error: output_schema for agent 1"), "{out}");
    assert!(session
        .store
        .child_sessions(session.id())
        .unwrap()
        .is_empty());
}
