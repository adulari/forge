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
