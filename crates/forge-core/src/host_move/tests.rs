use super::test_support::fake_repo;
use super::*;

fn here() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    (tmp, root)
}

fn laptop(root: &str, repos: Vec<RepoRef>) -> HostFingerprint {
    HostFingerprint {
        host: "laptop".into(),
        machine_id: Some("laptop-id".into()),
        home: None,
        cwd: root.into(),
        repo_root: Some(root.into()),
        repos,
        ..Default::default()
    }
}

fn state(fingerprint: HostFingerprint) -> HostState {
    HostState {
        fingerprint,
        ..Default::default()
    }
}

#[test]
fn a_fresh_session_records_its_host_without_a_notice() {
    let (_tmp, root) = here();
    let current = HostFingerprint::capture(&root);
    let d = detect(
        None,
        current.clone(),
        std::iter::empty(),
        true,
        false,
        false,
    );
    assert_eq!(d.notice, None);
    assert_eq!(d.state.fingerprint, current);
}

#[test]
fn the_same_host_never_gets_a_notice_even_with_dead_paths_in_history() {
    let (_tmp, root) = here();
    let current = HostFingerprint::capture(&root);
    let stored = state(current.clone());
    let history = ["/nonexistent-a/x/y.rs /nonexistent-b/z/w.rs"];
    let d = detect(
        Some(&stored),
        current,
        history.into_iter(),
        false,
        false,
        false,
    );
    assert_eq!(d.notice, None);
}

#[test]
fn another_machine_gets_a_notice_with_a_remote_based_path_map() {
    let (_tmp, root) = here();
    let app = root.join("API/app");
    fake_repo(&app, "git@github.com:acme/app.git");
    fake_repo(&root.join("API/dash"), "https://github.com/acme/dash");
    let gone = root.join("laptop-only/API");
    let old_wt = gone.join("app-wt-feature");
    let previous = laptop(
        old_wt.to_str().unwrap(),
        vec![
            RepoRef {
                path: old_wt.display().to_string(),
                remotes: vec!["github.com/acme/app".into()],
            },
            RepoRef {
                path: gone.join("dash").display().to_string(),
                remotes: vec!["github.com/acme/dash".into()],
            },
        ],
    );
    let current = HostFingerprint::capture(&app);
    let d = detect(
        Some(&state(previous)),
        current,
        std::iter::empty(),
        false,
        false,
        false,
    );
    let notice = d.notice.expect("moved");
    assert!(notice.starts_with("[host] This session moved: it was last active on host `laptop`"));
    assert!(notice.contains(&format!(
        "{} → {}",
        gone.join("dash").display(),
        root.join("API/dash").display()
    )));
    assert!(notice.contains("same git remote github.com/acme/dash"));
    assert!(notice.contains("this session's workspace"));
    assert!(notice.contains("- repo root: "));
    assert!(notice.contains("Tool outputs in this session are real and trustworthy"));
    assert!(notice.contains("no need to audit earlier tool calls"));
    assert_eq!(d.state.stale.len(), 2);
    assert!(d.state.fingerprint.cwd.ends_with("API/app"));
}

#[test]
fn a_moved_cwd_on_the_same_machine_is_a_move() {
    let (_tmp, root) = here();
    std::fs::create_dir_all(root.join("new")).unwrap();
    let current = HostFingerprint {
        // Pin home and repo root so the result does not depend on whether TMPDIR sits under a
        // real $HOME that is itself a git checkout (capture walks up to the nearest `.git`).
        home: None,
        repo_root: None,
        ..HostFingerprint::capture(&root.join("new"))
    };
    let previous = HostFingerprint {
        cwd: root.join("old").display().to_string(),
        repo_root: None,
        ..current.clone()
    };
    let d = detect(
        Some(&state(previous)),
        current,
        std::iter::empty(),
        false,
        false,
        false,
    );
    assert!(d.notice.unwrap().contains("its workspace moved from"));
    assert_eq!(
        d.state.stale[0].to.as_deref(),
        Some(root.join("new").to_str().unwrap())
    );
}

