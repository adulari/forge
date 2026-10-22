//! Instruction files that apply to part of the workspace: `CLAUDE.md`/`AGENTS.md` in a directory
//! below the root, and rules with `paths:` globs. They reach the model attached to the result of
//! the first read or edit that touches a file they cover, as in Claude Code, so the pinned block
//! (and the prompt cache behind it) does not change mid-session. A compaction or a rewind may drop
//! that result, so both forget what was delivered and the next touch delivers it again.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::collect::Collector;
use super::{dirs_below, render, InstructionFile, Loaded, ScopedRule};
use crate::Session;

/// Dependency and build directories: a package's own `CLAUDE.md` is not this project's.
const FOREIGN_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "vendor",
    "third_party",
    ".venv",
    "venv",
    "site-packages",
    "target",
];

/// What has reached the model since the session started, or since the last compaction or rewind.
#[derive(Debug, Default)]
pub(crate) struct ScopeState {
    /// Directories below the workspace root already checked.
    dirs: HashSet<PathBuf>,
    /// Files delivered (canonical), so a later directory's import of one is not repeated.
    files: HashSet<PathBuf>,
    /// Scoped rules found in those directories.
    rules: Vec<ScopedRule>,
    /// Scoped rules delivered, by path.
    delivered_rules: HashSet<PathBuf>,
}

/// The files a successful tool call touched, for the tools that read or write one.
pub(crate) fn touched_paths(
    tool: &str,
    args: &serde_json::Value,
    written: Option<&Path>,
) -> Vec<String> {
    if tool == "read_file" {
        return crate::recent_files::read_paths(args);
    }
    written
        .and_then(Path::to_str)
        .map(str::to_string)
        .into_iter()
        .collect()
}

impl Session {
    /// Replace the instruction files the pinned block carries. Whether a bridged claude already
    /// reads an `AGENTS.md`-style file through a `CLAUDE.md` copy of it is settled here, once.
    pub(crate) fn set_instructions(&mut self, mut loaded: Loaded) {
        for file in loaded.files.iter_mut().filter(|file| !file.claude_loads) {
            let mut owner = file.path.parent().unwrap_or(&file.path);
            if owner.ends_with(".forge") {
                owner = owner.parent().unwrap_or(owner);
            }
            file.claude_loads = crate::lean_context::claude_md_covers(owner, &file.body);
        }
        self.instructions = loaded;
    }

    /// `result` with the instruction files that apply to the files this call touched and have
    /// not reached the model yet appended.
    pub(crate) fn with_scoped_instructions(
        &mut self,
        paths: &[String],
        mut result: String,
    ) -> String {
        if let Some(note) = self.scoped_instructions_for(paths) {
            result.push_str("\n\n");
            result.push_str(&note);
        }
        result
    }

    fn scoped_instructions_for(&mut self, paths: &[String]) -> Option<String> {
        let root = self.workspace.root().to_path_buf();
        let mut found: Vec<InstructionFile> = Vec::new();
        for path in paths.iter().filter(|p| !p.trim().is_empty()) {
            let path = Path::new(path);
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            let Ok(relative) = absolute.strip_prefix(&root) else {
                continue;
            };
            if relative
                .components()
                .any(|part| FOREIGN_DIRS.iter().any(|dir| part.as_os_str() == *dir))
            {
                continue;
            }
            for dir in dirs_below(&root, &absolute) {
                if self.instruction_scope.dirs.insert(dir.clone()) {
                    found.extend(self.load_scoped_dir(&root, &dir));
                }
            }
            let state = &mut self.instruction_scope;
            for rule in self.instructions.scoped_rules.iter().chain(&state.rules) {
                if rule.matches(&absolute, &root) && state.delivered_rules.insert(rule.path.clone())
                {
                    found.push(rule.to_file());
                }
            }
        }
        let body = render(&found, &root)?;
        let names: Vec<String> = found
            .iter()
            .map(|file| super::display(&file.path, &root))
            .collect();
        Some(format!(
            "[instructions] {} {} to the file just used (loaded on first use):\n\n{body}",
            names.join(", "),
            if names.len() == 1 { "applies" } else { "apply" }
        ))
    }

    /// The not-yet-delivered instruction files of one directory below the root.
    fn load_scoped_dir(&mut self, root: &Path, dir: &Path) -> Vec<InstructionFile> {
        let known = self
            .instructions
            .files
            .iter()
            .map(|file| file.path.clone())
            .chain(
                self.instructions
                    .scoped_rules
                    .iter()
                    .map(|rule| rule.path.clone()),
            )
            .chain(self.instruction_scope.files.iter().cloned())
            .chain(
                self.instruction_scope
                    .rules
                    .iter()
                    .map(|rule| rule.path.clone()),
            );
        let mut collector = Collector::new(root, known.collect::<Vec<_>>());
        collector.dir(dir);
        let loaded = collector.finish();
        self.instruction_scope
            .files
            .extend(loaded.files.iter().map(|file| file.path.clone()));
        self.instruction_scope.rules.extend(loaded.scoped_rules);
        loaded.files
    }
}
