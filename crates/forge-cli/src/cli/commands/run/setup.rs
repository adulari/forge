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
    let rtk = rtk_rewriter(config);
    if !(config.shell.sandbox || config.shell.scoped_cargo_target) {
        // No sandbox knobs: keep the default (unconfined) shell tool, RTK-routed when enabled.
        return rtk.map(|r| forge_tools::ShellTool::default().with_rtk(Some(r)));
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
    Some(
        forge_tools::ShellTool::with_policy_in_workspace(
            forge_tools::SandboxPolicy {
                enabled: config.shell.sandbox,
                writable,
                cargo_target_base,
            },
            workspace,
        )
        .with_rtk(rtk),
    )
}

/// The RTK rewriter for `[shell] rtk` (docs/features/token-savings.md): `auto` uses a genuine
/// `rtk` found on `PATH` (probed once per process — the probe spawns `rtk --version`), `on`
/// insists on `rtk` even when the probe failed (a missing binary then surfaces as the command's
/// own error instead of silently running unfiltered), `off` never rewrites.
pub(crate) fn rtk_rewriter(config: &forge_config::Config) -> Option<forge_tools::RtkRewriter> {
    use forge_config::AutoToggle;
    static DETECTED: std::sync::OnceLock<Option<forge_tools::RtkRewriter>> =
        std::sync::OnceLock::new();
    if config.shell.rtk == AutoToggle::Off {
        return None;
    }
    let detected = DETECTED
        .get_or_init(forge_tools::RtkRewriter::detect)
        .clone();
    let rewriter = match (config.shell.rtk, detected) {
        (AutoToggle::On, None) => forge_tools::RtkRewriter::at("rtk"),
        (_, Some(r)) => r,
        (_, None) => return None,
    };
    Some(rewriter.skipping(config.shell.rtk_skip.clone()))
}

#[allow(dead_code)]
pub(crate) fn sandboxed_shell_tool(
    config: &forge_config::Config,
) -> Option<forge_tools::ShellTool> {
    let workspace = std::env::current_dir().ok()?;
    sandboxed_shell_tool_in(config, &workspace)
}

/// Resolve `[tools] extra_roots` (an opt-in allowlist of extra roots the structured file tools —
/// read_file/write_file/edit_file/multi_edit/apply_patch/append_file/notebook_edit/delete_file/
/// list_dir/search/glob — may read and write in addition to the session workspace) into
/// canonicalized paths for `ToolRegistry::bind_extra_roots`. Relative entries are ignored.
/// Shared by `forge run` and the `mcp-serve` CLI-bridge path so both entry points stay in sync.
pub(crate) fn resolve_extra_tool_roots(config: &forge_config::Config) -> Vec<std::path::PathBuf> {
    config
        .tools
        .extra_roots
        .iter()
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect()
}
