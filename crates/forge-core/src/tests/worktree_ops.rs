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

fn repo(dir: &Path) -> PathBuf {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-qm", "init"]);
    root.canonicalize().unwrap()
}

fn ctx(current: &Path) -> Context<'_> {
    Context {
        current,
        home: None,
        daemon_owned: None,
        confine: None,
        settings: None,
    }
}

fn req(args: Value) -> Request {
    Request::parse(&args).unwrap()
}

#[test]
fn create_branches_a_managed_worktree_and_keeps_main_status_clean() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let out = execute(
        &req(json!({"action": "create", "name": "Fix Bug"})),
        &ctx(&main),
    );
    assert!(out.ok, "{}", out.text);
    let path = out.switch_to.unwrap();
    assert_eq!(path, main.join(".forge/worktrees/fix-bug"));
    assert_eq!(git(&path, &["branch", "--show-current"]), "forge/fix-bug");
    assert_eq!(git(&main, &["status", "--porcelain"]), "");
}

#[test]
fn create_checks_out_an_existing_branch_and_works_from_a_detached_main_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    git(&main, &["branch", "topic"]);
    git(&main, &["checkout", "-q", "--detach"]);
    let out = execute(
        &req(json!({"action": "create", "branch": "topic"})),
        &ctx(&main),
    );
    assert!(out.ok, "{}", out.text);
    assert!(out.text.contains("existing branch topic"), "{}", out.text);
    let listed = execute(&req(json!({"action": "list"})), &ctx(&main)).text;
    assert!(listed.contains("detached @"), "{listed}");
    assert!(listed.contains("[topic]"), "{listed}");
}

#[test]
fn enter_finds_a_worktree_outside_the_repo_by_branch_path_or_dir_name() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let outside = dir.path().join("mnt/work/wt/side");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            outside.to_str().unwrap(),
        ],
    );
    let outside = outside.canonicalize().unwrap();
    for target in [
        json!({"action": "enter", "branch": "side"}),
        json!({"action": "enter", "path": outside.display().to_string()}),
        json!({"action": "enter", "name": "side"}),
    ] {
        let out = execute(&req(target), &ctx(&main));
        assert_eq!(
            out.switch_to.as_deref(),
            Some(outside.as_path()),
            "{}",
            out.text
        );
    }
    let missing = execute(
        &req(json!({"action": "enter", "branch": "nope"})),
        &ctx(&main),
    );
    assert!(
        !missing.ok && missing.text.contains("Known:"),
        "{}",
        missing.text
    );
}

#[test]
fn confinement_refuses_a_worktree_outside_the_workspace_and_extra_roots() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let outside = dir.path().join("elsewhere/side");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            outside.to_str().unwrap(),
        ],
    );
    let only_main = [main.clone()];
    let confined = Context {
        confine: Some(&only_main),
        settings: None,
        ..ctx(&main)
    };
    let out = execute(
        &req(json!({"action": "enter", "branch": "side"})),
        &confined,
    );
    assert!(!out.ok && out.text.contains("extra_roots"), "{}", out.text);
    let widened = [main.clone(), dir.path().join("elsewhere")];
    let allowed = Context {
        confine: Some(&widened),
        settings: None,
        ..ctx(&main)
    };
    assert!(execute(&req(json!({"action": "enter", "branch": "side"})), &allowed).ok);
}

#[test]
fn remove_refuses_untracked_files_and_keeps_an_unmerged_branch_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let wt = execute(&req(json!({"action": "create", "name": "w"})), &ctx(&main))
        .switch_to
        .unwrap();
    std::fs::write(wt.join("scratch.txt"), "x").unwrap();
    let refused = execute(&req(json!({"action": "remove", "name": "w"})), &ctx(&main));
    assert!(
        !refused.ok && refused.text.contains("scratch.txt"),
        "{}",
        refused.text
    );
    assert!(wt.exists());

    git(&wt, &["add", "scratch.txt"]);
    git(&wt, &["commit", "-qm", "work"]);
    let kept = execute(&req(json!({"action": "remove", "name": "w"})), &ctx(&main));
    assert!(kept.ok, "{}", kept.text);
    assert!(kept.text.contains("branch forge/w kept"), "{}", kept.text);
    assert!(!wt.exists());
    git(&main, &["rev-parse", "--verify", "forge/w"]);
}

