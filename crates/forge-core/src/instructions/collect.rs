//! Reading instruction files: de-duplication, `@path` imports and HTML comments.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::{InstructionFile, Loaded, Origin, ScopedRule, DIR_FILES, MAX_IMPORT_DEPTH, RULE_DIRS};

pub(super) struct Collector {
    root: PathBuf,
    files: Vec<InstructionFile>,
    /// Canonical path → the file that carries its text (`None`: nothing to carry, or a file loaded
    /// earlier in the session that this collector does not hold).
    seen: HashMap<PathBuf, Option<usize>>,
    pub(super) scoped_rules: Vec<ScopedRule>,
    pub(super) watched: Vec<PathBuf>,
}

/// Where a file's text is pushed from.
struct Source<'a> {
    path: &'a Path,
    origin: Origin,
    claude_loads: bool,
    /// Imports must stay inside this directory (`None`: the user's own files, unbounded).
    base: Option<&'a Path>,
    via: Option<&'a Path>,
    depth: usize,
}

impl Collector {
    /// `known`: canonical paths already carried, which this collector skips.
    pub(super) fn new(root: &Path, known: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            root: root.to_path_buf(),
            files: Vec::new(),
            seen: known.into_iter().map(|path| (path, None)).collect(),
            scoped_rules: Vec::new(),
            watched: Vec::new(),
        }
    }

    pub(super) fn finish(self) -> Loaded {
        Loaded {
            files: self.files,
            scoped_rules: self.scoped_rules,
            watched: self.watched,
        }
    }

    /// The files of one project directory, then its rule directories.
    pub(super) fn dir(&mut self, dir: &Path) {
        let base = forge_config::repo_top(dir).unwrap_or_else(|| dir.to_path_buf());
        for (name, origin, claude_loads) in DIR_FILES {
            self.file(&dir.join(name), *origin, *claude_loads, Some(&base));
        }
        for (name, claude_loads) in RULE_DIRS {
            self.rules(&dir.join(name), Some(dir), *claude_loads, Some(&base));
        }
    }

    pub(super) fn file(
        &mut self,
        path: &Path,
        origin: Origin,
        claude_loads: bool,
        base: Option<&Path>,
    ) {
        self.watched.push(path.to_path_buf());
        if let Ok(raw) = std::fs::read_to_string(path) {
            self.push(
                &raw,
                Source {
                    path,
                    origin,
                    claude_loads,
                    base,
                    via: None,
                    depth: 0,
                },
            );
        }
    }

    /// Whether `path` was met before; a claude-read route to it marks its carrier claude-read.
    pub(super) fn already_seen(&mut self, canonical: &Path, claude_loads: bool) -> bool {
        let Some(slot) = self.seen.get(canonical) else {
            return false;
        };
        if let (Some(index), true) = (*slot, claude_loads) {
            self.files[index].claude_loads = true;
        }
        true
    }

    pub(super) fn mark_seen(&mut self, canonical: PathBuf) {
        self.seen.insert(canonical, None);
    }

    pub(super) fn push_rule(
        &mut self,
        path: &Path,
        raw: &str,
        claude_loads: bool,
        base: Option<&Path>,
    ) {
        self.push(
            raw,
            Source {
                path,
                origin: Origin::Rule,
                claude_loads,
                base,
                via: None,
                depth: 0,
            },
        );
    }

    fn push(&mut self, raw: &str, source: Source<'_>) {
        let canonical = canonical(source.path);
        if self.already_seen(&canonical, source.claude_loads) {
            return;
        }
        let dir = canonical.parent().unwrap_or(&self.root).to_path_buf();
        let (body, imports) = self.scan(&strip_html_comments(raw), &dir, source.base);
        let carrier = if body.trim().is_empty() {
            None
        } else if let Some(index) = self
            .files
            .iter()
            .position(|file| file.body.trim() == body.trim())
        {
            self.files[index].claude_loads |= source.claude_loads;
            Some(index)
        } else {
            self.files.push(InstructionFile {
                path: canonical.clone(),
                body,
                origin: source.origin,
                via: source.via.map(Path::to_path_buf),
                claude_loads: source.claude_loads,
            });
            Some(self.files.len() - 1)
        };
        self.seen.insert(canonical.clone(), carrier);
        if source.depth >= MAX_IMPORT_DEPTH {
            return;
        }
        for target in imports {
            self.watched.push(target.clone());
            if let Ok(raw) = std::fs::read_to_string(&target) {
                self.push(
                    &raw,
                    Source {
                        path: &target,
                        origin: source.origin,
                        claude_loads: source.claude_loads,
                        base: source.base,
                        via: Some(&canonical),
                        depth: source.depth + 1,
                    },
                );
            }
        }
    }

    /// The text to carry and the files it imports. A line made only of imports is dropped from
    /// the text; `@` inside code (fenced or backticked) is not an import.
    fn scan(&self, text: &str, dir: &Path, base: Option<&Path>) -> (String, Vec<PathBuf>) {
        let mut kept = Vec::new();
        let mut imports = Vec::new();
        let mut fenced = false;
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                kept.push(line);
                continue;
            }
            if fenced {
                kept.push(line);
                continue;
            }
            let outside_code: String = line.split('`').step_by(2).collect::<Vec<_>>().join(" ");
            let words: Vec<&str> = outside_code.split_whitespace().collect();
            let mut only_imports = !words.is_empty();
            for word in &words {
                match word
                    .strip_prefix('@')
                    .and_then(|raw| self.resolve(raw, dir, base))
                {
                    Some(target) => imports.push(target),
                    None => only_imports = false,
                }
            }
            if !only_imports {
                kept.push(line);
            }
        }
        (kept.join("\n"), imports)
    }

    /// The file an `@raw` token names, when it exists and may be imported from `dir`.
    fn resolve(&self, raw: &str, dir: &Path, base: Option<&Path>) -> Option<PathBuf> {
        let candidates = [
            raw,
            raw.trim_end_matches(['.', ',', ';', ':', ')', '!', '?', '"', '\'']),
        ];
        let target = candidates.iter().find_map(|raw| {
            if raw.is_empty() || raw.contains('@') {
                return None;
            }
            let path = match raw.strip_prefix("~/") {
                Some(rest) => forge_config::home_dir()?.join(rest),
                None => dir.join(raw),
            };
            path.is_file().then(|| canonical(&path))
        })?;
        if base.is_some_and(|base| !target.starts_with(base)) {
            return None;
        }
        (!secret_paths().is_match(&target)).then_some(target)
    }
}

