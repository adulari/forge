use std::sync::Arc;

use forge_config::{Config, OneOrMany, PriceOverride};
use forge_core::Session;
use forge_mesh::HeuristicRouter;
use forge_provider::{EventSink, ModelResponse, Provider, ProviderError, ToolSpec};
use forge_store::Store;
use forge_tui::HeadlessPresenter;
use forge_types::{Message, PermissionMode, Role, ToolCall, Usage};

struct WorkspaceOpsProvider {
    marker: String,
}

#[async_trait::async_trait]
impl Provider for WorkspaceOpsProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let usage = Usage::default();
        if messages.iter().any(|message| message.role == Role::Tool) {
            return Ok(ModelResponse {
                content: "done".into(),
                tool_calls: vec![],
                usage,
                quotas: vec![],
            });
        }
        Ok(ModelResponse {
            content: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: "write".into(),
                    name: "write_file".into(),
                    args: serde_json::json!({
                        "path": "marker.txt",
                        "content": self.marker,
                    }),
                },
                ToolCall {
                    id: "shell".into(),
                    name: "shell".into(),
                    args: serde_json::json!({
                        "command": format!("printf {} > shell-marker.txt", self.marker),
                    }),
                },
            ],
            usage,
            quotas: vec![],
        })
    }
}

fn config() -> Config {
    let mut config = Config {
        permission_mode: PermissionMode::AcceptEdits,
        ..Config::default()
    };
    config
        .mesh
        .models
        .insert("standard".into(), OneOrMany::One("mock".into()));
    config.mesh.pricing.insert(
        "mock".into(),
        PriceOverride {
            input_per_1k: 0.0,
            output_per_1k: 0.0,
        },
    );
    config
}

#[tokio::test]
async fn concurrent_provider_driven_sessions_are_workspace_isolated() {
    let base =
        std::env::temp_dir().join(format!("forge-session-isolation-{}", forge_types::new_id()));
    let first = base.join("first");
    let second = base.join("second");
    let sentinel = base.join("sentinel-daemon-cwd");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::create_dir_all(&sentinel).unwrap();
    std::fs::write(sentinel.join("sentinel.txt"), "untouched").unwrap();

    let first_config = config();
    let second_config = config();
    let first_session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(WorkspaceOpsProvider {
            marker: "first".into(),
        }),
        Arc::new(HeuristicRouter::new(first_config.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(&first),
        Box::new(HeadlessPresenter::new(false)),
        first_config,
        first.to_str().unwrap(),
    )
    .unwrap();
    let second_session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(WorkspaceOpsProvider {
            marker: "second".into(),
        }),
        Arc::new(HeuristicRouter::new(second_config.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(&second),
        Box::new(HeadlessPresenter::new(false)),
        second_config,
        second.to_str().unwrap(),
    )
    .unwrap();

    let (first_result, second_result) = tokio::join!(
        async move {
            let mut session = first_session;
            session.run_turn("write first").await
        },
        async move {
            let mut session = second_session;
            session.run_turn("write second").await
        },
    );
    first_result.unwrap();
    second_result.unwrap();

    for (root, marker) in [(&first, "first"), (&second, "second")] {
        assert_eq!(
            std::fs::read_to_string(root.join("marker.txt")).unwrap(),
            marker
        );
        assert_eq!(
            std::fs::read_to_string(root.join("shell-marker.txt")).unwrap(),
            marker
        );
    }
    assert_eq!(
        std::fs::read_to_string(sentinel.join("sentinel.txt")).unwrap(),
        "untouched"
    );
    assert!(!sentinel.join("marker.txt").exists());
    assert!(!sentinel.join("shell-marker.txt").exists());
    let _ = std::fs::remove_dir_all(base);
}
