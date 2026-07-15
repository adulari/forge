use std::sync::Arc;

use forge_config::{Config, OneOrMany, PriceOverride};
use forge_core::{test_cwd_guard, Session};
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
async fn reset_resumed_rebinds_provider_tools_to_persisted_workspace() {
    let base =
        std::env::temp_dir().join(format!("forge-reset-workspace-{}", forge_types::new_id()));
    let a = base.join("a");
    let b = base.join("b");
    let sentinel = base.join("sentinel");
    for root in [&a, &b, &sentinel] {
        std::fs::create_dir_all(root).unwrap();
    }
    let _cwd_guard = test_cwd_guard(&sentinel);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let cfg = config();
    let persisted = Session::start(
        Arc::clone(&store),
        Arc::new(WorkspaceOpsProvider { marker: "b".into() }),
        Arc::new(HeuristicRouter::new(cfg.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(&b),
        Box::new(HeadlessPresenter::new(false)),
        cfg.clone(),
        b.to_str().unwrap(),
    )
    .unwrap();
    let persisted_id = persisted.id().to_string();
    drop(persisted);
    let mut live = Session::start(
        Arc::clone(&store),
        Arc::new(WorkspaceOpsProvider { marker: "b".into() }),
        Arc::new(HeuristicRouter::new(cfg.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(&a),
        Box::new(HeadlessPresenter::new(false)),
        cfg,
        a.to_str().unwrap(),
    )
    .unwrap();
    live.set_lattice(Some(Arc::new(forge_index::Lattice::new(
        Arc::clone(&store),
        &a,
    ))));
    live.reset_resumed(&persisted_id).unwrap();
    assert_eq!(
        live.lattice_root(),
        Some(b.canonicalize().unwrap().to_string_lossy().as_ref())
    );
    assert_eq!(live.workspace_root(), b.canonicalize().unwrap());
    assert_eq!(
        live.checkpoint_root().display().to_string(),
        b.canonicalize()
            .unwrap()
            .join(".forge/checkpoints")
            .display()
            .to_string()
    );
    live.run_turn("write b").await.unwrap();
    assert_eq!(std::fs::read_to_string(b.join("marker.txt")).unwrap(), "b");
    assert_eq!(
        std::fs::read_to_string(b.join("shell-marker.txt")).unwrap(),
        "b"
    );
    assert!(!a.join("marker.txt").exists());
    assert!(!sentinel.join("marker.txt").exists());
    drop(_cwd_guard);
    let _ = std::fs::remove_dir_all(base);
}

#[tokio::test]
async fn transition_reinstalls_lattice_watcher_for_new_workspace() {
    let base =
        std::env::temp_dir().join(format!("forge-transition-watch-{}", forge_types::new_id()));
    let a = base.join("a");
    let b = base.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("old.rs"), "pub fn a_before_transition() {}\n").unwrap();
    std::fs::write(b.join("new.rs"), "pub fn b_before_transition() {}\n").unwrap();

    let store = Arc::new(Store::open_in_memory().unwrap());
    let cfg = config();
    let mut live = Session::start(
        Arc::clone(&store),
        Arc::new(WorkspaceOpsProvider {
            marker: "watch".into(),
        }),
        Arc::new(HeuristicRouter::new(cfg.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(&a),
        Box::new(HeadlessPresenter::new(false)),
        cfg,
        a.to_str().unwrap(),
    )
    .unwrap();
    let lattice = Arc::new(forge_index::Lattice::new(Arc::clone(&store), &a));
    lattice.update().unwrap();
    live.set_lattice(Some(lattice));
    live.install_lattice_watcher();

    live.reset_fresh(b.to_str().unwrap()).unwrap();
    assert_eq!(
        live.lattice_root(),
        Some(b.canonicalize().unwrap().to_string_lossy().as_ref())
    );

    // Let the detached B watcher finish registration, then prove an external B edit reaches the
    // new index while a later A edit does not reach it.
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::fs::write(b.join("new.rs"), "pub fn b_after_transition() {}\n").unwrap();
    std::fs::write(a.join("old.rs"), "pub fn a_after_transition() {}\n").unwrap();

    let mut b_reindexed = false;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if live
            .lattice_view("b_after_transition")
            .unwrap()
            .is_some_and(|view| view.query == "b_after_transition")
        {
            b_reindexed = true;
            break;
        }
    }
    assert!(
        b_reindexed,
        "B watcher did not reindex the post-transition edit"
    );
    assert!(
        live.lattice_view("a_after_transition")
            .unwrap()
            .unwrap()
            .roots
            .is_empty(),
        "A is still watched after the transition"
    );

    drop(live);
    let _ = std::fs::remove_dir_all(base);
}

#[tokio::test]
async fn reset_fresh_rebinds_provider_tools_to_new_workspace() {
    let base = std::env::temp_dir().join(format!("forge-reset-fresh-{}", forge_types::new_id()));
    let a = base.join("a");
    let b = base.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("AGENTS.md"), "A guidance").unwrap();
    std::fs::write(b.join("AGENTS.md"), "B guidance").unwrap();
    let cfg = config();
    let mut tools = forge_tools::ToolRegistry::with_core_tools_in(&a);
    tools.register(Box::new(forge_tools::ShellTool::with_policy(
        forge_tools::SandboxPolicy::default(),
    )));
    let mut live = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(WorkspaceOpsProvider {
            marker: "fresh".into(),
        }),
        Arc::new(HeuristicRouter::new(cfg.clone())),
        tools,
        Box::new(HeadlessPresenter::new(false)),
        cfg,
        a.to_str().unwrap(),
    )
    .unwrap();
    live.reset_fresh(b.to_str().unwrap()).unwrap();
    assert_eq!(live.workspace_root(), b.canonicalize().unwrap());
    assert_eq!(
        live.cached_agents_md(),
        Some("B guidance"),
        "fresh transition replaces A project guidance with B guidance"
    );
    assert_eq!(
        live.workspace_scope(),
        b.canonicalize().unwrap().display().to_string()
    );
    live.run_turn("write fresh").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(b.join("marker.txt")).unwrap(),
        "fresh"
    );
    assert!(!a.join("marker.txt").exists());
    let _ = std::fs::remove_dir_all(base);
}