#[test]
fn exit_with_remove_after_a_merge_deletes_worktree_and_branch() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let wt = execute(&req(json!({"action": "create", "name": "m"})), &ctx(&main))
        .switch_to
        .unwrap();
    std::fs::write(wt.join("b.txt"), "b\n").unwrap();
    git(&wt, &["add", "b.txt"]);
    git(&wt, &["commit", "-qm", "b"]);
    git(&main, &["merge", "-q", "forge/m"]);
    let out = execute(&req(json!({"action": "exit", "remove": true})), &ctx(&wt));
    assert!(out.ok, "{}", out.text);
    assert_eq!(out.switch_to.as_deref(), Some(main.as_path()));
    assert!(out.text.contains("deleted branch forge/m"), "{}", out.text);
    assert!(!wt.exists());
}

#[test]
fn the_current_main_and_daemon_owned_worktrees_are_never_removed() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let wt = execute(&req(json!({"action": "create", "name": "d"})), &ctx(&main))
        .switch_to
        .unwrap();
    let main_str = main.display().to_string();
    let refuse_main = execute(
        &req(json!({"action": "remove", "path": main_str})),
        &ctx(&wt),
    );
    assert!(
        refuse_main.text.contains("main checkout"),
        "{}",
        refuse_main.text
    );
    let refuse_current = execute(&req(json!({"action": "remove", "name": "d"})), &ctx(&wt));
    assert!(
        refuse_current.text.contains("exit"),
        "{}",
        refuse_current.text
    );
    let owned = Context {
        daemon_owned: Some(&wt),
        ..ctx(&main)
    };
    let refuse_owned = execute(&req(json!({"action": "remove", "name": "d"})), &owned);
    assert!(
        refuse_owned.text.contains("forge serve"),
        "{}",
        refuse_owned.text
    );
    let daemon_session = Context {
        daemon_owned: Some(&wt),
        ..ctx(&wt)
    };
    let refuse_exit = execute(&req(json!({"action": "exit"})), &daemon_session);
    assert!(
        !refuse_exit.ok && refuse_exit.switch_to.is_none(),
        "{}",
        refuse_exit.text
    );
    assert!(wt.exists());
}

#[test]
fn moving_between_checkouts_is_read_only_but_creating_and_removing_are_not() {
    assert_eq!(
        req(json!({"action": "enter", "branch": "x"})).side_effect(),
        SideEffect::ReadOnly
    );
    assert_eq!(
        req(json!({"action": "exit"})).side_effect(),
        SideEffect::ReadOnly
    );
    assert_eq!(
        req(json!({"action": "exit", "remove": true})).side_effect(),
        SideEffect::Shell
    );
    assert_eq!(
        req(json!({"action": "create", "name": "x"})).side_effect(),
        SideEffect::Shell
    );
    assert!(Request::parse(&json!({"action": "teleport"})).is_err());
}

#[test]
fn a_daemon_session_leaving_a_second_worktree_returns_to_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let owned = execute(
        &req(json!({"action": "create", "name": "owned"})),
        &ctx(&main),
    )
    .switch_to
    .unwrap();
    let second = execute(
        &req(json!({"action": "create", "name": "second"})),
        &ctx(&owned),
    )
    .switch_to
    .unwrap();
    let from_second = Context {
        daemon_owned: Some(&owned),
        ..ctx(&second)
    };
    let out = execute(&req(json!({"action": "exit"})), &from_second);
    assert_eq!(
        out.switch_to.as_deref(),
        Some(owned.as_path()),
        "{}",
        out.text
    );
}

