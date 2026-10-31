//! `[harness] edit_mode = "hashline"` through a real session: the model-visible surface, and an
//! anchored `edit_file` going through the permission broker, hooks and `/undo` like any write.

use super::*;
use forge_config::EditMode;

struct AnchoredEdit {
    path: String,
    tag: String,
}

#[async_trait::async_trait]
impl Provider for AnchoredEdit {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let done = messages.iter().any(|m| m.role == Role::Tool);
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if done { "done" } else { "editing" }.into(),
            tool_calls: if done {
                vec![]
            } else {
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "edit_file".into(),
                    args: serde_json::json!({
                        "path": self.path,
                        "tag": self.tag,
                        "edits": [{"start": 2, "new": "BETA"}, {"after": 3, "new": "delta"}]
                    }),
                }]
            },
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn hashline_config(mode: PermissionMode) -> Config {
    let mut config = Config {
        permission_mode: mode,
        ..Config::default()
    };
    config.harness.edit_mode = EditMode::Hashline;
    config
}

fn session(dir: &std::path::Path, provider: Arc<dyn Provider>, config: Config) -> Session {
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(HeadlessPresenter::new(false)),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

fn workspace_with(text: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("forge-hashline-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("f.txt");
    std::fs::write(&file, text).unwrap();
    (dir, file)
}

const SOURCE: &str = "alpha\nbeta\ngamma\n";

fn provider_for(file: &std::path::Path) -> Arc<AnchoredEdit> {
    Arc::new(AnchoredEdit {
        path: file.to_string_lossy().into_owned(),
        tag: forge_tools::hashline_file_tag(SOURCE),
    })
}

#[test]
fn only_hashline_mode_changes_the_tools_and_the_prompt() {
    let (dir, _) = workspace_with(SOURCE);
    let off = session(&dir, Arc::new(MockProvider), Config::default());
    let on = session(
        &dir,
        Arc::new(MockProvider),
        hashline_config(PermissionMode::Default),
    );
    let (off_specs, on_specs) = (off.full_tool_specs(), on.full_tool_specs());
    let spec = |specs: &[ToolSpec], name: &str| specs.iter().find(|s| s.name == name).cloned();

    assert!(spec(&off_specs, "multi_edit").is_some());
    assert!(spec(&off_specs, "edit_file").unwrap().schema["properties"]["old"].is_object());
    assert!(!off.system_preamble()[0].content.contains("path#TAG"));

    assert!(spec(&on_specs, "multi_edit").is_none());
    let edit = spec(&on_specs, "edit_file").unwrap();
    assert!(edit.schema["properties"]["edits"].is_object());
    assert!(edit.schema["properties"]["old"].is_null());
    assert!(on.system_preamble()[0].content.contains("path#TAG"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn an_anchored_edit_applies_and_undo_restores_the_file() {
    let (dir, file) = workspace_with(SOURCE);
    let mut session = session(
        &dir,
        provider_for(&file),
        hashline_config(PermissionMode::Bypass),
    );
    session.set_checkpoint_root(dir.join("snaps"));
    session.run_turn("rename beta").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "alpha\nBETA\ngamma\ndelta\n"
    );
    let report = session.undo().unwrap().unwrap().restore;
    assert!(
        report.restored.iter().any(|p| p.contains("f.txt")),
        "{report:?}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), SOURCE);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn the_permission_broker_still_gates_an_anchored_edit() {
    let (dir, file) = workspace_with(SOURCE);
    let mut session = session(
        &dir,
        provider_for(&file),
        hashline_config(PermissionMode::Default),
    );
    session.run_turn("rename beta").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        SOURCE,
        "an unattended session in default mode must not write"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[tokio::test]
async fn a_blocking_pre_tool_hook_stops_an_anchored_edit() {
    let (dir, file) = workspace_with(SOURCE);
    let mut config = hashline_config(PermissionMode::Bypass);
    config.hooks = vec![forge_config::HookConfig {
        event: forge_config::HookEvent::PreToolUse,
        kind: Default::default(),
        if_rule: None,
        url: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        run_async: false,
        matcher: Some("edit_file".into()),
        command: "echo blocked-by-hook >&2; exit 2".into(),
        prompt: None,
        model: None,
        timeout_secs: 10,
        cc_compat: true,
    }];
    let mut session = session(&dir, provider_for(&file), config);
    session.run_turn("rename beta").await.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), SOURCE);
    let _ = std::fs::remove_dir_all(dir);
}
