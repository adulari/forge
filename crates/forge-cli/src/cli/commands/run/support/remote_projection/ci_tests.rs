use super::*;

fn frame_in(cwd: &str) -> remote::Snapshot {
    build_snapshot_frame(
        &forge_tui::App::default(),
        SnapshotIdentity {
            session_id: "sess-ci",
            title: "",
            cwd,
            worktree: None,
            project_initialized: true,
            project_init_hint: None,
            exposure: "loopback".into(),
            model_pinned: false,
            worktree_setup: None,
            worktree_base: None,
        },
        None,
        0,
        Vec::new(),
        1,
    )
}

#[test]
fn the_frame_carries_the_ci_summary_only_once_the_repository_has_had_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(
        wf.join("ci.yml"),
        "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: lint\n        run: exit 3\n",
    )
    .unwrap();
    let cwd = dir.path().to_str().unwrap();

    let before = frame_in(cwd);
    assert!(before.ci.is_none());
    assert!(
        serde_json::to_value(&before).unwrap().get("ci").is_none(),
        "the key is absent for a repository without a run, so an older client sees no change"
    );

    let crate::ci_run::Check::Failed(_) = crate::ci_run::automatic_check(dir.path(), "goal") else {
        panic!("the plan should fail");
    };
    let after = serde_json::to_value(frame_in(cwd)).unwrap();
    assert_eq!(after["ci"]["runnable"], 1);
    assert_eq!(after["ci"]["steps"][0]["status"], "fail");
    assert_eq!(after["ci"]["blocked"]["name"], "lint");
    assert_eq!(after["ci"]["blocked"]["via"], "goal");
    assert!(after["ci"].get("running").is_none());
}
