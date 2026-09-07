//! Session workspace scoping for tool execution.
//!
//! This module owns argument rooting, containment validation, and task-local
//! workspace binding so tools cannot escape a session's canonical workspace.

use async_trait::async_trait;
use forge_types::{FileDiff, SideEffect};
use serde_json::Value;

use crate::{Tool, ToolError, SESSION_WORKSPACE};

pub(crate) struct WorkspaceTool {
    pub(crate) inner: Box<dyn Tool>,
    pub(crate) workspace: std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>,
    pub(crate) extra_roots: std::sync::Arc<std::sync::RwLock<Vec<std::path::PathBuf>>>,
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
        let extra_roots = self.extra_roots.read().ok()?.clone();
        let args = root_workspace_args(self.inner.name(), args, &workspace);
        validate_workspace_args(&args, &workspace, &extra_roots).ok()?;
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
        let extra_roots = self
            .extra_roots
            .read()
            .map_err(|_| ToolError::Failed("extra tool roots binding poisoned".to_string()))?
            .clone();
        let args = root_workspace_args(self.inner.name(), args, &workspace);
        validate_workspace_args(&args, &workspace, &extra_roots)?;
        SESSION_WORKSPACE
            .scope(workspace, self.inner.run(&args))
            .await
    }
}

pub(crate) fn validate_workspace_args(
    args: &Value,
    workspace: &std::path::Path,
    extra_roots: &[std::path::PathBuf],
) -> Result<(), ToolError> {
    let is_allowed = |target: &std::path::Path| {
        target.starts_with(workspace) || extra_roots.iter().any(|root| target.starts_with(root))
    };
    for key in ["path", "cwd"] {
        if let Some(path) = args.get(key).and_then(Value::as_str) {
            let target = crate::core_tools::normalize_target(std::path::Path::new(path));
            if !is_allowed(&target) {
                return Err(ToolError::Failed(format!(
                    "{key} resolves outside the workspace"
                )));
            }
        }
    }
    if let Some(paths) = args.get("paths").and_then(Value::as_array) {
        for path in paths.iter().filter_map(Value::as_str) {
            let target = crate::core_tools::normalize_target(std::path::Path::new(path));
            if !is_allowed(&target) {
                return Err(ToolError::Failed(
                    "path resolves outside the workspace".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn root_workspace_args(
    tool_name: &str,
    args: &Value,
    workspace: &std::path::Path,
) -> Value {
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