#[test]
fn an_untracked_session_counts_as_moved_only_on_enough_missing_directories() {
    let (_tmp, root) = here();
    let app = root.join("API/app");
    fake_repo(&app, "https://github.com/acme/app");
    let current = HostFingerprint {
        home: Some(root.display().to_string()),
        ..HostFingerprint::capture(&app)
    };
    let one = format!("read {}/API/app-wt-x/src/main.rs", root.display());
    let d = detect(
        None,
        current.clone(),
        [one.as_str()].into_iter(),
        false,
        false,
        false,
    );
    assert_eq!(
        d.notice, None,
        "one missing directory is not evidence of a move"
    );

    let two = format!("{one} and {}/sg-pro/index.ts", root.display());
    let d = detect(
        None,
        current,
        [two.as_str()].into_iter(),
        false,
        false,
        false,
    );
    let notice = d.notice.expect("legacy move");
    assert!(notice.contains("predates host tracking"));
    assert!(notice.contains(&format!(
        "{}/API/app-wt-x → {}",
        root.display(),
        app.display()
    )));
    assert!(notice.contains("no equivalent here"));
}

#[test]
fn reground_forces_a_notice_on_an_unmoved_session() {
    let (_tmp, root) = here();
    let current = HostFingerprint::capture(&root);
    let d = detect(
        Some(&state(current.clone())),
        current,
        std::iter::empty(),
        false,
        true,
        false,
    );
    assert!(d.notice.unwrap().contains("(/reground)"));
}

#[test]
fn stale_entries_from_an_earlier_move_survive_later_resumes() {
    let (_tmp, root) = here();
    let current = HostFingerprint::capture(&root);
    let stale = StaleRoot {
        from: "/nonexistent-host-move/old".into(),
        to: Some(root.display().to_string()),
        via: "this session's workspace".into(),
        alternates: Vec::new(),
    };
    let stored = HostState {
        fingerprint: current.clone(),
        stale: vec![stale.clone()],
        local_repos: Vec::new(),
    };
    let d = detect(
        Some(&stored),
        current,
        std::iter::empty(),
        false,
        false,
        false,
    );
    assert_eq!(d.notice, None);
    assert_eq!(d.state.stale, vec![stale]);
}

#[test]
fn resume_relocates_a_missing_cwd_to_the_checkout_with_the_same_remote() {
    let (_tmp, root) = here();
    let local = root.join("server/app");
    fake_repo(&local, "https://github.com/acme/app.git");
    std::fs::create_dir_all(local.join("crates/core")).unwrap();
    let store = forge_store::Store::open_in_memory().unwrap();
    let gone = "/nonexistent-host-move/laptop/app-wt-x";
    let id = store
        .create_session(&format!("{gone}/crates/core"), "Default")
        .unwrap();
    store.set_session_worktree(&id, gone).unwrap();
    let previous = state(laptop(
        gone,
        vec![RepoRef {
            path: gone.into(),
            remotes: vec!["github.com/acme/app".into()],
        }],
    ));
    store
        .swap_session_host_state(&id, None, &serde_json::to_string(&previous).unwrap())
        .unwrap();
    let _cwd = crate::test_cwd_guard(&local);
    let resolved = resolve_resume_cwd(&store, &id, &format!("{gone}/crates/core"));
    assert_eq!(resolved, local.join("crates/core").display().to_string());
    assert_eq!(
        store.session_cwd(&id).unwrap().as_deref(),
        Some(resolved.as_str())
    );
    assert_eq!(store.session_worktree(&id).unwrap(), None);
    assert_eq!(resolve_resume_cwd(&store, &id, &resolved), resolved);
}

#[test]
fn the_notice_reassures_instead_of_spreading_blanket_distrust() {
    let (_tmp, root) = here();
    let current = HostFingerprint::capture(&root);
    let notice = plan::notice(None, &current, "it moved", &[]);
    assert!(notice.contains("Tool outputs in this session are real and trustworthy"));
    assert!(notice.contains("the session itself is fine"));
    let lower = notice.to_lowercase();
    for banned in [
        "never rely",
        "never claim",
        "do not trust",
        "cannot be trusted",
        "session is broken",
        "fake",
        "hallucinat",
    ] {
        assert!(
            !lower.contains(banned),
            "notice must not say {banned:?}: {notice}"
        );
    }
}
