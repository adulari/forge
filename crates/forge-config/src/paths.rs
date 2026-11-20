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

/// Where the daemon keeps a JSON state file (`pr-watch.json`, `drains.json`). Follows the store
/// like the workspace state and install notices do: `<FORGE_DB>.daemon/<name>` when a scratch or
/// operator-selected store is in use, so an isolated run never reads or writes the owner's real
/// files; otherwise `<data dir>/<name>`.
pub fn daemon_state_file(name: &str) -> Option<PathBuf> {
    daemon_state_file_for(std::env::var_os("FORGE_DB"), data_dir(), name)
}

fn daemon_state_file_for(
    forge_db: Option<std::ffi::OsString>,
    data: Option<PathBuf>,
    name: &str,
) -> Option<PathBuf> {
    match forge_db.filter(|v| !v.is_empty()) {
        Some(db) => {
            let mut beside = db;
            beside.push(".daemon");
            Some(PathBuf::from(beside).join(name))
        }
        None => data.map(|d| d.join(name)),
    }
}

#[cfg(test)]
mod daemon_state_tests {
    use super::*;

    #[test]
    fn an_isolated_store_gets_its_own_daemon_state() {
        let data = Some(PathBuf::from("/real/forge"));
        assert_eq!(
            daemon_state_file_for(
                Some("/scratch/run.db".into()),
                data.clone(),
                "pr-watch.json"
            ),
            Some(PathBuf::from("/scratch/run.db.daemon/pr-watch.json"))
        );
        assert_eq!(
            daemon_state_file_for(None, data.clone(), "pr-watch.json"),
            Some(PathBuf::from("/real/forge/pr-watch.json"))
        );
        assert_eq!(
            daemon_state_file_for(Some("".into()), data, "drains.json"),
            Some(PathBuf::from("/real/forge/drains.json"))
        );
    }
}

/// Whether `path` sits under the directory that holds `[worktree] root = "global"` checkouts.
pub fn is_global_worktree_path(path: &Path) -> bool {
    data_dir().is_some_and(|d| path.starts_with(d.join("worktrees")))
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
/// working directory so worktrees and multi-project servers are evaluated correctly. A
/// subdirectory of a set-up repository counts as set up: its parents up to the repository top
/// ([`crate::project_resource_dirs`]) are checked too, so `repo/pkg/` is not offered `/init` (or
/// an automatic setup turn) when `repo/AGENTS.md` exists.
pub fn project_initialization(cwd: &Path) -> ProjectInitialization {
    let initialized = crate::project_resource_dirs(cwd)
        .iter()
        .any(|dir| directory_is_set_up(dir));

    ProjectInitialization {
        initialized,
        hint: (!initialized).then(|| {
            "No project guidance, custom agents, skills, commands, or Forge config found. Add AGENTS.md or run /init."
                .to_string()
        }),
    }
}

/// [`project_initialization`], re-checked at most every [`ProjectInitCache::EVERY`] per directory.
/// The remote snapshot loops build a frame every tick while a turn runs; each check stats about a
/// dozen files per ancestor and re-reads the trust store, which is wasted work at 30+ Hz.
#[derive(Debug, Default)]
pub struct ProjectInitCache {
    last: Option<(PathBuf, std::time::Instant, ProjectInitialization)>,
}

impl ProjectInitCache {
    pub const EVERY: std::time::Duration = std::time::Duration::from_secs(2);

    pub fn get(&mut self, cwd: &Path) -> ProjectInitialization {
        if let Some((dir, at, value)) = &self.last {
            if dir == cwd && at.elapsed() < Self::EVERY {
                return value.clone();
            }
        }
        let value = project_initialization(cwd);
        self.last = Some((cwd.to_path_buf(), std::time::Instant::now(), value.clone()));
        value
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

fn directory_is_set_up(dir: &Path) -> bool {
    let forge = dir.join(".forge");
    [
        dir.join("AGENTS.md"),
        dir.join("FORGE.md"),
        dir.join("CLAUDE.md"),
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
        || directory_has_entries(&dir.join(".claude/agents"))
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
