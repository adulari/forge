use super::*;

fn git_in(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo_with_remote() -> (tempfile::TempDir, tempfile::TempDir) {
    let remote = tempfile::tempdir().unwrap();
    git_in(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    let work = tempfile::tempdir().unwrap();
    git_in(work.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(work.path().join("a.txt"), "one\n").unwrap();
    git_in(work.path(), &["add", "a.txt"]);
    git_in(work.path(), &["commit", "-qm", "init"]);
    git_in(
        work.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    (work, remote)
}

#[test]
fn discard_restores_tracked_files_and_deletes_untracked_ones() {
    let (work, _remote) = repo_with_remote();
    std::fs::write(work.path().join("a.txt"), "changed\n").unwrap();
    std::fs::write(work.path().join("new.txt"), "scratch\n").unwrap();
    std::fs::write(work.path().join("keep.txt"), "keep\n").unwrap();
    let discarded =
        discard_paths(work.path(), &["a.txt".to_string(), "new.txt".to_string()]).unwrap();
    assert_eq!(discarded, 2);
    assert_eq!(
        std::fs::read_to_string(work.path().join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(!work.path().join("new.txt").exists());
    assert!(
        work.path().join("keep.txt").exists(),
        "unrelated files stay"
    );
}

#[test]
fn discard_leaves_staged_changes_alone() {
    let (work, _remote) = repo_with_remote();
    std::fs::write(work.path().join("a.txt"), "staged\n").unwrap();
    git_in(work.path(), &["add", "a.txt"]);
    std::fs::write(work.path().join("a.txt"), "staged\nthen more\n").unwrap();
    discard_paths(work.path(), &["a.txt".to_string()]).unwrap();
    assert_eq!(
        std::fs::read_to_string(work.path().join("a.txt")).unwrap(),
        "staged\n",
        "the working tree goes back to the index, not to HEAD"
    );
}

#[test]
fn discard_refuses_a_path_that_does_not_exist() {
    let (work, _remote) = repo_with_remote();
    assert!(discard_paths(work.path(), &["ghost.txt".to_string()]).is_err());
}

#[tokio::test]
async fn a_branch_without_an_upstream_pushes_and_gains_one() {
    let (work, remote) = repo_with_remote();
    assert_eq!(upstream_of(work.path(), "main"), None);
    let branch = current_branch(work.path()).ok().unwrap();
    assert_eq!(branch, "main");
    let remote_name = resolve_remote(work.path(), Some("main"), None)
        .ok()
        .unwrap();
    assert_eq!(remote_name, "origin");
    run_network_git(
        work.path(),
        &[
            "push".to_string(),
            "--set-upstream".to_string(),
            remote_name,
            "HEAD:refs/heads/main".to_string(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        upstream_of(work.path(), "main"),
        Some(("origin".to_string(), "main".to_string()))
    );
    let pushed = git_stdout(remote.path(), &["rev-parse", "main"]).unwrap();
    assert_eq!(
        pushed,
        git_stdout(work.path(), &["rev-parse", "HEAD"]).unwrap()
    );
}

#[tokio::test]
async fn a_remote_that_runs_commands_never_executes() {
    let (work, _remote) = repo_with_remote();
    let marker = work.path().join("pwned");
    let evil = format!("ext::sh -c touch% {}", marker.display());
    git_in(work.path(), &["remote", "add", "evil", &evil]);
    let outcome = run_network_git(work.path(), &["fetch".to_string(), "evil".to_string()]).await;
    assert!(
        outcome.is_err(),
        "the ext transport is not on the allow-list"
    );
    assert!(!marker.exists(), "the command must not have run");
    assert!(resolve_remote(work.path(), None, Some(&evil)).is_err());
}

#[test]
fn the_remote_resolves_from_the_branch_then_origin() {
    let (work, _remote) = repo_with_remote();
    assert_eq!(
        resolve_remote(work.path(), Some("main"), None)
            .ok()
            .as_deref(),
        Some("origin")
    );
    assert!(resolve_remote(work.path(), Some("main"), Some("--upload-pack=x")).is_err());
    assert!(resolve_remote(work.path(), Some("main"), Some("nope")).is_err());
    git_in(work.path(), &["remote", "remove", "origin"]);
    assert!(resolve_remote(work.path(), Some("main"), None).is_err());
}
