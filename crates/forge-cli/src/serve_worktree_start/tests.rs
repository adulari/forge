use super::*;

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

fn repo(dir: &Path) -> PathBuf {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a"), "a").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "init"]);
    root.canonicalize().unwrap()
}

#[tokio::test]
async fn creating_returns_before_a_slow_setup_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let started = std::time::Instant::now();
    let created = create_off_thread(&root, Some("slow".into())).await.unwrap();
    let pending = Pending {
        repo: root.clone(),
        path: created.guard.path().to_path_buf(),
        branch: created.guard.branch().to_string(),
        settings: forge_config::worktree::WorktreeConfig {
            setup: Some("sleep 2; echo ready > .ready".into()),
            ..Default::default()
        },
    };
    let path = pending.path.clone();
    let cell = SetupCell::default();
    spawn_provisioning(pending, cell.clone(), Default::default());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let state = |c: &SetupCell| c.lock().unwrap().as_ref().map(|s| s.state.clone());
    assert_eq!(state(&cell).as_deref(), Some("running"));
    std::mem::forget(created.guard);
    for _ in 0..100 {
        if state(&cell).as_deref() != Some("running") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(state(&cell).as_deref(), Some("done"));
    assert!(path.join(".ready").is_file());
}

#[tokio::test]
async fn a_failed_setup_is_published_with_its_output() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let created = create_off_thread(&root, Some("bad".into())).await.unwrap();
    let pending = Pending {
        repo: root,
        path: created.guard.path().to_path_buf(),
        branch: created.guard.branch().to_string(),
        settings: forge_config::worktree::WorktreeConfig {
            setup: Some("echo boom >&2; exit 4".into()),
            ..Default::default()
        },
    };
    std::mem::forget(created.guard);
    let cell = SetupCell::default();
    spawn_provisioning(pending, cell.clone(), Default::default());
    for _ in 0..100 {
        if cell
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.state != "running")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let status = cell.lock().unwrap().clone().unwrap();
    assert_eq!(status.state, "failed");
    assert!(status.message.contains("boom"), "{}", status.message);
}

#[test]
fn a_stored_setup_problem_is_shown_until_the_agent_has_been_told() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let wt = root.display().to_string();
    write_marker(&root, "running", "");
    assert_eq!(interrupted_status(&wt).unwrap().state, "interrupted");
    write_marker(&root, "done", "ok");
    assert!(interrupted_status(&wt).is_none());
    write_marker(&root, "running", "");
    clear_marker(&wt);
    assert!(interrupted_status(&wt).is_none(), "cleared once delivered");
}

fn rev(dir: &Path, rev: &str) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", rev])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn a_new_worktree_branches_from_an_origin_that_is_ahead_and_keeps_the_note() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let mine = repo(&base);
    let origin = base.join("origin.git");
    git(
        &base,
        &[
            "clone",
            "-q",
            "--bare",
            mine.to_str().unwrap(),
            origin.to_str().unwrap(),
        ],
    );
    git(
        &mine,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&mine, &["fetch", "-q", "origin"]);
    git(&mine, &["remote", "set-head", "origin", "main"]);
    let other = base.join("other");
    git(
        &base,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    git(&other, &["config", "user.email", "t@t"]);
    git(&other, &["config", "user.name", "t"]);
    for n in 0..3 {
        std::fs::write(other.join(format!("r{n}")), "x").unwrap();
        git(&other, &["add", "."]);
        git(&other, &["commit", "-qm", &format!("r{n}")]);
    }
    git(&other, &["push", "-q", "origin", "main"]);
    let tip = rev(&other, "HEAD");
    let local = rev(&mine, "HEAD");

    let created = create_only(&mine, Some("fresh")).unwrap();

    assert_eq!(rev(created.guard.path(), "HEAD"), tip);
    assert_eq!(rev(&mine, "HEAD"), local);
    assert_eq!(
        forge_core::worktree_base::stored_note(created.guard.path()).unwrap(),
        format!(
            "based on origin/main @{} (local main was 3 behind)",
            &tip[..7]
        )
    );
}
