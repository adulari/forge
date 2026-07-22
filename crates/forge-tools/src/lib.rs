//! The `Tool` trait and Forge's core coding tools. Each tool declares its [`SideEffect`]
//! class, which the core's permission broker (ADR-0008) uses to decide whether to allow,
//! ask, or deny. Adding a tool is implementing this trait and registering it — no core
//! changes.

use std::collections::HashMap;

use async_trait::async_trait;
use forge_types::{FileDiff, SideEffect};
use serde_json::Value;

tokio::task_local! {
    pub(crate) static SESSION_WORKSPACE: std::path::PathBuf;
}

mod core_tools;
mod lattice_tool;
mod sandbox;
mod shell;
mod web;
pub use core_tools::{
    AppendFileTool, ApplyPatchTool, DeleteFileTool, EditFileTool, GlobTool, ListDirTool,
    MultiEditTool, NotebookEditTool, ReadFileTool, SearchTool, WriteFileTool,
};
pub use lattice_tool::LatticeTool;
pub use sandbox::{ApplyResult, SandboxPolicy};
pub use shell::ShellTool;
pub use web::{BraveSearch, DuckDuckGo, SearchBackend, SearchResult, WebFetchTool, WebSearchTool};

/// Run a shell command without a sandbox (for use by the autofix loop and other internal
/// callers that don't need filesystem confinement). Never returns `Err`.
pub async fn run_shell_command(command: &str, cwd: &str, timeout_secs: u64) -> String {
    shell::run_command(command, cwd, timeout_secs, &SandboxPolicy::default()).await
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("missing or invalid argument: {0}")]
    BadArgs(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tool execution failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn side_effect(&self) -> SideEffect;
    /// JSON Schema for the arguments object (advertised to the model).
    fn schema(&self) -> Value;
    async fn run(&self, args: &Value) -> Result<String, ToolError>;

    /// Compute the proposed change *without touching disk*, for diff-review before the write
    /// is confirmed. Returns `None` for tools that don't mutate files, or when a preview
    /// can't be produced (the real error then surfaces from `run`). Default: no preview.
    async fn preview(&self, _args: &Value) -> Option<FileDiff> {
        None
    }
}

/// Holds the available tools, looked up by name during the agent loop.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    workspace: Option<std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>>,
}

struct WorkspaceTool {
    inner: Box<dyn Tool>,
    workspace: std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>,
}

#[async_trait]
impl Tool for WorkspaceTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn side_effect(&self) -> SideEffect {
        self.inner.side_effect()
    }

    fn schema(&self) -> Value {
        self.inner.schema()
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let workspace = self.workspace.read().ok()?.clone();
        let args = root_workspace_args(self.inner.name(), args, &workspace);
        validate_workspace_args(&args, &workspace).ok()?;
        SESSION_WORKSPACE
            .scope(workspace, self.inner.preview(&args))
            .await
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let workspace = self
            .workspace
            .read()
            .map_err(|_| ToolError::Failed("session workspace binding poisoned".to_string()))?
            .clone();
        let args = root_workspace_args(self.inner.name(), args, &workspace);
        validate_workspace_args(&args, &workspace)?;
        SESSION_WORKSPACE
            .scope(workspace, self.inner.run(&args))
            .await
    }
}

