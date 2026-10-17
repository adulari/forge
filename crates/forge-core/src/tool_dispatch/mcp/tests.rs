use std::sync::{Arc, Mutex};

use forge_mesh::HeuristicRouter;
use forge_provider::{Provider, ToolSpec};
use forge_store::Store;
use forge_tools::ToolRegistry;
use forge_types::{ConfirmOutcome, Message, Role, SideEffect};

use crate::{Config, PermissionMode, Presenter, PresenterEvent, Session};

/// Records the tool name of every confirmation it is asked for, and allows each.
#[derive(Clone, Default)]
struct ConfirmLog(Arc<Mutex<Vec<String>>>);

impl Presenter for ConfirmLog {
    fn emit(&mut self, _event: PresenterEvent) {}
    fn confirm(&mut self, tool: &str, _side_effect: SideEffect) -> ConfirmOutcome {
        self.0.lock().unwrap().push(tool.to_string());
        ConfirmOutcome::Allow
    }
    fn ask(&mut self, _q: &str, _o: &[forge_types::QChoice], _other: bool) -> String {
        String::new()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
    fn is_attended(&self) -> bool {
        true
    }
}

struct CallsEcho;

#[async_trait::async_trait]
impl Provider for CallsEcho {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_provider::ModelResponse;
        let done = messages.iter().any(|m| m.role == Role::Tool);
        Ok(ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if done { "done".into() } else { String::new() },
            tool_calls: if done {
                Vec::new()
            } else {
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "mcp_call".into(),
                    args: serde_json::json!({ "name": "test__echo", "arguments": { "msg": "hi" } }),
                }]
            },
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn an_mcp_call_asks_once_for_the_inner_tool_not_twice() {
    let mcp_cfg = forge_config::McpConfig::default();
    let config = Config {
        permission_mode: PermissionMode::Default,
        mcp: mcp_cfg.clone(),
        ..Config::default()
    };
    let confirmations = ConfirmLog::default();
    let manager = Arc::new(forge_mcp::testsupport::manager_with_echo(&mcp_cfg).await);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(CallsEcho),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(workspace),
        Box::new(confirmations.clone()),
        config,
        workspace.to_str().unwrap(),
    )
    .unwrap();
    session.set_mcp(Some(manager));

    let _ = session.run_turn("call echo").await.unwrap();

    let asked = confirmations.0.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec!["test__echo".to_string()],
        "one prompt, for the real tool"
    );
}
