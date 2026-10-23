//! The `[permissions] auto_allow_read_only_shell` setting as the broker sees it: an inert rule.

use crate::{PermissionDecision, PermissionRule, RuleSource};

/// Tool name of the inert rule that carries `[permissions] auto_allow_read_only_shell = false`
/// into the broker. It names no real tool, so [`PermissionRule`] matching never selects it; the
/// broker reads it as a setting. Rules are the one per-session policy input every decision site
/// already receives (native loop, CLI bridge, subagents), so a setting carried this way reaches
/// all of them without a new parameter on each.
pub const NO_AUTO_ALLOW_READ_ONLY_SHELL: &str = "@permissions/auto_allow_read_only_shell";

/// The rule that turns Ask-mode auto-allow of provably read-only shell commands off.
pub fn read_only_shell_opt_out_rule() -> PermissionRule {
    PermissionRule {
        tool: NO_AUTO_ALLOW_READ_ONLY_SHELL.to_string(),
        patterns: Vec::new(),
        decision: PermissionDecision::Deny,
        source: RuleSource::Configured,
        reason: Some("[permissions] auto_allow_read_only_shell = false".into()),
    }
}

/// Whether `rules` leave Ask-mode auto-allow of read-only shell commands on (the default).
pub fn auto_allow_read_only_shell_enabled(rules: &[PermissionRule]) -> bool {
    !rules
        .iter()
        .any(|r| r.tool == NO_AUTO_ALLOW_READ_ONLY_SHELL)
}