#[test]
fn pop_stash_applies_the_named_entry_not_the_newest() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    std::fs::write(main.join("a.txt"), "mine\n").unwrap();
    git(&main, &["stash", "push", "-m", "mine"]);
    let mine = top_stash(&main).unwrap();
    std::fs::write(main.join("a.txt"), "theirs\n").unwrap();
    git(&main, &["stash", "push", "-m", "theirs"]);
    pop_stash(&main, &mine, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(main.join("a.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(git(&main, &["stash", "list"]).lines().count(), 1);
    assert!(pop_stash(&main, &mine, false).is_err());
}

#[test]
fn global_root_worktrees_pass_confinement_for_create_and_enter() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
    let main = repo(dir.path());
    let settings = forge_config::worktree::WorktreeConfig {
        root: forge_config::worktree::WorktreeRoot::Global,
        ..Default::default()
    };
    let only_main = [main.clone()];
    let confined = Context {
        confine: Some(&only_main),
        settings: Some(&settings),
        ..ctx(&main)
    };
    let created = execute(&req(json!({"action": "create", "name": "far"})), &confined);
    assert!(created.ok, "{}", created.text);
    let path = created.switch_to.unwrap();
    assert!(!path.starts_with(&main), "{}", path.display());
    let entered = execute(
        &req(json!({"action": "enter", "branch": "forge/far"})),
        &confined,
    );
    assert!(entered.ok, "{}", entered.text);
}

#[test]
fn the_overlay_never_force_deletes_a_branch_that_gained_commits() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let made = execute(
        &req(json!({"action": "create", "name": "wip"})),
        &ctx(&main),
    );
    let wt = made.switch_to.unwrap();
    std::fs::write(wt.join("new.txt"), "x\n").unwrap();
    git(&wt, &["add", "new.txt"]);
    git(&wt, &["commit", "-qm", "unmerged work"]);
    let out = execute(
        &req(
            json!({"action": "remove", "path": wt.to_str().unwrap(), "force": true, "keep_unmerged": true}),
        ),
        &ctx(&main),
    );
    assert!(out.ok, "{}", out.text);
    assert!(out.text.contains("kept"), "{}", out.text);
    assert!(!git(&main, &["branch", "--list", "forge/wip"]).is_empty());
}

async fn cancelled_create(main: &Path, carry: bool) -> PathBuf {
    let settings = forge_config::worktree::WorktreeConfig {
        setup: Some("sleep 2".into()),
        ..Default::default()
    };
    let request = req(json!({
        "action": "create",
        "name": "orphan",
        "dirty": if carry { "carry" } else { "leave" },
    }));
    let call = crate::session_worktree::run_off_thread(
        request,
        main.to_path_buf(),
        None,
        None,
        None,
        settings,
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(500), call)
            .await
            .is_err(),
        "the create should still be running when it is cancelled"
    );
    let path = main.join(".forge/worktrees/orphan");
    // The abandoned blocking task finishes its setup, then decides what to do with the worktree.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    path
}

#[tokio::test]
async fn cancelling_a_create_that_carried_changes_keeps_them() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    std::fs::write(main.join("a.txt"), "my uncommitted work\n").unwrap();
    let path = cancelled_create(&main, true).await;
    assert!(path.is_dir(), "the worktree must survive");
    assert_eq!(
        std::fs::read_to_string(path.join("a.txt")).unwrap(),
        "my uncommitted work\n"
    );
    assert!(!git(&main, &["branch", "--list", "forge/orphan"]).is_empty());
}

#[tokio::test]
async fn cancelling_a_clean_create_removes_only_the_fresh_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let path = cancelled_create(&main, false).await;
    assert!(!path.exists(), "a clean, empty worktree is cleaned up");
    assert!(git(&main, &["branch", "--list", "forge/orphan"]).is_empty());
}
