use super::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn identity(dir: &Path) {
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
}

#[test]
fn repo_names_come_from_the_last_url_segment() {
    for (url, name) in [
        ("https://github.com/adulari/forge.git", "forge"),
        ("https://github.com/adulari/forge", "forge"),
        ("git@github.com:adulari/forge.git", "forge"),
        ("ssh://git@host/x/y.z.git/", "y.z"),
    ] {
        assert_eq!(repo_name_from_url(url).as_deref(), Some(name), "{url}");
    }
    assert_eq!(repo_name_from_url("https://host/.."), None);
}

#[test]
fn clone_refuses_code_running_and_local_urls_before_touching_disk() {
    let dir = tempfile::tempdir().unwrap();
    for url in [
        "ext::sh -c touch /tmp/x",
        "file:///etc",
        "/local/path",
        "-oProxyCommand=x",
    ] {
        assert!(clone_repo(url, dir.path(), None).is_err(), "{url}");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn clone_rejects_odd_folder_names_and_missing_parents() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://example.invalid/x.git";
    assert!(clone_repo(url, dir.path(), Some("../evil")).is_err());
    assert!(clone_repo(url, &dir.path().join("missing"), None).is_err());
}

#[test]
fn init_makes_a_folder_a_repo_with_a_head_commit() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    assert_eq!(git(dir.path(), &["rev-list", "--count", "HEAD"]), "1");
    assert!(init_repo(dir.path()).is_err(), "second init refused");
}

#[test]
fn gh_json_parses_and_skips_malformed_rows() {
    let repos = parse_gh_repos(
        r#"[{"nameWithOwner":"a/b","url":"https://github.com/a/b","description":null,"isPrivate":true},{"url":"x"}]"#,
    )
    .unwrap();
    assert_eq!(repos.len(), 1);
    assert!(repos[0].private && repos[0].description.is_empty());
    assert!(parse_gh_repos("nope").is_none());
}

/// An upstream bare repo with one commit and a clone of it.
fn upstream_and_clone(dir: &Path) -> (PathBuf, PathBuf) {
    let bare = dir.join("up.git");
    let seed = dir.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    identity(&seed);
    std::fs::write(seed.join("a.txt"), "1\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-qm", "one"]);
    git(
        dir,
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let work = dir.join("work");
    git(
        dir,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    identity(&work);
    git(&seed, &["remote", "add", "up", bare.to_str().unwrap()]);
    (seed, work)
}

fn push_change(seed: &Path, file: &str, text: &str) {
    std::fs::write(seed.join(file), text).unwrap();
    git(seed, &["add", "."]);
    git(seed, &["commit", "-qm", "change"]);
    git(seed, &["push", "-q", "up", "HEAD:main"]);
}

#[test]
fn pull_with_dirty_tree_stashes_and_restores() {
    let dir = tempfile::tempdir().unwrap();
    let (seed, work) = upstream_and_clone(dir.path());
    push_change(&seed, "b.txt", "new\n");
    std::fs::write(work.join("a.txt"), "local edit\n").unwrap();
    std::fs::write(work.join("scratch.txt"), "untracked\n").unwrap();
    let out = pull(&work);
    assert_eq!(out.status, PullStatus::Updated, "{out:?}");
    assert!(out.stashed);
    assert!(work.join("b.txt").is_file());
    assert_eq!(
        std::fs::read_to_string(work.join("a.txt")).unwrap(),
        "local edit\n"
    );
    assert!(work.join("scratch.txt").is_file());
    assert_eq!(pull(&work).status, PullStatus::UpToDate);
}

#[test]
fn pull_conflict_is_left_in_place_with_a_handoff_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let (seed, work) = upstream_and_clone(dir.path());
    std::fs::write(work.join("a.txt"), "mine\n").unwrap();
    git(&work, &["commit", "-qam", "mine"]);
    push_change(&seed, "a.txt", "theirs\n");
    let out = pull(&work);
    assert_eq!(out.status, PullStatus::Conflict, "{out:?}");
    assert_eq!(out.conflicts, vec!["a.txt".to_string()]);
    assert!(out.handoff_prompt.unwrap().contains("a.txt"));
}

#[test]
fn stash_conflict_keeps_the_stash_and_names_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let (seed, work) = upstream_and_clone(dir.path());
    push_change(&seed, "a.txt", "theirs\n");
    std::fs::write(work.join("a.txt"), "local\n").unwrap();
    let out = pull(&work);
    assert_eq!(out.status, PullStatus::StashConflict, "{out:?}");
    assert_eq!(out.conflicts, vec!["a.txt".to_string()]);
    assert!(!git(&work, &["stash", "list"]).is_empty());
}

#[test]
fn pull_without_an_upstream_fails_before_touching_local_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (_seed, work) = upstream_and_clone(dir.path());
    git(&work, &["checkout", "-q", "-b", "forge/local-only"]);
    std::fs::write(work.join("a.txt"), "local edit\n").unwrap();
    let out = pull(&work);
    assert_eq!(out.status, PullStatus::Failed, "{out:?}");
    assert!(out.message.contains("no upstream"), "{out:?}");
    assert!(!out.stashed);
    assert!(git(&work, &["stash", "list"]).is_empty());
    assert_eq!(
        std::fs::read_to_string(work.join("a.txt")).unwrap(),
        "local edit\n"
    );
}

#[test]
fn init_repo_refuses_a_folder_inside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let (_seed, work) = upstream_and_clone(dir.path());
    std::fs::create_dir_all(work.join("src")).unwrap();
    let err = init_repo(&work.join("src")).unwrap_err();
    assert!(err.contains("already inside"), "{err}");
    assert!(!work.join("src/.git").exists());
}
