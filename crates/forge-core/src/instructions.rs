//! Standing instruction files (docs/features/instruction-files.md).
//!
//! Forge used to inject the first non-empty file of a fixed list, so a project `AGENTS.md` hid the
//! user's global `CLAUDE.md`, parent directories were never read, and Claude Code's
//! `CLAUDE.local.md`, `@path` imports, `.claude/rules/` and nested `CLAUDE.md` files did not exist.
//! Every file now applies, lowest precedence first:
//!
//! 1. user-global: `<config>/forge/AGENTS.md`, `~/.forge/AGENTS.md`, Claude's `CLAUDE.md`
//!    (`$CLAUDE_CONFIG_DIR` or `~/.claude`), then the unconditional rules in `<config>/forge/rules/`
//!    and Claude's `rules/`;
//! 2. each directory from the filesystem root down to the workspace: `.forge/AGENTS.md`,
//!    `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`, then its unconditional
//!    `.claude/rules/` and `.forge/rules/` files. A parent anyone on the machine can write to
//!    (`/tmp`) is skipped: a file any local user can create is not an instruction file;
//! 3. after a read or edit (`scoped.rs`): the same files in directories below the workspace root,
//!    and rules whose `paths:` globs match the file, attached to that tool result.
//!
//! `@path` imports load the named file after the one importing it, up to five levels deep. A
//! project file imports only from inside its repository (or its own directory outside one), and
//! never a file the built-in secret rules keep `read_file` away from. HTML comments are dropped.
//! A file reached twice (symlink, import, identical copy) is carried once.
//!
//! A bridged `claude` reads the `CLAUDE.md` family, `.claude/rules/` and their imports itself, so
//! each file records whether it does ([`InstructionFile::claude_loads`]) and the claude bridge is
//! sent only the rest; `lean_context::claude_md_covers` still catches an `AGENTS.md` that a
//! `CLAUDE.md` carries by copy.

mod collect;
mod rules;
mod scoped;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(crate) use rules::ScopedRule;
pub(crate) use scoped::{touched_paths, ScopeState};

/// How deep `@path` imports nest (Claude Code's limit).
const MAX_IMPORT_DEPTH: usize = 5;

/// The instruction files one directory can hold, in the order they apply, and whether a bridged
/// claude reads each itself.
const DIR_FILES: &[(&str, Origin, bool)] = &[
    (".forge/AGENTS.md", Origin::Project, false),
    ("AGENTS.md", Origin::Project, false),
    ("CLAUDE.md", Origin::Project, true),
    (".claude/CLAUDE.md", Origin::Project, true),
    ("CLAUDE.local.md", Origin::Local, true),
];

/// Rule directories one directory can hold, and whether a bridged claude reads them itself.
const RULE_DIRS: &[(&str, bool)] = &[(".claude/rules", true), (".forge/rules", false)];

/// Where a file came from, which is what the model is told about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    User,
    Project,
    Local,
    Rule,
}

#[derive(Debug, Clone)]
pub(crate) struct InstructionFile {
    pub(crate) path: PathBuf,
    pub(crate) body: String,
    pub(crate) origin: Origin,
    /// The file whose `@path` import brought this one in.
    pub(crate) via: Option<PathBuf>,
    /// A bridged `claude` reads this file itself, so the claude bridge is not sent it.
    pub(crate) claude_loads: bool,
}

impl InstructionFile {
    fn label(&self, root: &Path) -> String {
        if let Some(via) = &self.via {
            return format!("imported by {}", display(via, root));
        }
        match self.origin {
            Origin::User => "user's instructions for every project",
            Origin::Project => "project instructions",
            Origin::Local => "personal project instructions, not checked in",
            Origin::Rule => "rule",
        }
        .to_string()
    }
}

/// What one load found: the files the pinned block carries, the rules waiting for a matching file,
/// and every path it looked at.
#[derive(Debug, Clone, Default)]
pub(crate) struct Loaded {
    pub(crate) files: Vec<InstructionFile>,
    pub(crate) scoped_rules: Vec<ScopedRule>,
    /// Present or not: creating, editing or deleting any of them reloads.
    pub(crate) watched: Vec<PathBuf>,
}

/// The (modified time, length) of each watched path, `None` where nothing exists.
pub(crate) type Fingerprint = Vec<Option<(SystemTime, u64)>>;

pub(crate) fn fingerprint(paths: &[PathBuf]) -> Fingerprint {
    paths
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .ok()
                .map(|meta| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len()))
        })
        .collect()
}

