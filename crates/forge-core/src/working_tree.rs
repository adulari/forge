//! Did the repository change during a turn? Shared by the empty-diff, completeness and
//! tools-unavailable gates, which all compare a start-of-turn marker with the current one.

use std::path::Path;

pub(crate) fn git_output(root: Option<&Path>, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args);
    if let Some(root) = root {
        cmd.current_dir(root);
    }
    cmd.output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| out.stdout)
}

/// An untracked path that a build or a test run leaves behind (`__pycache__/`, `node_modules/`,
/// a top-level `target/`), not work the model did. A repository without a `.gitignore` lists
/// these as untracked, so running the tests once would otherwise read as editing a file per
/// compiled module: the review gate saw a two-line fix as a four-file change, and the answer
/// settling saw a pure question as a turn that changed the tree.
pub(crate) fn is_run_artifact(path: &str) -> bool {
    const ANYWHERE: [&str; 8] = [
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        ".ruff_cache",
        ".tox",
        "node_modules",
        ".gradle",
        ".forge",
    ];
    let path = path.trim_matches('"');
    path.ends_with(".pyc")
        || path.ends_with(".pyo")
        || path.split('/').any(|part| ANYWHERE.contains(&part))
        || path.split('/').next() == Some("target")
}

/// Untracked files that count as the model's work: git's own ignore rules apply first, then
/// [`is_run_artifact`]. `None` is a git failure (or no repository).
pub(crate) fn untracked_work_files(root: &Path) -> Option<Vec<String>> {
    let raw = git_output(
        Some(root),
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;
    Some(
        String::from_utf8_lossy(&raw)
            .split('\0')
            .filter(|path| !path.is_empty() && !is_run_artifact(path))
            .map(str::to_string)
            .collect(),
    )
}

/// A cheap fingerprint of the working tree (HEAD, status and diff), or `None` outside a git
/// repository. Tool counters can't tell a review that only ran `git diff` from one that edited:
/// every successful shell command counts as a mutation.
///
/// Untracked files count by content, not only by name (a review that rewrote the `ANSWER.md` the
/// turn had just created left the status line unchanged), and the byproducts of running the
/// tests do not count at all: a verification re-drive that ran `python -m unittest` and so
/// created `__pycache__/` used to read as an edit, and its one-line "tests passed" replaced the
/// answer the user had asked for.
pub(crate) fn worktree_fingerprint(root: &std::path::Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout)
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // A repository with no commits has no HEAD: `status` still succeeds there, so only a missing
    // repository yields `None`.
    git(&["rev-parse", "HEAD"])
        .unwrap_or_default()
        .hash(&mut hasher);
    let status = git(&["status", "--porcelain=v1", "-uall"])?;
    for line in String::from_utf8_lossy(&status).lines() {
        match line.strip_prefix("?? ") {
            Some(path) if crate::working_tree::is_run_artifact(path) => {}
            Some(path) => {
                line.hash(&mut hasher);
                untracked_digest(root, path.trim_matches('"')).hash(&mut hasher);
            }
            None => line.hash(&mut hasher),
        }
    }
    git(&["diff", "HEAD"]).unwrap_or_default().hash(&mut hasher);
    Some(hasher.finish())
}

