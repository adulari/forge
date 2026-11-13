//! Deterministic hunk risk for the review queue: `routine` hunks (lockfile churn, generated files,
//! snapshots, whitespace-only edits, import reorders) can be accepted in bulk; everything else is
//! `look`, with a one-line reason. Computed where the diff is parsed so every surface agrees.
//!
//! The rules are conservative on purpose: a hunk is routine only when a rule proves it, and a
//! routine hunk outside a lockfile, generated file or snapshot never removes a non-blank line that
//! it does not put back.

use super::{GitDiffFile, GitDiffHunk};

pub(crate) const ROUTINE: &str = "routine";
pub(crate) const LOOK: &str = "look";

const LOCKFILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
    "Podfile.lock",
    "pubspec.lock",
];

/// Tokens of a path (split on anything that is not a letter or digit) that mark permission code.
const SENSITIVE_TOKENS: &[&str] = &[
    "auth",
    "authn",
    "authz",
    "authentication",
    "authorization",
    "oauth",
    "permission",
    "permissions",
    "acl",
    "rbac",
    "sandbox",
    "secret",
    "secrets",
    "credential",
    "credentials",
    "policy",
    "policies",
];

const TEST_LINE_MARKERS: &[&str] = &[
    "#[test]",
    "#[tokio::test]",
    "fn test_",
    "def test_",
    "it(",
    "it.each",
    "test(",
    "test.each",
    "describe(",
    "assert",
    "expect(",
    "@test",
    "func test",
];

const ERROR_MARKERS: &[&str] = &[
    "?;",
    ")?",
    "catch",
    "except",
    "rescue",
    "map_err",
    ".context(",
    "err != nil",
    "Err(",
    "throw ",
    "raise ",
    "try {",
    "try:",
];

/// Classify every hunk of `file` in place.
pub(super) fn classify_file(file: &mut GitDiffFile) {
    let path = file.path.clone();
    let kind = file.kind.clone();
    let truncated = file.skipped_lines > 0;
    let generated = is_generated_path(&path) || has_generated_marker(&file.hunks);
    let exempt = if is_lockfile(&path) {
        Some("lockfile churn")
    } else if is_snapshot(&path) {
        Some("snapshot update")
    } else if generated {
        Some("generated file")
    } else {
        None
    };
    if file.hunks.is_empty() {
        return;
    }
    for hunk in &mut file.hunks {
        let (risk, reason) = match exempt {
            Some(_) if kind == "deleted" => (LOOK, format!("deletes {}", exempt_noun(&path))),
            Some(reason) => (ROUTINE, reason.to_string()),
            None if truncated => (LOOK, "diff too large to check".to_string()),
            None if file.binary => (LOOK, "binary file".to_string()),
            None => classify_hunk(&path, &kind, hunk),
        };
        hunk.risk = risk;
        hunk.reason = reason;
    }
}

fn exempt_noun(path: &str) -> &'static str {
    if is_lockfile(path) {
        "a lockfile"
    } else if is_snapshot(path) {
        "a snapshot"
    } else {
        "a generated file"
    }
}

/// The verdict for one hunk of a hand-written file.
fn classify_hunk(path: &str, kind: &str, hunk: &GitDiffHunk) -> (&'static str, String) {
    let added: Vec<&str> = hunk
        .lines
        .iter()
        .filter_map(|l| l.strip_prefix('+'))
        .collect();
    let removed: Vec<&str> = hunk
        .lines
        .iter()
        .filter_map(|l| l.strip_prefix('-'))
        .collect();
    let removed_code = removed.iter().filter(|l| !l.trim().is_empty()).count();

    if is_sensitive_path(path) {
        return (LOOK, "touches permission code".to_string());
    }
    if is_migration(path) {
        let reason = if kind == "created" {
            "new migration"
        } else {
            "edits a migration"
        };
        return (LOOK, reason.to_string());
    }
    if is_test_path(path)
        && (kind == "deleted"
            || removed
                .iter()
                .any(|l| TEST_LINE_MARKERS.iter().any(|m| l.contains(m))))
    {
        return (LOOK, "deletes a test".to_string());
    }
    if added
        .iter()
        .chain(removed.iter())
        .any(|l| has_word(l, "unsafe"))
    {
        return (LOOK, "touches unsafe code".to_string());
    }
    let count = |lines: &[&str]| {
        lines
            .iter()
            .filter(|l| ERROR_MARKERS.iter().any(|m| l.contains(m)))
            .count()
    };
    if count(&removed) > count(&added) {
        return (LOOK, "removes error handling".to_string());
    }
    if kind == "deleted" {
        return (LOOK, "deletes a file".to_string());
    }
    if kind == "renamed" {
        return (LOOK, "renamed file".to_string());
    }
    if whitespace_only(path, &hunk.lines) {
        return (ROUTINE, "whitespace only".to_string());
    }
    if import_reorder(path, &hunk.lines) {
        return (ROUTINE, "import reorder".to_string());
    }
    let reason = if kind == "created" {
        "new file".to_string()
    } else if removed_code > 0 {
        format!("removes {removed_code} line{}", plural(removed_code))
    } else {
        "adds code".to_string()
    };
    (LOOK, reason)
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn extension(path: &str) -> String {
    file_name(path)
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

fn tokens(path: &str) -> impl Iterator<Item = String> + '_ {
    path.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase)
}

fn is_lockfile(path: &str) -> bool {
    LOCKFILES.contains(&file_name(path))
}

fn is_snapshot(path: &str) -> bool {
    let ext = extension(path);
    ext == "snap" || path.contains("__snapshots__/") || file_name(path).ends_with(".snap.new")
}

