//! Session adjustments the chat's slash commands make: `/add-dir`, `/permissions`, and
//! `/mcp reconnect`. Each one changes only this live session; nothing is written to config.

use super::*;

/// The reason a `/permissions allow|ask|deny` rule carries, so the listing can say where it came
/// from (it lives only as long as the session).
pub const SESSION_RULE_REASON: &str = "added with /permissions for this session";

impl Session {
    /// Let the structured file tools work in `dir` for the rest of this session, exactly as a
    /// `[tools] extra_roots` entry would: reachable when the tools are confined, and inside the
    /// project for Auto-edit's write rule. Returns the canonical directory.
    pub fn add_dir(&mut self, dir: &std::path::Path) -> Result<std::path::PathBuf, CoreError> {
        let absolute = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            self.workspace.root().join(dir)
        };
        let canonical = absolute
            .canonicalize()
            .map_err(|e| CoreError::Workspace(format!("cannot add {}: {e}", absolute.display())))?;
        if !canonical.is_dir() {
            return Err(CoreError::Workspace(format!(
                "{} is not a directory",
                canonical.display()
            )));
        }
        let text = canonical.to_string_lossy().into_owned();
        if !self.config.tools.extra_roots.contains(&text) {
            self.config.tools.extra_roots.push(text);
            self.extra_tool_roots.push(canonical.clone());
            self.tools
                .bind_extra_roots(self.extra_tool_roots.clone())
                .map_err(|e| CoreError::Internal(e.to_string()))?;
        }
        Ok(canonical)
    }

    /// The directories file tools may use beyond the workspace (`[tools] extra_roots` plus every
    /// `/add-dir`).
    pub fn added_dirs(&self) -> &[String] {
        &self.config.tools.extra_roots
    }

    /// Add a rule for the rest of this session (`/permissions allow Bash(git log:*)`).
    pub fn add_session_rule(
        &mut self,
        tool: String,
        patterns: Vec<String>,
        decision: forge_types::PermissionDecision,
    ) {
        self.rules.push(forge_types::PermissionRule {
            tool,
            patterns,
            decision,
            source: forge_types::RuleSource::Configured,
            reason: Some(SESSION_RULE_REASON.to_string()),
        });
    }

    /// The live MCP connection pool, for work that must not hold the session lock while a
    /// server connects (`/mcp reconnect`). `None` when no servers are configured.
    pub fn mcp_manager(&self) -> Option<Arc<forge_mcp::McpManager>> {
        self.mcp.clone()
    }
}
