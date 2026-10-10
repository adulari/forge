//! Session workspace scoping for tool execution.
//!
//! This module owns argument rooting, containment validation, and task-local
//! workspace binding so tools cannot escape a session's canonical workspace.

use async_trait::async_trait;
use forge_types::{FileDiff, SideEffect};
use serde_json::Value;

use crate::{Tool, ToolError, SESSION_EXTRA_ROOTS, SESSION_WORKSPACE};

pub(crate) struct WorkspaceTool {
    pub(crate) inner: Box<dyn Tool>,
    pub(crate) workspace: std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>,
    pub(crate) extra_roots: std::sync::Arc<std::sync::RwLock<Vec<std::path::PathBuf>>>,
}

impl WorkspaceTool {
    /// Root `args` in the workspace, steer main-checkout paths into a linked worktree, and widen
    /// the readable roots for read-only tools.
    fn scoped(
        &self,
        args: &Value,
        workspace: &std::path::Path,
        mut extra_roots: Vec<std::path::PathBuf>,
    ) -> (Value, Vec<std::path::PathBuf>) {
        let read_only = self.inner.side_effect() == SideEffect::ReadOnly;
        let mut args = root_workspace_args(self.inner.name(), args, workspace);
        if let Some(origin) = worktree_origin(workspace) {
            args = remap_origin_paths(&args, &origin, workspace, read_only);
        }
        if read_only {
            extra_roots.extend(read_only_roots(workspace));
        }
        (args, extra_roots)
    }
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
        let (args, extra_roots) = self.scoped(args, &workspace, extra_roots);
        validate_workspace_args(&args, &workspace, &extra_roots).ok()?;
        SESSION_EXTRA_ROOTS
            .scope(
                extra_roots,
                SESSION_WORKSPACE.scope(workspace, self.inner.preview(&args)),
            )
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
        let (args, extra_roots) = self.scoped(args, &workspace, extra_roots);
        validate_workspace_args(&args, &workspace, &extra_roots)?;
        SESSION_EXTRA_ROOTS
            .scope(
                extra_roots,
                SESSION_WORKSPACE.scope(workspace, self.inner.run(&args)),
            )
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
                    "{key} '{path}' resolves outside the workspace {}; use a path inside it \
                     (relative paths resolve against it)",
                    workspace.display()
                )));
            }
        }
    }
    for path in nested_paths(args) {
        let target = crate::core_tools::normalize_target(std::path::Path::new(path));
        if !is_allowed(&target) {
            return Err(ToolError::Failed(format!(
                "path '{path}' resolves outside the workspace {}; use a path inside it \
                 (relative paths resolve against it)",
                workspace.display()
            )));
        }
    }
    Ok(())
}

/// Paths carried in list arguments: `paths[]` (batched reads) and `edits[].path` (multi-file edits).
fn nested_paths(args: &Value) -> impl Iterator<Item = &str> {
    let listed = args.get("paths").and_then(Value::as_array);
    let edits = args.get("edits").and_then(Value::as_array);
    listed
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(
            edits
                .into_iter()
                .flatten()
                .filter_map(|edit| edit.get("path").and_then(Value::as_str)),
        )
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
    let root = |value: &mut Value| {
        if let Value::String(path) = value {
            let candidate = std::path::Path::new(path.as_str());
            if candidate.is_relative() {
                *path = workspace.join(candidate).display().to_string();
            }
        }
    };
    if let Some(Value::Array(paths)) = object.get_mut("paths") {
        paths.iter_mut().for_each(root);
    }
    if let Some(Value::Array(edits)) = object.get_mut("edits") {
        for edit in edits {
            if let Some(path) = edit.get_mut("path") {
                root(path);
            }
        }
    }
    Value::Object(object)
}

/// The main checkout a linked git worktree belongs to (`None` for an ordinary checkout, a bare
/// repo or a submodule). Read from the worktree's `.git` file, so no `git` process is spawned.
pub fn worktree_origin(workspace: &std::path::Path) -> Option<std::path::PathBuf> {
    let marker = workspace.join(".git");
    if !marker.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(&marker).ok()?;
    let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let gitdir = workspace.join(gitdir);
    let common = match std::fs::read_to_string(gitdir.join("commondir")) {
        Ok(rel) => gitdir.join(rel.trim()),
        Err(_) => gitdir,
    };
    let common = common.canonicalize().ok()?;
    if common.file_name()? != ".git" {
        return None;
    }
    common.parent().map(std::path::Path::to_path_buf)
}

/// Git administrative directories a linked worktree writes to on `git add`/`commit` (its own
/// gitdir plus the shared common dir). They live outside the worktree, so a write sandbox rooted
/// at the worktree must be told about them or every git write fails.
pub fn worktree_git_dirs(workspace: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(origin) = worktree_origin(workspace) else {
        return Vec::new();
    };
    vec![origin.join(".git")]
}