/// What an untracked file holds, cheaply: its size, and its bytes up to a bound.
pub(crate) fn untracked_digest(root: &std::path::Path, path: &str) -> (u64, u64) {
    use std::hash::{Hash, Hasher};
    const READ_LIMIT: u64 = 64 << 10;
    let Ok(meta) = std::fs::metadata(root.join(path)) else {
        return (0, 0);
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if meta.len() <= READ_LIMIT {
        std::fs::read(root.join(path))
            .unwrap_or_default()
            .hash(&mut hasher);
    } else {
        meta.modified().ok().hash(&mut hasher);
    }
    (meta.len(), hasher.finish())
}

/// Snapshot of "where the work stands": `git status --porcelain` plus the checked-out commit.
///
/// Status alone cannot see a turn whose work was committed. The model edits, runs the tests and
/// commits (the harness itself asks it to commit each verified unit), so the tree is clean before
/// and after and the porcelain output is byte-identical — which read as an empty diff and
/// re-drove every committed turn with "You have not modified any files". A repository with no
/// commit yet has no HEAD; that contributes nothing and status alone decides.
///
/// `None` outside a git repository, which callers treat as "changed" so a code-change nudge can
/// never fire where progress cannot be measured.
pub(crate) fn working_tree_marker(root: Option<&Path>) -> Option<Vec<u8>> {
    let mut marker = git_output(root, &["status", "--porcelain"])?;
    if let Some(head) = git_output(root, &["rev-parse", "--verify", "-q", "HEAD"]) {
        marker.extend_from_slice(b"\0HEAD ");
        marker.extend_from_slice(&head);
    }
    Some(marker)
}

/// Whether the repository moved on from `baseline` (a [`working_tree_marker`]): files modified,
/// added or removed, or the checked-out commit changed. Outside a git repository, conservatively
/// reports changed so code-change nudges never fire.
pub(crate) fn working_tree_changed_since(root: Option<&Path>, baseline: Option<&[u8]>) -> bool {
    match (baseline, working_tree_marker(root)) {
        (Some(before), Some(after)) => before != after,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.invalid"]);
        git(dir.path(), &["config", "user.name", "t"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("f.txt"), "seed").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "seed"]);
        dir
    }

    #[test]
    fn a_turn_that_edits_and_commits_counts_as_progress() {
        let dir = repo();
        let baseline = working_tree_marker(Some(dir.path())).unwrap();
        assert!(!working_tree_changed_since(
            Some(dir.path()),
            Some(&baseline)
        ));

        std::fs::write(dir.path().join("f.txt"), "edited").unwrap();
        git(dir.path(), &["commit", "-qam", "edit"]);

        assert!(
            working_tree_changed_since(Some(dir.path()), Some(&baseline)),
            "the tree is clean again, but HEAD moved: the edit was committed"
        );
    }

    #[test]
    fn an_untouched_repository_is_unchanged() {
        let dir = repo();
        let baseline = working_tree_marker(Some(dir.path())).unwrap();
        assert!(!working_tree_changed_since(
            Some(dir.path()),
            Some(&baseline)
        ));
    }

    #[test]
    fn a_repository_without_commits_still_tracks_status() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        let baseline = working_tree_marker(Some(dir.path())).unwrap();
        assert!(!working_tree_changed_since(
            Some(dir.path()),
            Some(&baseline)
        ));

        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        assert!(working_tree_changed_since(
            Some(dir.path()),
            Some(&baseline)
        ));
    }

    #[test]
    fn test_run_byproducts_are_not_work() {
        for path in [
            "src/roomtool/__pycache__/scheduler.cpython-312.pyc",
            "mod.pyc",
            "web/node_modules/left-pad/index.js",
            ".pytest_cache/v/cache/lastfailed",
            "target/debug/deps/x.d",
            ".forge/state.json",
        ] {
            assert!(is_run_artifact(path), "{path}");
        }
        for path in [
            "src/target_picker.rs",
            "src/target/mod.rs",
            "tests/test_scheduler.py",
            "ANSWER.md",
            "docs/node_modules.md",
        ] {
            assert!(!is_run_artifact(path), "{path}");
        }
    }

    #[test]
    fn untracked_work_files_skip_run_artifacts_and_ignored_paths() {
        let dir = repo();
        std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "x").unwrap();
        std::fs::write(dir.path().join("new file.md"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("pkg/__pycache__")).unwrap();
        std::fs::write(dir.path().join("pkg/__pycache__/m.pyc"), "x").unwrap();
        let mut files = untracked_work_files(dir.path()).unwrap();
        files.sort();
        assert_eq!(files, vec![".gitignore", "new file.md"]);
    }

    #[test]
    fn outside_a_repository_reports_changed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(working_tree_changed_since(Some(dir.path()), Some(b"")));
    }
}
