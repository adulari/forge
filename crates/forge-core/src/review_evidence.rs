//! The change a completeness review looks at, handed over with the review itself.
//!
//! The review used to tell the model to run `git status --short` and `git diff <start>`, which
//! cost one full model round trip (the whole context again, plus the tool call) before the model
//! could do the only part that matters, comparing the change with the request. Forge can run the
//! same two commands in a few milliseconds, so the review carries their output. A change too big
//! to carry falls back to the instruction, which then also covers what the model has to page.

use std::path::Path;

use crate::working_tree::{git_output, is_run_artifact, untracked_work_files};

/// Largest evidence block attached. The review is a transcript message that later renders send
/// again, so this stays near the size of a small change; a bigger one is reviewed the old way.
const MAX_EVIDENCE_BYTES: usize = 12_000;
/// Largest untracked file shown whole (a bigger one is read by the model, as before).
const MAX_UNTRACKED_FILE_BYTES: usize = 6_000;

fn text(root: &Path, args: &[&str]) -> Option<String> {
    git_output(Some(root), args).map(|raw| String::from_utf8_lossy(&raw).into_owned())
}

/// `git status --short` without the files a build or test run leaves behind.
fn status(root: &Path) -> Option<String> {
    let raw = text(root, &["status", "--short"])?;
    let kept: Vec<&str> = raw
        .lines()
        .filter(|line| !is_run_artifact(line.get(3..).unwrap_or(line)))
        .collect();
    Some(kept.join("\n"))
}

/// Status, the diff against the commit the turn started from, and the content of each new file,
/// or `None` when there is nothing to show or it does not fit [`MAX_EVIDENCE_BYTES`].
pub(crate) fn gather(root: &Path, base: Option<&str>) -> Option<String> {
    let status = status(root)?;
    let diff = text(
        root,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            base.unwrap_or("HEAD"),
        ],
    )?;
    if status.trim().is_empty() && diff.trim().is_empty() {
        return None;
    }
    let mut out = format!(
        "$ git status --short\n{}\n\n$ git diff {}\n{}",
        status.trim_end(),
        base.unwrap_or("HEAD"),
        diff.trim_end()
    );
    for path in untracked_work_files(root)? {
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
        let evidence = gather(dir.path(), Some(&base)).unwrap();
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
        let evidence = gather(dir.path(), Some(&base)).unwrap();
        assert!(evidence.contains("-one\n+two"), "{evidence}");
    }

    #[test]
    fn nothing_changed_or_too_much_changed_falls_back_to_the_instruction() {
        let (dir, base) = repo();
        assert!(gather(dir.path(), Some(&base)).is_none());
        std::fs::write(dir.path().join("a.txt"), "x\n".repeat(MAX_EVIDENCE_BYTES)).unwrap();
        assert!(gather(dir.path(), Some(&base)).is_none());
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        std::fs::write(
            dir.path().join("big.txt"),
            "x".repeat(MAX_UNTRACKED_FILE_BYTES + 1),
        )
        .unwrap();
        assert!(gather(dir.path(), Some(&base)).is_none());
    }
}