/// Directories read-only tools may read beyond the workspace: the main checkout of a linked
/// worktree, where untracked files such as `node_modules` and build output exist only. The system
/// temp dir stays closed on purpose — sibling temp workspaces must not see each other.
pub fn read_only_roots(workspace: &std::path::Path) -> Vec<std::path::PathBuf> {
    worktree_origin(workspace).into_iter().collect()
}

/// Re-point absolute paths that name the main checkout of a linked worktree at the same relative
/// location inside the worktree. A session in a worktree is told, and `git` reports, absolute
/// paths of the main checkout; resolving them literally either refuses the call or — worse —
/// edits the checkout the worktree was created to isolate. `read_only` calls keep the original
/// path when the worktree has no such file (ignored directories live only in the main checkout).
pub fn remap_origin_paths(
    args: &Value,
    origin: &std::path::Path,
    workspace: &std::path::Path,
    read_only: bool,
) -> Value {
    let mut args = args.clone();
    let remap = |value: &mut Value| {
        let Value::String(text) = value else { return };
        let original = std::path::Path::new(text.as_str());
        if !original.is_absolute() || original.starts_with(workspace) {
            return;
        }
        let Ok(relative) = original.strip_prefix(origin) else {
            return;
        };
        if relative.starts_with(".forge/worktrees") {
            return;
        }
        let mapped = if relative.as_os_str().is_empty() {
            workspace.to_path_buf()
        } else {
            workspace.join(relative)
        };
        if read_only && !mapped.exists() && original.exists() {
            return;
        }
        *text = mapped.display().to_string();
    };
    let Some(object) = args.as_object_mut() else {
        return args;
    };
    for key in ["path", "cwd"] {
        if let Some(value) = object.get_mut(key) {
            remap(value);
        }
    }
    if let Some(Value::Array(paths)) = object.get_mut("paths") {
        paths.iter_mut().for_each(remap);
    }
    if let Some(Value::Array(edits)) = object.get_mut("edits") {
        for edit in edits {
            if let Some(value) = edit.get_mut("path") {
                remap(value);
            }
        }
    }
    args
}

#[cfg(test)]
mod worktree_scope_tests {
    use super::*;
    use serde_json::json;

    fn linked_worktree() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("repo");
        std::fs::create_dir_all(&origin).unwrap();
        let origin = origin.canonicalize().unwrap();
        let admin = origin.join(".git/worktrees/wt");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        let wt = dir.path().join("elsewhere/wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        (dir, origin, wt.canonicalize().unwrap())
    }

    #[test]
    fn origin_of_a_linked_worktree_is_its_main_checkout() {
        let (_dir, origin, wt) = linked_worktree();
        assert_eq!(worktree_origin(&wt), Some(origin.clone()));
        assert_eq!(worktree_git_dirs(&wt), vec![origin.join(".git")]);
        assert_eq!(worktree_origin(&origin), None);
    }

    #[test]
    fn writes_aimed_at_the_main_checkout_land_in_the_worktree() {
        let (_dir, origin, wt) = linked_worktree();
        let args = json!({
            "path": origin.join("src/a.rs"),
            "paths": [origin.join("b.rs"), "rel.rs", wt.join("c.rs")],
            "cwd": origin,
            "edits": [{ "path": origin.join("d.rs"), "old": "x", "new": "y" }],
        });
        let out = remap_origin_paths(&args, &origin, &wt, false);
        assert_eq!(out["path"], json!(wt.join("src/a.rs")));
        assert_eq!(
            out["paths"],
            json!([wt.join("b.rs"), "rel.rs", wt.join("c.rs")])
        );
        assert_eq!(out["cwd"], json!(wt));
        assert_eq!(out["edits"][0]["path"], json!(wt.join("d.rs")));
    }

    #[test]
    fn reads_fall_back_to_the_main_checkout_for_files_only_it_has() {
        let (_dir, origin, wt) = linked_worktree();
        std::fs::create_dir_all(origin.join("node_modules")).unwrap();
        std::fs::write(origin.join("node_modules/x.js"), "1").unwrap();
        let only_origin = origin.join("node_modules/x.js");
        let args = json!({ "path": only_origin });
        assert_eq!(
            remap_origin_paths(&args, &origin, &wt, true)["path"],
            json!(only_origin)
        );
        assert_eq!(
            remap_origin_paths(&args, &origin, &wt, false)["path"],
            json!(wt.join("node_modules/x.js"))
        );
        std::fs::create_dir_all(wt.join("node_modules")).unwrap();
        std::fs::write(wt.join("node_modules/x.js"), "2").unwrap();
        assert_eq!(
            remap_origin_paths(&args, &origin, &wt, true)["path"],
            json!(wt.join("node_modules/x.js"))
        );
    }

    #[test]
    fn a_sibling_worktree_is_not_remapped() {
        let (_dir, origin, wt) = linked_worktree();
        let sibling = origin.join(".forge/worktrees/other/a.rs");
        let args = json!({ "path": sibling });
        assert_eq!(remap_origin_paths(&args, &origin, &wt, false), args);
    }
}
