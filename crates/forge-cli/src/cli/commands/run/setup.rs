//! Shell sandbox setup for interactive runs.

/// Build the `shell` tool's Landlock sandbox and/or scoped `CARGO_TARGET_DIR` carve-out from
/// `[shell]` config (`sandbox` / `scoped_cargo_target`, ADR-0008 + PR #521). Returns `None` when
/// both knobs are off, in which case the caller should keep the plain `ShellTool::default()`
/// already registered by `ToolRegistry::with_core_tools()`. Shared by `forge run` (this file) and
/// the `mcp-serve` CLI-bridge path (`crate::mcp_serve::run`) so the two entry points can't drift —
/// a bridged claude/codex agent gets the same compile-check carve-out as a direct `forge run`
/// session.
pub(crate) fn sandboxed_shell_tool_in(
    config: &forge_config::Config,
    workspace: &std::path::Path,
) -> Option<forge_tools::ShellTool> {
    if !(config.shell.sandbox || config.shell.scoped_cargo_target) {
        return None;
    }
    let writable = config
        .shell
        .sandbox_writable
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    let cargo_target_base = config.shell.scoped_cargo_target.then(|| {
        config
            .shell
            .scoped_cargo_target_dir
            .clone()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("forge-cargo-target"))
    });
    Some(forge_tools::ShellTool::with_policy_in_workspace(
        forge_tools::SandboxPolicy {
            enabled: config.shell.sandbox,
            writable,
            cargo_target_base,
        },
        workspace,
    ))
}

#[allow(dead_code)]
pub(crate) fn sandboxed_shell_tool(
    config: &forge_config::Config,
) -> Option<forge_tools::ShellTool> {
    let workspace = std::env::current_dir().ok()?;
    sandboxed_shell_tool_in(config, &workspace)
}
