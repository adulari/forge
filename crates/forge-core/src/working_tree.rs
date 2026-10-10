//! Did the repository change during a turn? Shared by the empty-diff, completeness and
//! tools-unavailable gates, which all compare a start-of-turn marker with the current one.

use std::path::Path;

fn git_output(root: Option<&Path>, args: &[&str]) -> Option<Vec<u8>> {
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
    fn outside_a_repository_reports_changed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(working_tree_changed_since(Some(dir.path()), Some(b"")));
    }
}