fn is_generated_path(path: &str) -> bool {
    let name = file_name(path);
    name.contains(".min.")
        || name.ends_with(".pb.go")
        || name.ends_with("_pb2.py")
        || name.ends_with("_pb2_grpc.py")
        || name.ends_with(".generated.ts")
        || name.ends_with(".generated.rs")
        || name.ends_with(".g.dart")
        || path
            .split('/')
            .any(|seg| seg == "generated" || seg == "__generated__")
}

/// A leading comment saying the file is machine-written (`@generated`, `DO NOT EDIT`, ...).
fn has_generated_marker(hunks: &[GitDiffHunk]) -> bool {
    let Some(first) = hunks.first() else {
        return false;
    };
    first
        .lines
        .iter()
        .take(12)
        .filter_map(|l| l.get(1..))
        .map(str::trim)
        .filter(|l| {
            ["//", "#", "/*", "*", "--", "<!--"]
                .iter()
                .any(|c| l.starts_with(c))
        })
        .any(|l| {
            let lower = l.to_ascii_lowercase();
            lower.contains("@generated")
                || lower.contains("do not edit")
                || lower.contains("code generated")
                || lower.contains("auto-generated")
                || lower.contains("autogenerated")
        })
}

fn is_sensitive_path(path: &str) -> bool {
    tokens(path).any(|t| SENSITIVE_TOKENS.contains(&t.as_str()))
}

fn is_migration(path: &str) -> bool {
    tokens(path).any(|t| t == "migration" || t == "migrations")
}

fn is_test_path(path: &str) -> bool {
    let name = file_name(path);
    path.split('/')
        .any(|seg| matches!(seg, "test" | "tests" | "__tests__" | "spec" | "specs"))
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
        || name.starts_with("test_")
        || name.ends_with("_tests.rs")
        || name == "tests.rs"
}

fn has_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + word.len()..].chars().next();
        let boundary = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        boundary(before) && boundary(after)
    })
}

/// Files where indentation and trailing spaces carry meaning: only blank-line edits are routine.
fn whitespace_sensitive(path: &str) -> bool {
    let name = file_name(path);
    name == "Makefile"
        || matches!(
            extension(path).as_str(),
            "py" | "pyi"
                | "yml"
                | "yaml"
                | "mk"
                | "hs"
                | "coffee"
                | "nim"
                | "pug"
                | "haml"
                | "slim"
                | "md"
                | "markdown"
                | "rst"
                | "txt"
        )
}

/// Each contiguous run of changed lines as `(removed, added)`. Compared run by run so a line
/// moved across unchanged lines is never mistaken for a formatting edit.
fn change_blocks(lines: &[String]) -> Vec<(Vec<&str>, Vec<&str>)> {
    let mut blocks = Vec::new();
    let mut open = false;
    for line in lines {
        match line.as_bytes().first() {
            Some(b'-' | b'+') => {
                if !open {
                    blocks.push((Vec::new(), Vec::new()));
                    open = true;
                }
                let block = blocks.last_mut().expect("block was just opened");
                if let Some(rest) = line.strip_prefix('-') {
                    block.0.push(rest);
                } else {
                    block.1.push(&line[1..]);
                }
            }
            Some(b'\\') => {}
            _ => open = false,
        }
    }
    blocks
}

/// In every changed run the same lines come back in the same order once surrounding whitespace
/// is ignored, or only blank lines were added or removed.
fn whitespace_only(path: &str, lines: &[String]) -> bool {
    let code = |lines: &[&str]| -> Vec<String> {
        lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                if whitespace_sensitive(path) {
                    (*l).to_string()
                } else {
                    l.trim().to_string()
                }
            })
            .collect()
    };
    change_blocks(lines)
        .iter()
        .all(|(removed, added)| code(added) == code(removed))
}

fn import_line(path: &str, line: &str) -> Option<String> {
    let t = line.trim();
    let ok = match extension(path).as_str() {
        "rs" => {
            let body = t
                .strip_prefix("pub(crate) use ")
                .or_else(|| t.strip_prefix("pub use "))
                .or_else(|| t.strip_prefix("use "));
            body.is_some() && t.ends_with(';') && !t.contains("//")
        }
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
            t.starts_with("import ")
                && t.contains(" from ")
                && (t.ends_with(';') || t.ends_with('\'') || t.ends_with('"'))
                && !t.contains("//")
        }
        "py" => {
            !t.contains('#')
                && !t.contains('(')
                && !t.contains('\\')
                && (t.starts_with("import ") || (t.starts_with("from ") && t.contains(" import ")))
        }
        _ => false,
    };
    ok.then(|| t.to_string())
}

/// Every changed line is an import, the removed set is the added set reordered, and nothing but
/// imports and blank lines sits between the first and the last change.
fn import_reorder(path: &str, lines: &[String]) -> bool {
    fn body(l: &str) -> &str {
        l.get(1..).unwrap_or("")
    }
    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
        .map(|(i, _)| i)
        .collect();
    let (Some(&first), Some(&last)) = (changed.first(), changed.last()) else {
        return false;
    };
    let all_imports = lines[first..=last]
        .iter()
        .all(|l| body(l).trim().is_empty() || import_line(path, body(l)).is_some());
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for line in &lines[first..=last] {
        let Some(import) = import_line(path, body(line)) else {
            continue;
        };
        if line.starts_with('+') {
            added.push(import);
        } else if line.starts_with('-') {
            removed.push(import);
        }
    }
    added.sort();
    removed.sort();
    all_imports && !added.is_empty() && added == removed
}

#[cfg(test)]
mod tests;
