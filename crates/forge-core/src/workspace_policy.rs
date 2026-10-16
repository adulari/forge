//! Per-call workspace policy for the structured file tools: the containment check that still
//! applies when the tools are not confined, and the Auto-edit escalation for writes beyond the
//! project (docs/features/temper-modes.md, "File tools outside the workspace").

use super::*;

impl Session {
    /// The workspace containment check for one call: the root allowlist, plus the sibling-worktree
    /// write refusal that holds even when the file tools are not confined to the workspace.
    pub(crate) fn check_workspace(
        &self,
        tool: &str,
        args: &serde_json::Value,
        roots: &[std::path::PathBuf],
    ) -> Result<(), CoreError> {
        validate_workspace_args(args, &self.workspace, roots)?;
        if let Some(stale) = self.stale_host_path(args) {
            return Err(CoreError::StaleHostPath(stale));
        }
        let read_only = self
            .tools
            .get(tool)
            .is_some_and(|t| t.side_effect() == forge_types::SideEffect::ReadOnly);
        if !read_only {
            if let Some(target) = forge_tools::sibling_worktree_write(args, self.workspace.root()) {
                return Err(CoreError::Workspace(format!(
                    "{target} is inside another agent's worktree; write inside this session's \
                     workspace ({}) instead",
                    self.workspace.root().display()
                )));
            }
        }
        Ok(())
    }

    /// Whether a file-tool write leaves the workspace and every explicitly configured
    /// `tools.extra_roots` entry. Auto-edit trusts edits to the project, not to the whole disk,
    /// so such a write asks there instead of passing silently.
    pub(crate) fn writes_beyond_workspace(&self, args: &serde_json::Value) -> bool {
        writes_beyond_workspace(args, self.workspace.root(), &self.config.tools.extra_roots)
    }
}

/// [`Session::writes_beyond_workspace`] without a session: shared with the CLI bridge's
/// `mcp-serve`, whose Auto-edit gate must ask for the same writes the direct path asks for.
pub fn writes_beyond_workspace(
    args: &serde_json::Value,
    root: &std::path::Path,
    extra_roots: &[String],
) -> bool {
    let configured: Vec<std::path::PathBuf> = extra_roots
        .iter()
        .map(std::path::Path::new)
        .filter(|path| path.is_absolute())
        .map(normalize_workspace_target)
        .collect();
    let own = normalize_workspace_target(root);
    forge_tools::path_arg_targets(args, root)
        .iter()
        .any(|target| {
            !target.starts_with(&own) && !configured.iter().any(|r| target.starts_with(r))
        })
}

/// Auto-edit's write rule on any surface: an allowed write that lands beyond the workspace asks.
pub fn accept_edits_write_gate(
    decision: forge_types::PermissionDecision,
    mode: forge_types::PermissionMode,
    side_effect: forge_types::SideEffect,
    beyond_workspace: impl FnOnce() -> bool,
) -> forge_types::PermissionDecision {
    if decision == forge_types::PermissionDecision::Allow
        && mode == forge_types::PermissionMode::AcceptEdits
        && side_effect == forge_types::SideEffect::Write
        && beyond_workspace()
    {
        forge_types::PermissionDecision::Ask
    } else {
        decision
    }
}
