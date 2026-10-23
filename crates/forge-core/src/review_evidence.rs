//! The change a completeness review looks at, handed over with the review itself.
//!
//! The review used to tell the model to run `git status --short` and `git diff <start>`, which
//! cost one full model round trip (the whole context again, plus the tool call) before the model
//! could do the only part that matters, comparing the change with the request. Forge can run the
//! same two commands in a few milliseconds, so the review carries their output. A change too big
//! to carry falls back to the instruction, which then also covers what the model has to page.

use std::path::Path;

use crate::review_gate::TurnBaseline;
use crate::working_tree::{git_output, is_run_artifact, untracked_work_files};

/// Largest evidence block attached. The review is a transcript message that later renders send
/// again, so this stays near the size of a small change; a bigger one is reviewed the old way.
const MAX_EVIDENCE_BYTES: usize = 12_000;
/// Largest untracked file shown whole (a bigger one is read by the model, as before).
const MAX_UNTRACKED_FILE_BYTES: usize = 6_000;
/// More changed paths than this is past the evidence budget anyway.
const MAX_PATHS: usize = 200;
const NOTHING_CHANGED: &str = "No file changed during this turn.";

fn text(root: &Path, args: &[&str]) -> Option<String> {
    git_output(Some(root), args).map(|raw| String::from_utf8_lossy(&raw).into_owned())
}

/// `git status --short` without the files a build or test run leaves behind.
fn status(root: &Path, pathspec: &[&str]) -> Option<String> {
    let mut args = vec!["--literal-pathspecs", "status", "--short"];
    if !pathspec.is_empty() {
        args.push("--");
        args.extend_from_slice(pathspec);
    }
    let raw = text(root, &args)?;
    let kept: Vec<&str> = raw
        .lines()
        .filter(|line| !is_run_artifact(line.get(3..).unwrap_or(line)))
        .collect();
    Some(kept.join("\n"))
}

/// Status, the diff against the commit the turn started from, and the content of each new file,
/// all limited to the paths this turn changed, or `None` when there is nothing to show or it does
/// not fit [`MAX_EVIDENCE_BYTES`]. Work an earlier turn left uncommitted is not this turn's
/// change: in a multi-turn session it pushed the evidence past the limit, and the review then
/// cost a model round re-running `git diff`. A default `baseline` limits nothing.
pub(crate) fn gather(root: &Path, base: Option<&str>, baseline: &TurnBaseline) -> Option<String> {
    let only = baseline.changed_paths(root, base);
    match &only {
        Some(paths) if paths.is_empty() => return Some(NOTHING_CHANGED.to_string()),
        Some(paths) if paths.len() > MAX_PATHS => return None,
        _ => {}
    }
    let pathspec: Vec<&str> = only.iter().flatten().map(String::as_str).collect();
    let base = base.unwrap_or("HEAD");
    let mut diff_args = vec![
        "--literal-pathspecs",
        "diff",
        "--no-color",
        "--no-ext-diff",
        base,
    ];
    let mut shown = String::new();
    if !pathspec.is_empty() {
        diff_args.push("--");
        diff_args.extend_from_slice(&pathspec);
        shown = format!(" -- {}", pathspec.join(" "));
    }
    let status = status(root, &pathspec)?;
    let diff = text(root, &diff_args)?;
    if status.trim().is_empty() && diff.trim().is_empty() {
        return None;
    }
    let mut out = format!(
        "$ git status --short{shown}\n{}\n\n$ git diff {base}{shown}\n{}",
        status.trim_end(),
        diff.trim_end()
    );
    for path in untracked_work_files(root)? {
        if only.as_ref().is_some_and(|paths| !paths.contains(&path)) {
            continue;
        }
        let file = root.join(&path);
        // The size is read before the file is: an untracked data file can be gigabytes.
        if std::fs::metadata(&file).ok()?.len() > MAX_UNTRACKED_FILE_BYTES as u64 {
            return None;
        }
        let Ok(content) = String::from_utf8(std::fs::read(&file).ok()?) else {
            continue;
        };
        out.push_str(&format!(
            "\n\n$ cat {path}  (untracked)\n{}",
            content.trim_end()
        ));
        if out.len() > MAX_EVIDENCE_BYTES {
            return None;
        }
    }
    (out.len() <= MAX_EVIDENCE_BYTES).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        let head = text(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        (dir, head.trim().to_string())
    }

    #[test]
    fn carries_the_tracked_diff_and_the_new_files_but_not_test_byproducts() {
        let (dir, base) = repo();
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.path().join("new.py"), "print('x')\n").unwrap();
        std::fs::create_dir(dir.path().join("__pycache__")).unwrap();
        std::fs::write(dir.path().join("__pycache__/new.pyc"), "junk").unwrap();
        let evidence = gather(dir.path(), Some(&base), &TurnBaseline::default()).unwrap();
        assert!(evidence.contains("$ git status --short"), "{evidence}");
        assert!(evidence.contains("-one\n+two"), "{evidence}");
        assert!(
            evidence.contains("$ cat new.py  (untracked)\nprint('x')"),
            "{evidence}"
        );
        assert!(!evidence.contains("pycache"), "{evidence}");
    }

    #[test]
    fn a_commit_made_during_the_turn_is_still_in_the_diff() {
        let (dir, base) = repo();
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        git(dir.path(), &["commit", "-qam", "change"]);
        let evidence = gather(dir.path(), Some(&base), &TurnBaseline::default()).unwrap();
        assert!(evidence.contains("-one\n+two"), "{evidence}");
    }

    #[test]
    fn work_an_earlier_turn_left_uncommitted_is_not_in_the_evidence() {
        let (dir, _) = repo();
        std::fs::write(dir.path().join("b.txt"), "base\n").unwrap();
        git(dir.path(), &["add", "b.txt"]);
        git(dir.path(), &["commit", "-qm", "b"]);
        let base = text(dir.path(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        std::fs::write(dir.path().join("a.txt"), "earlier turn\n").unwrap();
        std::fs::write(dir.path().join("old.py"), "earlier = 1\n").unwrap();
        let baseline = TurnBaseline::capture(dir.path(), Some(&base));
        assert_eq!(
            gather(dir.path(), Some(&base), &baseline).as_deref(),
            Some(NOTHING_CHANGED)
        );
        std::fs::write(dir.path().join("b.txt"), "this turn\n").unwrap();
        std::fs::write(dir.path().join("new.py"), "now = 2\n").unwrap();
        let evidence = gather(dir.path(), Some(&base), &baseline).unwrap();
        assert!(evidence.contains("-base\n+this turn"), "{evidence}");
        assert!(evidence.contains("$ cat new.py  (untracked)"), "{evidence}");
        assert!(!evidence.contains("earlier"), "{evidence}");
        assert!(!evidence.contains("a.txt"), "{evidence}");
    }

    #[test]
    fn nothing_changed_says_so_and_too_much_changed_falls_back_to_the_instruction() {
        let (dir, base) = repo();
        assert_eq!(
            gather(dir.path(), Some(&base), &TurnBaseline::default()).as_deref(),
            Some(NOTHING_CHANGED)
        );
        std::fs::write(dir.path().join("a.txt"), "x\n".repeat(MAX_EVIDENCE_BYTES)).unwrap();
        assert!(gather(dir.path(), Some(&base), &TurnBaseline::default()).is_none());
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        std::fs::write(
            dir.path().join("big.txt"),
            "x".repeat(MAX_UNTRACKED_FILE_BYTES + 1),
        )
        .unwrap();
        assert!(gather(dir.path(), Some(&base), &TurnBaseline::default()).is_none());
    }
}
