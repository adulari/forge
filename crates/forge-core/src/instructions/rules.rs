//! `.claude/rules/` and `.forge/rules/`: every `*.md` below them is a rule. A rule without
//! `paths:` frontmatter applies everywhere and loads with the other files; one with `paths:` globs
//! waits until a read or edit touches a matching file (`scoped.rs`).

use std::path::{Path, PathBuf};

use super::collect::{canonical, strip_html_comments, Collector};
use super::InstructionFile;

/// How deep rule directories are walked.
const MAX_RULE_DEPTH: usize = 6;

#[derive(Debug, Clone)]
pub(crate) struct ScopedRule {
    pub(crate) path: PathBuf,
    body: String,
    patterns: Vec<String>,
    globs: globset::GlobSet,
    /// The directory holding the rules folder: globs match paths relative to it, as well as
    /// relative to the workspace root.
    owner: Option<PathBuf>,
    pub(crate) claude_loads: bool,
}

impl ScopedRule {
    pub(crate) fn matches(&self, file: &Path, root: &Path) -> bool {
        [Some(root), self.owner.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(|base| file.strip_prefix(base).ok())
            .any(|relative| self.globs.is_match(relative))
    }

    pub(crate) fn to_file(&self) -> InstructionFile {
        InstructionFile {
            path: self.path.clone(),
            body: format!(
                "(applies to files matching {})\n{}",
                self.patterns.join(", "),
                self.body
            ),
            origin: super::Origin::Rule,
            via: None,
            claude_loads: self.claude_loads,
        }
    }
}

impl Collector {
    pub(in crate::instructions) fn rules(
        &mut self,
        dir: &Path,
        owner: Option<&Path>,
        claude_loads: bool,
        base: Option<&Path>,
    ) {
        self.watched.push(dir.to_path_buf());
        if !dir.is_dir() {
            return;
        }
        let mut found = Vec::new();
        walk(dir, 0, &mut found, &mut self.watched);
        found.sort();
        for path in found {
            self.watched.push(path.clone());
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let canonical = canonical(&path);
            if self.already_seen(&canonical, claude_loads) {
                continue;
            }
            let (front, body) = forge_skills::frontmatter::split(&raw);
            let patterns: Vec<String> = front
                .and_then(|front| forge_skills::frontmatter::parse(front).ok())
                .map(|front| front.list("paths"))
                .unwrap_or_default()
                .iter()
                .flat_map(|entry| split_patterns(entry))
                .collect();
            if patterns.is_empty() {
                self.push_rule(&path, body, claude_loads, base);
                continue;
            }
            self.mark_seen(canonical.clone());
            if let Some(globs) = compile(&patterns) {
                self.scoped_rules.push(ScopedRule {
                    path: canonical,
                    body: strip_html_comments(body).trim().to_string(),
                    patterns,
                    globs,
                    owner: owner.map(Path::to_path_buf),
                    claude_loads,
                });
            }
        }
    }
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>, watched: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < MAX_RULE_DEPTH {
                watched.push(path.clone());
                walk(&path, depth + 1, found, watched);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
}

/// `src/**/*.ts, lib/**` → two patterns; a comma inside `{ts,tsx}` stays.
fn split_patterns(entry: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut braces = 0usize;
    for c in entry.chars() {
        match c {
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            ',' if braces == 0 => {
                out.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    out.push(current);
    out.into_iter()
        .map(|p| p.trim().trim_matches(['"', '\'']).to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// The valid globs among `patterns`; `None` when none is.
fn compile(patterns: &[String]) -> Option<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        if let Ok(glob) = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
        {
            builder.add(glob);
            any = true;
        }
    }
    any.then(|| builder.build().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_split_on_commas_outside_braces() {
        assert_eq!(
            split_patterns("src/**/*.{ts,tsx}, \"lib/**\""),
            ["src/**/*.{ts,tsx}", "lib/**"]
        );
    }

    #[test]
    fn a_double_star_spans_directories_and_a_single_star_does_not() {
        let globs = compile(&["src/**/*.rs".into(), "*.toml".into()]).unwrap();
        assert!(globs.is_match("src/a/b/c.rs"));
        assert!(globs.is_match("src/c.rs"));
        assert!(globs.is_match("Cargo.toml"));
        assert!(!globs.is_match("crates/x/Cargo.toml"));
        assert!(compile(&["[".into()]).is_none());
    }
}
