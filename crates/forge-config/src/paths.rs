//! Filesystem locations and project-initialization policy.
//!
//! This module owns platform directories and project-local setup detection so
//! config loading does not depend on ambient process working directories.

use std::path::{Path, PathBuf};

/// Per-OS config directory: `<config>/forge`.
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "forge", "forge").map(|d| d.config_dir().to_path_buf())
}

/// Per-OS data directory: `<data>/forge` (e.g. `~/.local/share/forge`). The session + usage store
/// lives here so spend/budget and history persist across restarts and are shared regardless of the
/// directory `forge` is launched from (FR-5 budget is global, not per-project).
pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "forge", "forge").map(|d| d.data_dir().to_path_buf())
}

/// Claude Code's home directory (`~/.claude`), source for `forge import claude`. `None` if no
/// home directory resolves on this platform.
pub fn claude_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".claude"))
}

/// Codex CLI's home directory (`~/.codex`), source for `forge import codex`. Custom prompts live
/// under `~/.codex/prompts/*.md` (plain markdown slash-command templates). `None` if no home
/// directory resolves on this platform.
pub fn codex_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".codex"))
}

/// Cursor AI's home directory (`~/.cursor`), source for `forge import cursor`. Rules live under
/// `~/.cursor/rules/*.mdc`. `None` if no home directory resolves on this platform.
pub fn cursor_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".cursor"))
}

/// Home directory, `None` if not resolvable. Used by `forge import aider` to locate convention
/// files that don't follow a fixed tool-specific directory structure.
pub fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
}

/// Whether a project contains Forge-specific guidance, configuration, agents, skills, or commands.
///
/// This is the canonical project-initialization check. Callers must pass the session's effective
/// working directory so worktrees and multi-project servers are evaluated correctly.
pub fn project_initialization(cwd: &Path) -> ProjectInitialization {
    let forge = cwd.join(".forge");
    let initialized = [
        cwd.join("AGENTS.md"),
        cwd.join("FORGE.md"),
        cwd.join("CLAUDE.md"),
        forge.join("AGENTS.md"),
        forge.join("FORGE.md"),
        forge.join("config.toml"),
        forge.join("agents.md"),
        forge.join("mcp.toml"),
        forge.join("settings.json"),
    ]
    .iter()
    .any(|path| path.is_file())
        || directory_has_entries(&forge.join("agents"))
        || directory_has_entries(&forge.join("skills"))
        || directory_has_entries(&forge.join("commands"))
        || directory_has_entries(&cwd.join(".claude/agents"));

    ProjectInitialization {
        initialized,
        hint: (!initialized).then(|| {
            "No project guidance, custom agents, skills, commands, or Forge config found. Add AGENTS.md or run /init."
                .to_string()
        }),
    }
}

/// Whether this project has already attempted automatic setup. Stored as a Forge-owned marker so
/// an unsuccessful model turn cannot consume quota again on every new session.
pub fn project_auto_setup_attempted(cwd: &Path) -> bool {
    cwd.join(".forge/.auto-setup-attempted").is_file()
}

/// Record an automatic setup attempt before its model turn starts. The marker never overwrites
/// user files and is only used to prevent repeated opt-in attempts for the same project.
pub fn mark_project_auto_setup_attempted(cwd: &Path) -> std::io::Result<()> {
    let dir = cwd.join(".forge");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(".auto-setup-attempted"), "auto setup attempted\n")
}

fn directory_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}

/// Canonical project-initialization result for a session working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInitialization {
    pub initialized: bool,
    pub hint: Option<String>,
}
