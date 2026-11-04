//! The TUI diff overlay's "revert hunk": the overlay holds parsed lines, not raw patch text, so
//! the host re-reads the file's diff, cuts out the one hunk the person confirmed (matched by its
//! `@@` header) and applies it in reverse through the same validated `git apply` the daemon's
//! `POST /api/git/hunk` uses.

use std::path::Path;
use std::process::Command;

use crate::serve_git::hunk::{apply_hunk, validate_hunk_patch, HunkAction};

fn git_text(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `--- / +++` header lines plus the hunk starting at `header`, from one file's `git diff` output.
fn single_hunk_patch(diff: &str, header: &str) -> Option<String> {
    let mut preamble = String::new();
    let mut hunk: Option<String> = None;
    for line in diff.lines() {
        if line.starts_with("@@") {
            if hunk.is_some() {
                break;
            }
            if line.trim_end() == header {
                hunk = Some(format!("{line}\n"));
            }
        } else if let Some(current) = hunk.as_mut() {
            current.push_str(line);
            current.push('\n');
        } else if line.starts_with("--- ") || line.starts_with("+++ ") {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    hunk.map(|hunk| format!("{preamble}{hunk}"))
}

/// Revert one hunk of `path` (repository-relative) in the working tree of `workspace`'s repo.
pub(crate) fn revert_hunk(workspace: &Path, path: &str, header: &str) -> Result<String, String> {
    let root = git_text(workspace, &["rev-parse", "--show-toplevel"])
        .map(|root| root.trim().to_string())
        .filter(|root| !root.is_empty())
        .ok_or_else(|| "not inside a git repository".to_string())?;
    let root = Path::new(&root);
    let diff = git_text(
        root,
        &["diff", "HEAD", "--no-color", "--no-ext-diff", "--", path],
    )
    .ok_or_else(|| "could not read the file's diff".to_string())?;
    let raw = single_hunk_patch(&diff, header)
        .ok_or_else(|| "the file changed since the diff was read; refreshed".to_string())?;
    let patch = validate_hunk_patch(path, header, &raw)?;
    match apply_hunk(root, HunkAction::Revert, &patch) {
        Ok(()) => Ok(format!("reverted hunk {header} in {path}")),
        Err((true, _)) => Err("the file changed since the diff was read; refreshed".to_string()),
        Err((false, message)) => Err(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn reverts_only_the_confirmed_hunk() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        let base: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        std::fs::write(dir.path().join("f.txt"), &base).unwrap();
        git(dir.path(), &["add", "f.txt"]);
        git(dir.path(), &["commit", "-qm", "init"]);
        let edited = base
            .replace("line 2\n", "TWO\n")
            .replace("line 28\n", "TWENTY-EIGHT\n");
        std::fs::write(dir.path().join("f.txt"), edited).unwrap();

        let diff = git_text(dir.path(), &["diff", "HEAD", "--", "f.txt"]).unwrap();
        let header = diff
            .lines()
            .find(|l| l.starts_with("@@"))
            .unwrap()
            .to_string();
        revert_hunk(dir.path(), "f.txt", &header).unwrap();
        let now = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
        assert!(now.contains("line 2\n") && now.contains("TWENTY-EIGHT"));

        let err = revert_hunk(dir.path(), "f.txt", &header).unwrap_err();
        assert!(err.contains("changed since"), "{err}");
    }
}