/// Where a load looks besides the workspace.
#[derive(Debug, Clone, Default)]
pub(crate) struct Locations {
    /// User-global files, lowest precedence first, with whether a bridged claude reads each.
    pub(crate) global_files: Vec<(PathBuf, bool)>,
    pub(crate) global_rule_dirs: Vec<(PathBuf, bool)>,
    /// Read the workspace and its parents (`--setting-sources` without `project` turns it off).
    pub(crate) project: bool,
    /// The highest directory read; `None` reads up to the filesystem root.
    pub(crate) top: Option<PathBuf>,
}

impl Locations {
    pub(crate) fn for_workspace(root: &Path) -> Self {
        let sources = forge_config::setting_sources::setting_sources();
        let mut locations = Self {
            project: sources.project,
            ..Self::default()
        };
        // Unit tests must not read the developer's real instruction files.
        if cfg!(test) {
            locations.top = Some(root.to_path_buf());
            return locations;
        }
        if !sources.user {
            return locations;
        }
        if let Some(config) = forge_config::config_dir() {
            locations
                .global_files
                .push((config.join("AGENTS.md"), false));
            locations
                .global_rule_dirs
                .push((config.join("rules"), false));
        }
        if let Some(home) = forge_config::home_dir() {
            locations
                .global_files
                .push((home.join(".forge/AGENTS.md"), false));
        }
        if let Some(claude) = claude_config_dir() {
            locations
                .global_files
                .push((claude.join("CLAUDE.md"), true));
            locations
                .global_rule_dirs
                .push((claude.join("rules"), true));
        }
        locations
    }
}

/// The directory Claude Code reads its user memory from.
fn claude_config_dir() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => forge_config::home_dir().map(|home| home.join(".claude")),
    }
}

/// Every instruction file for a session rooted at `root`.
pub(crate) fn load(root: &Path) -> Loaded {
    load_with(root, &Locations::for_workspace(root))
}

pub(crate) fn load_with(root: &Path, locations: &Locations) -> Loaded {
    let mut collector = collect::Collector::new(root, std::iter::empty());
    for (path, claude_loads) in &locations.global_files {
        collector.file(path, Origin::User, *claude_loads, None);
    }
    for (dir, claude_loads) in &locations.global_rule_dirs {
        collector.rules(dir, None, *claude_loads, None);
    }
    if locations.project {
        for dir in workspace_and_parents(root, locations.top.as_deref()) {
            collector.dir(&dir);
        }
    }
    collector.finish()
}

/// `root` and its parents, filesystem root first, up to `top`. A parent anyone can write to is
/// left out; the workspace itself is the person's own choice.
fn workspace_and_parents(root: &Path, top: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in root.ancestors() {
        if dir == root || !world_writable(dir) {
            dirs.push(dir.to_path_buf());
        }
        if Some(dir) == top {
            break;
        }
    }
    dirs.reverse();
    dirs
}

#[cfg(unix)]
fn world_writable(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(dir).is_ok_and(|meta| meta.permissions().mode() & 0o002 != 0)
}

#[cfg(not(unix))]
fn world_writable(_dir: &Path) -> bool {
    false
}

/// The directories strictly below `root` down to the one holding `file`, outermost first.
fn dirs_below(root: &Path, file: &Path) -> Vec<PathBuf> {
    let Some(parent) = file.parent() else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = parent
        .ancestors()
        .take_while(|dir| *dir != root && dir.starts_with(root))
        .map(Path::to_path_buf)
        .collect();
    dirs.reverse();
    dirs
}

/// The files as the model reads them. One plain file reads exactly as it always did; several are
/// each headed with where they came from.
pub(crate) fn render<'a>(
    files: impl IntoIterator<Item = &'a InstructionFile>,
    root: &Path,
) -> Option<String> {
    let files: Vec<&InstructionFile> = files
        .into_iter()
        .filter(|file| !file.body.trim().is_empty())
        .collect();
    match files.as_slice() {
        [] => None,
        [only] if only.via.is_none() => Some(only.body.trim_end().to_string()),
        _ => {
            let mut out = String::from(
                "Instruction files, broadest first; where they disagree, the later, more specific \
                 file wins.",
            );
            for file in files {
                out.push_str(&format!(
                    "\n\nContents of {} ({}):\n\n{}",
                    display(&file.path, root),
                    file.label(root),
                    file.body.trim()
                ));
            }
            Some(out)
        }
    }
}

/// `path` relative to the workspace when inside it, `~/…` under the home directory.
fn display(path: &Path, root: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(root) {
        return relative.display().to_string();
    }
    if let Some(home) = forge_config::home_dir() {
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}