fn validate_workspace_args(args: &Value, workspace: &std::path::Path) -> Result<(), ToolError> {
    for key in ["path", "cwd"] {
        if let Some(path) = args.get(key).and_then(Value::as_str) {
            let target = crate::core_tools::normalize_target(std::path::Path::new(path));
            if !target.starts_with(workspace) {
                return Err(ToolError::Failed(format!(
                    "{key} resolves outside the workspace"
                )));
            }
        }
    }
    if let Some(paths) = args.get("paths").and_then(Value::as_array) {
        for path in paths.iter().filter_map(Value::as_str) {
            let target = crate::core_tools::normalize_target(std::path::Path::new(path));
            if !target.starts_with(workspace) {
                return Err(ToolError::Failed(
                    "path resolves outside the workspace".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn root_workspace_args(tool_name: &str, args: &Value, workspace: &std::path::Path) -> Value {
    let Some(mut object) = args.as_object().cloned() else {
        return args.clone();
    };
    match tool_name {
        "shell" if !object.contains_key("cwd") => {
            object.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
        }
        "apply_patch" if !object.contains_key("cwd") => {
            object.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
        }
        "list_dir" | "search" | "glob" if !object.contains_key("path") => {
            object.insert(
                "path".to_string(),
                Value::String(workspace.display().to_string()),
            );
        }
        _ => {}
    }
    for key in ["path", "cwd"] {
        if let Some(Value::String(value)) = object.get_mut(key) {
            let candidate = std::path::Path::new(value);
            if candidate.is_relative() {
                *value = workspace.join(candidate).display().to_string();
            }
        }
    }
    if let Some(Value::Array(paths)) = object.get_mut("paths") {
        for path in paths {
            if let Value::String(path) = path {
                let candidate = std::path::Path::new(path);
                if candidate.is_relative() {
                    *path = workspace.join(candidate).display().to_string();
                }
            }
        }
    }
    Value::Object(object)
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_core_tools_in(workspace: &std::path::Path) -> Self {
        let mut registry = Self::with_core_tools();
        registry.bind_workspace(workspace);
        registry
    }

    /// Bind every registered tool to one mutable session workspace. Rebinding updates the
    /// shared binding without replacing configured tools, policies, lattice, or MCP adapters.
    pub fn bind_workspace(&mut self, workspace: &std::path::Path) {
        let workspace = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        let binding = std::sync::Arc::new(std::sync::RwLock::new(workspace.clone()));
        self.workspace = Some(std::sync::Arc::clone(&binding));
        self.tools = std::mem::take(&mut self.tools)
            .into_iter()
            .map(|(name, inner)| {
                (
                    name,
                    Box::new(WorkspaceTool {
                        inner,
                        workspace: std::sync::Arc::clone(&binding),
                    }) as Box<dyn Tool>,
                )
            })
            .collect();
    }

    /// Atomically retarget all scoped wrappers before a session publishes its new identity.
    pub fn rebind_workspace(&self, workspace: &std::path::Path) -> Result<(), ToolError> {
        let workspace = workspace
            .canonicalize()
            .map_err(|error| ToolError::Failed(format!("resolving session workspace: {error}")))?;
        let binding = self.workspace.as_ref().ok_or_else(|| {
            ToolError::Failed("tool registry has no workspace binding".to_string())
        })?;
        *binding
            .write()
            .map_err(|_| ToolError::Failed("session workspace binding poisoned".to_string()))? =
            workspace;
        Ok(())
    }

    /// Register all core coding tools.
    pub fn with_core_tools() -> Self {
        let mut r = Self::new();
        r.register(Box::new(ReadFileTool));
        r.register(Box::new(WriteFileTool));
        r.register(Box::new(AppendFileTool));
        r.register(Box::new(EditFileTool));
        r.register(Box::new(MultiEditTool));
        r.register(Box::new(ApplyPatchTool));
        r.register(Box::new(NotebookEditTool));
        r.register(Box::new(DeleteFileTool));
        r.register(Box::new(ShellTool::with_policy(SandboxPolicy::default())));
        r.register(Box::new(ListDirTool));
        r.register(Box::new(SearchTool));
        r.register(Box::new(GlobTool));
        r.register(Box::new(WebFetchTool));
        r.register(Box::new(WebSearchTool::new()));
        r
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        let tool = if let Some(workspace) = &self.workspace {
            Box::new(WorkspaceTool {
                inner: tool,
                workspace: std::sync::Arc::clone(workspace),
            }) as Box<dyn Tool>
        } else {
            tool
        };
        self.tools.insert(name, tool);
    }

    pub fn remove(&mut self, name: &str) {
        self.tools.remove(name);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }
}

/// Extract a required string argument from a JSON args object.
pub(crate) fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::BadArgs(format!("expected string '{key}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_default_injection_is_tool_specific() {
        let workspace = std::path::Path::new("/tmp/forge-wrapper-root");
        let web = serde_json::json!({ "query": "workspace status" });
        assert_eq!(
            root_workspace_args("web_search", &web, workspace),
            web,
            "web_search query must not grow a path"
        );
        let custom = serde_json::json!({ "command": "custom" });
        assert_eq!(
            root_workspace_args("custom_tool", &custom, workspace),
            custom,
            "custom tool args must remain opaque"
        );
        assert!(
            !root_workspace_args("read_file", &serde_json::json!({}), workspace)
                .as_object()
                .unwrap()
                .contains_key("path"),
            "required read_file path must not be fabricated"
        );
        assert_eq!(
            root_workspace_args("list_dir", &serde_json::json!({}), workspace)["path"],
            workspace.display().to_string()
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn scoped_registry_defaults_and_late_shell_registration_rebind() {
        let base =
            std::env::temp_dir().join(format!("forge-scoped-defaults-{}", std::process::id()));
        let first = base.join("first");
        let second = base.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("first.txt"), "first").unwrap();
        std::fs::write(second.join("second.txt"), "second").unwrap();
        let mut registry = ToolRegistry::with_core_tools_in(&first);
        registry.register(Box::new(ShellTool::with_policy(SandboxPolicy::default())));
        let shell = registry.get("shell").unwrap();
        assert!(shell
            .run(&serde_json::json!({ "command": "pwd" }))
            .await
            .unwrap()
            .contains(first.to_string_lossy().as_ref()));
        assert!(registry
            .get("list_dir")
            .unwrap()
            .run(&serde_json::json!({}))
            .await
            .unwrap()
            .contains("first.txt"));
        registry.rebind_workspace(&second).unwrap();
        assert!(shell
            .run(&serde_json::json!({ "command": "pwd" }))
            .await
            .unwrap()
            .contains(second.to_string_lossy().as_ref()));
        assert!(registry
            .get("list_dir")
            .unwrap()
            .run(&serde_json::json!({}))
            .await
            .unwrap()
            .contains("second.txt"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn scoped_registry_roots_file_tools_and_rejects_peer_workspace() {
        let base = std::env::temp_dir().join(format!("forge-scoped-tools-{}", std::process::id()));
        let workspace = base.join("workspace");
        let peer = base.join("peer");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&peer).unwrap();
        std::fs::write(workspace.join("inside.txt"), "inside").unwrap();
        std::fs::write(peer.join("peer.txt"), "peer").unwrap();

        let registry = ToolRegistry::with_core_tools_in(&workspace);
        let read = registry.get("read_file").unwrap();
        assert_eq!(
            read.run(&serde_json::json!({ "path": "inside.txt" }))
                .await
                .unwrap(),
            "inside"
        );
        for path in [peer.join("peer.txt"), workspace.join("../peer/peer.txt")] {
            assert!(read
                .run(&serde_json::json!({ "path": path }))
                .await
                .is_err());
        }
        let write = registry.get("write_file").unwrap();
        for path in [
            peer.join("written.txt"),
            workspace.join("../peer/traversal.txt"),
        ] {
            assert!(write
                .run(&serde_json::json!({ "path": path, "content": "escape" }))
                .await
                .is_err());
        }
        let delete = registry.get("delete_file").unwrap();
        assert!(delete
            .run(&serde_json::json!({ "path": workspace.join("../peer/peer.txt") }))
            .await
            .is_err());
        write
            .run(&serde_json::json!({ "path": "written.txt", "content": "owned" }))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("written.txt")).unwrap(),
            "owned"
        );
        assert_eq!(
            std::fs::read_to_string(peer.join("peer.txt")).unwrap(),
            "peer"
        );
        assert!(!peer.join("written.txt").exists());
        assert!(!peer.join("traversal.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scoped_registry_rejects_existing_and_nonexistent_symlink_escapes() {
        let base =
            std::env::temp_dir().join(format!("forge-scoped-symlink-{}", std::process::id()));
        let workspace = base.join("workspace");
        let peer = base.join("peer");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&peer).unwrap();
        std::fs::write(peer.join("secret.txt"), "peer").unwrap();
        std::os::unix::fs::symlink(&peer, workspace.join("link")).unwrap();
        let registry = ToolRegistry::with_core_tools_in(&workspace);
        let read = registry.get("read_file").unwrap();
        let write = registry.get("write_file").unwrap();
        assert!(read
            .run(&serde_json::json!({ "path": "link/secret.txt" }))
            .await
            .is_err());
        assert!(write
            .run(&serde_json::json!({ "path": "link/new.txt", "content": "escape" }))
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(peer.join("secret.txt")).unwrap(),
            "peer"
        );
        assert!(!peer.join("new.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn scoped_registry_previews_are_rooted_and_confined() {
        let base =
            std::env::temp_dir().join(format!("forge-scoped-preview-{}", std::process::id()));
        let workspace = base.join("workspace");
        let peer = base.join("peer");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&peer).unwrap();
        let registry = ToolRegistry::with_core_tools_in(&workspace);
        let write = registry.get("write_file").unwrap();
        assert!(write
            .preview(&serde_json::json!({ "path": "inside.txt", "content": "inside" }))
            .await
            .is_some());
        for path in [peer.join("peer.txt"), workspace.join("../peer/peer.txt")] {
            assert!(write
                .preview(&serde_json::json!({ "path": path, "content": "escape" }))
                .await
                .is_none());
        }
        assert!(!workspace.join("inside.txt").exists());
        assert!(!peer.join("peer.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn registry_has_core_tools() {
        let r = ToolRegistry::with_core_tools();
        for name in [
            "read_file",
            "write_file",
            "append_file",
            "edit_file",
            "delete_file",
            "shell",
            "list_dir",
            "search",
            "glob",
            "web_fetch",
            "web_search",
        ] {
            assert!(r.get(name).is_some(), "missing tool: {name}");
        }
    }

    #[tokio::test]
    async fn write_file_preview_new_path_is_created_kind() {
        let path = std::env::temp_dir().join(format!("forge-prev-{}.txt", forge_types::new_id()));
        let args = serde_json::json!({ "path": path.to_str().unwrap(), "content": "hi there" });
        let diff = WriteFileTool
            .preview(&args)
            .await
            .expect("preview for a write");
        assert_eq!(diff.kind, forge_types::DiffKind::Created);
        assert!(diff.old.is_none(), "no prior content for a new file");
        assert_eq!(diff.new.as_deref(), Some("hi there"));
        // preview must NOT create the file.
        assert!(!path.exists(), "preview is side-effect-free");
    }

    #[tokio::test]
    async fn append_file_adds_verbatim_chunks_and_previews_the_combined_file() {
        let path = std::env::temp_dir().join(format!("forge-append-{}.txt", forge_types::new_id()));
        std::fs::write(&path, "first\n").unwrap();
        let args = serde_json::json!({ "path": path, "content": "second\n" });
        let diff = AppendFileTool.preview(&args).await.expect("append preview");
        assert_eq!(diff.old.as_deref(), Some("first\n"));
        assert_eq!(diff.new.as_deref(), Some("first\nsecond\n"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");

        AppendFileTool.run(&args).await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn read_only_tool_has_no_preview() {
        assert!(ReadFileTool
            .preview(&serde_json::json!({"path":"x"}))
            .await
            .is_none());
    }

    #[test]
    fn side_effect_classes_are_correct() {
        let r = ToolRegistry::with_core_tools();
        assert_eq!(
            r.get("read_file").unwrap().side_effect(),
            SideEffect::ReadOnly
        );
        assert_eq!(
            r.get("write_file").unwrap().side_effect(),
            SideEffect::Write
        );
        assert_eq!(
            r.get("append_file").unwrap().side_effect(),
            SideEffect::Write
        );
        assert_eq!(r.get("shell").unwrap().side_effect(), SideEffect::Shell);
        assert_eq!(r.get("edit_file").unwrap().side_effect(), SideEffect::Write);
        assert_eq!(
            r.get("list_dir").unwrap().side_effect(),
            SideEffect::ReadOnly
        );
        assert_eq!(r.get("search").unwrap().side_effect(), SideEffect::ReadOnly);
        assert_eq!(r.get("glob").unwrap().side_effect(), SideEffect::ReadOnly);
        assert_eq!(
            r.get("delete_file").unwrap().side_effect(),
            SideEffect::Write
        );
        assert_eq!(
            r.get("web_fetch").unwrap().side_effect(),
            SideEffect::Network
        );
        assert_eq!(
            r.get("web_search").unwrap().side_effect(),
            SideEffect::Network
        );
    }
}
