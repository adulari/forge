//! Conflict resolution for append-only changelog files.
//!
//! A merge conflict in `CHANGELOG.md` is almost always two branches that each added entries at the
//! same place (the top of the same heading). That has one right answer: keep both, the branch's
//! entries first and the base's after. Anything else (one side edited or removed an existing
//! line, a file added on both sides, a deletion) has no mechanical answer and is refused, so the
//! caller hands it to a person or the session.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;

/// A path the resolver is allowed to touch: a changelog file in any directory, so the repo's
/// package copy is covered by the same rule.
pub(crate) fn is_changelog_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    matches!(name.as_str(), "changelog.md" | "changelog")
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Merged(String),
    Refused(String),
}

fn is_marker(line: &str) -> bool {
    ["<<<<<<<", ">>>>>>>", "|||||||"]
        .iter()
        .any(|m| line.starts_with(m) && (line.len() == 7 || line.as_bytes()[7] == b' '))
}

/// Marker lines left in `text` that none of the inputs already had. A markdown `=======`
/// underline in the inputs is not a leftover.
fn leftover_markers(text: &str, inputs: &[&str]) -> bool {
    let has_rule = inputs.iter().any(|i| i.lines().any(|l| l == "======="));
    text.lines()
        .any(|l| is_marker(l) || (l == "=======" && !has_rule))
}

fn deletions(dir: &Path, from: &str, to: &str) -> Result<u64, String> {
    let out = Command::new("git")
        .args(["diff", "--no-index", "--numstat", from, to])
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    // `--no-index` exits 1 when the files differ.
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or_default();
    if first.is_empty() {
        return Ok(0);
    }
    first
        .split('\t')
        .nth(1)
        .and_then(|d| d.parse().ok())
        .ok_or_else(|| format!("unreadable diff stat: {first}"))
}

/// Merge `ours` (the PR branch) and `theirs` (the base) over their common `base`, keeping every
/// line either side added. Refuses unless both sides only added lines.
pub(crate) fn resolve(base: &str, ours: &str, theirs: &str) -> Resolution {
    match resolve_inner(base, ours, theirs) {
        Ok(r) => r,
        Err(e) => Resolution::Refused(e),
    }
}

fn resolve_inner(base: &str, ours: &str, theirs: &str) -> Result<Resolution, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let write = |name: &str, text: &str| -> Result<(), String> {
        let mut f = std::fs::File::create(dir.path().join(name)).map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())
    };
    write("base", base)?;
    write("ours", ours)?;
    write("theirs", theirs)?;
    for (side, name) in [("the branch", "ours"), ("the base branch", "theirs")] {
        if deletions(dir.path(), "base", name)? > 0 {
            return Ok(Resolution::Refused(format!(
                "{side} changed or removed existing changelog lines"
            )));
        }
    }
    let out = Command::new("git")
        .args(["merge-file", "-p", "--union", "ours", "base", "theirs"])
        .current_dir(dir.path())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.code().is_none_or(|c| c > 127) {
        return Err("git merge-file failed".into());
    }
    let merged = String::from_utf8(out.stdout).map_err(|_| "changelog is not UTF-8".to_string())?;
    if leftover_markers(&merged, &[base, ours, theirs]) {
        return Ok(Resolution::Refused(
            "conflict markers remained after the merge".into(),
        ));
    }
    Ok(Resolution::Merged(merged))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "# Changelog\n\n## Unreleased\n\n- old entry\n";

    #[test]
    fn recognises_changelog_files_anywhere() {
        assert!(is_changelog_path("CHANGELOG.md"));
        assert!(is_changelog_path("packages/cli/CHANGELOG.md"));
        assert!(!is_changelog_path("src/changelog_parser.rs"));
        assert!(!is_changelog_path("README.md"));
    }

    #[test]
    fn both_sides_prepending_under_one_heading_keep_both_ours_first() {
        let ours = "# Changelog\n\n## Unreleased\n\n- mine\n- old entry\n";
        let theirs = "# Changelog\n\n## Unreleased\n\n- theirs\n- old entry\n";
        let Resolution::Merged(text) = resolve(BASE, ours, theirs) else {
            panic!("expected a merge");
        };
        assert_eq!(
            text,
            "# Changelog\n\n## Unreleased\n\n- mine\n- theirs\n- old entry\n"
        );
    }

    #[test]
    fn identical_additions_are_not_duplicated() {
        let both = "# Changelog\n\n## Unreleased\n\n- same\n- old entry\n";
        assert_eq!(
            resolve(BASE, both, both),
            Resolution::Merged(both.to_string())
        );
    }

    #[test]
    fn an_edited_or_removed_existing_line_is_refused() {
        let theirs = "# Changelog\n\n## Unreleased\n\n- theirs\n- old entry\n";
        let edited = "# Changelog\n\n## Unreleased\n\n- old entry reworded\n";
        assert!(matches!(
            resolve(BASE, edited, theirs),
            Resolution::Refused(r) if r.contains("changed or removed")
        ));
        let removed = "# Changelog\n\n## Unreleased\n";
        assert!(matches!(
            resolve(BASE, theirs, removed),
            Resolution::Refused(_)
        ));
    }

    #[test]
    fn a_setext_rule_in_the_inputs_is_not_a_marker() {
        let base = "Changelog\n=======\n\n- old\n";
        let ours = "Changelog\n=======\n\n- a\n- old\n";
        let theirs = "Changelog\n=======\n\n- b\n- old\n";
        assert!(matches!(resolve(base, ours, theirs), Resolution::Merged(_)));
        assert!(leftover_markers("x\n=======\ny\n", &["x\ny\n"]));
        assert!(leftover_markers("<<<<<<< ours\n", &[]));
    }
}