/// The paths the built-in rules keep `read_file` away from (`.env`, keys, `~/.ssh/`, …), compiled
/// once: building them per import cost milliseconds per file.
fn secret_paths() -> &'static globset::GlobSet {
    static SECRETS: OnceLock<globset::GlobSet> = OnceLock::new();
    SECRETS.get_or_init(|| {
        let mut builder = globset::GlobSetBuilder::new();
        for rule in forge_config::builtin_deny_rules()
            .iter()
            .filter(|rule| rule.tool == "read_file")
        {
            for pattern in &rule.patterns {
                if let Ok(glob) = globset::Glob::new(pattern) {
                    builder.add(glob);
                }
            }
        }
        builder
            .build()
            .unwrap_or_else(|_| globset::GlobSet::empty())
    })
}

pub(super) fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Drop `<!-- … -->` comments (they may span lines) outside fenced code. A line that held only a
/// comment goes with it.
pub(super) fn strip_html_comments(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_comment = false;
    let mut fenced = false;
    for line in text.lines() {
        if !in_comment {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
            }
            if fenced || !line.contains("<!--") {
                out.push(line.to_string());
                continue;
            }
        }
        let mut kept = String::new();
        let mut rest = line;
        loop {
            if in_comment {
                match rest.find("-->") {
                    Some(end) => {
                        rest = &rest[end + 3..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else {
                match rest.find("<!--") {
                    Some(start) => {
                        kept.push_str(&rest[..start]);
                        rest = &rest[start + 4..];
                        in_comment = true;
                    }
                    None => {
                        kept.push_str(rest);
                        break;
                    }
                }
            }
        }
        if !kept.trim().is_empty() {
            out.push(kept.trim_end().to_string());
        }
    }
    out.join("\n")
}
