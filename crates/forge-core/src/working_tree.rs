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

/// The state of the working tree as the progress ledger tells states apart: a repository's
/// [`worktree_fingerprint`], or outside one a bounded scan of the directory's files, so that
/// shell-driven work is just as visible in a scratch directory. `None` when the tree cannot be
/// read or is too large to scan cheaply, which the ledger treats as "no evidence".
pub(crate) fn progress_fingerprint(root: &std::path::Path) -> Option<u64> {
    worktree_fingerprint(root).or_else(|| scan_fingerprint(root))
}

/// Entries, depth and bytes a scan may cover before it gives up or stops reading contents.
const SCAN_MAX_ENTRIES: usize = 5000;
const SCAN_MAX_DEPTH: usize = 8;
const SCAN_READ_BUDGET: u64 = 4 << 20;
const SCAN_SMALL_FILE: u64 = 64 << 10;

/// A fingerprint of every file under `root` by path and content (size and mtime for the large
/// ones), build byproducts and `.git` left out. Contents rather than mtimes for ordinary files, so
/// a file that flips between two texts is in a known state each time it comes back to one.
fn scan_fingerprint(root: &std::path::Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut files: Vec<(String, u64, u64)> = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut read = 0u64;
    let mut visited = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        // An unreadable directory or a file that vanished mid-scan is left out, not fatal.
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).ok()?.to_string_lossy().into_owned();
            if rel == ".git" || rel.ends_with("/.git") || is_run_artifact(&rel) {
                continue;
            }
            visited += 1;
            if visited > SCAN_MAX_ENTRIES {
                return None;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                if depth + 1 > SCAN_MAX_DEPTH {
                    return None;
                }
                pending.push((path, depth + 1));
                continue;
            }
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            if meta.is_file()
                && meta.len() <= SCAN_SMALL_FILE
                && read + meta.len() <= SCAN_READ_BUDGET
            {
                read += meta.len();
                std::fs::read(&path).unwrap_or_default().hash(&mut hasher);
            } else {
                meta.modified().ok().hash(&mut hasher);
            }
            files.push((rel, meta.len(), hasher.finish()));
        }
    }
    files.sort_unstable();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut hasher);
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

/// Snapshot of "where the work stands": the [`worktree_fingerprint`] (HEAD, status, the diff
/// against HEAD and each untracked file's content).
///
/// Status alone cannot see a turn whose work was committed. The model edits, runs the tests and
/// commits (the harness itself asks it to commit each verified unit), so the tree is clean before
/// and after and the porcelain output is byte-identical — which read as an empty diff and
/// re-drove every committed turn with "You have not modified any files". Nor can it see a turn
/// that edits files an earlier turn left modified or untracked: the status lines are the same
/// before and after. In a multi-turn bridge session (whose edits never reach the direct-path edit
/// counter) every turn after the first few was re-driven that way. A repository with no commit
/// yet has no HEAD; that contributes nothing.
///
/// `None` outside a git repository, which callers treat as "changed" so a code-change nudge can
/// never fire where progress cannot be measured.
pub(crate) fn working_tree_marker(root: Option<&Path>) -> Option<Vec<u8>> {
    let root = match root {
        Some(root) => root.to_path_buf(),
        None => std::env::current_dir().ok()?,
    };
    worktree_fingerprint(&root).map(|hash| hash.to_le_bytes().to_vec())
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
    fn editing_files_an_earlier_turn_left_dirty_counts_as_progress() {
        let dir = repo();
        std::fs::write(dir.path().join("f.txt"), "earlier turn").unwrap();
        std::fs::write(dir.path().join("new.py"), "x = 1\n").unwrap();
        let baseline = working_tree_marker(Some(dir.path())).unwrap();
        assert!(!working_tree_changed_since(
            Some(dir.path()),
            Some(&baseline)
        ));

        std::fs::write(dir.path().join("f.txt"), "this turn").unwrap();
        assert!(
            working_tree_changed_since(Some(dir.path()), Some(&baseline)),
            "same ` M f.txt` status line, different content"
        );

        let baseline = working_tree_marker(Some(dir.path())).unwrap();
        std::fs::write(dir.path().join("new.py"), "x = 2\n").unwrap();
        assert!(
            working_tree_changed_since(Some(dir.path()), Some(&baseline)),
            "same `?? new.py` status line, different content"
        );
    }

    #[test]
    fn running_the_tests_alone_is_not_progress() {
        let dir = repo();
        let baseline = working_tree_marker(Some(dir.path())).unwrap();
        std::fs::create_dir(dir.path().join("__pycache__")).unwrap();
        std::fs::write(dir.path().join("__pycache__/m.pyc"), "junk").unwrap();
        assert!(!working_tree_changed_since(
            Some(dir.path()),
            Some(&baseline)
        ));
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

    #[test]
    fn a_scan_tells_contents_apart_and_a_file_that_comes_back_is_in_a_known_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a").unwrap();
        let a = progress_fingerprint(dir.path()).unwrap();
        std::fs::write(dir.path().join("f.txt"), "b").unwrap();
        let b = progress_fingerprint(dir.path()).unwrap();
        std::fs::write(dir.path().join("f.txt"), "a").unwrap();
        assert_ne!(a, b);
        assert_eq!(a, progress_fingerprint(dir.path()).unwrap());
    }

    #[test]
    fn a_scan_counts_directories_too_so_a_tree_of_empty_ones_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..SCAN_MAX_ENTRIES + 1 {
            std::fs::create_dir(dir.path().join(format!("d{i}"))).unwrap();
        }
        assert_eq!(progress_fingerprint(dir.path()), None);
    }

    #[test]
    fn a_scan_ignores_build_byproducts_and_gives_up_on_a_huge_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a").unwrap();
        let before = progress_fingerprint(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/out.o"), "x").unwrap();
        std::fs::create_dir(dir.path().join("__pycache__")).unwrap();
        std::fs::write(dir.path().join("__pycache__/m.pyc"), "x").unwrap();
        assert_eq!(before, progress_fingerprint(dir.path()).unwrap());
        for i in 0..SCAN_MAX_ENTRIES + 1 {
            std::fs::write(dir.path().join(format!("n{i}.txt")), "").unwrap();
        }
        assert_eq!(progress_fingerprint(dir.path()), None);
    }
}
