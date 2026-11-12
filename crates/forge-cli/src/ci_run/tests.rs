use super::*;

fn repo(workflow: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
    crate::ci_run::opt_in_ci_gate(dir.path());
    std::fs::write(wf.join("ci.yml"), workflow).unwrap();
    std::fs::write(
        dir.path().join("check.sh"),
        "#!/bin/sh\nexit ${CHECK_EXIT:-0}\n",
    )
    .unwrap();
    dir
}

const WF: &str = "name: CI\non:\n  pull_request:\nenv:\n  STRICT: \"1\"\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - name: Strict check\n        run: test \"$STRICT\" = 1 && sh ./check.sh\n      - name: Needs a secret\n        env:\n          T: ${{ secrets.T }}\n        run: echo $T\n  win:\n    runs-on: windows-latest\n    steps:\n      - run: echo hi\n";

fn run(root: &Path, opts: &RunOptions) -> (CiPlan, CiReport) {
    let (plan, settings) = make_plan(root);
    let report = run_plan(root, &plan, &settings, opts, &mut |_| {}).unwrap();
    (plan, report)
}

#[test]
fn the_plan_applies_workflow_env_and_passes() {
    let dir = repo(WF);
    let (plan, report) = run(dir.path(), &RunOptions::default());
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.skips.len(), 2);
    assert!(report.ok(), "{report:?}");
    assert_eq!(report.results[0].status, StepStatus::Pass);
    assert!(report.skipped.iter().any(|s| s.contains("needs secret T")));
}

#[test]
fn a_failing_step_is_named_with_command_and_output_tail() {
    let dir = repo(&WF.replace(
        "sh ./check.sh",
        "echo boom-output >&2; sh ./check.sh; exit 3",
    ));
    let (plan, report) = run(dir.path(), &RunOptions::default());
    assert!(!report.ok());
    let text = failure_text(&plan, &report);
    assert!(
        text.contains("Local CI step failed: Strict check"),
        "{text}"
    );
    assert!(
        text.contains("STRICT=1") && text.contains("boom-output") && text.contains("exit"),
        "{text}"
    );
}

#[test]
fn passing_steps_are_not_rerun_on_an_unchanged_tree_unless_fresh() {
    let dir = repo(WF);
    let marker = dir.path().join("ran");
    let wf = WF.replace("sh ./check.sh", &format!("echo x >> {}", marker.display()));
    std::fs::write(dir.path().join(".github/workflows/ci.yml"), wf).unwrap();
    // The marker file sits in the tree, so ignore it to keep the fingerprint stable.
    std::fs::write(dir.path().join(".gitignore"), "ran\n").unwrap();
    for args in [
        &["init", "-q"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "i",
        ],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    run(dir.path(), &RunOptions::default());
    let (_, second) = run(dir.path(), &RunOptions::default());
    assert_eq!(second.results[0].status, StepStatus::Cached);
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
    run(
        dir.path(),
        &RunOptions {
            fresh: true,
            ..RunOptions::default()
        },
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);
}

#[test]
fn a_step_that_outruns_its_timeout_is_killed_and_reported() {
    let dir = repo(&WF.replace("test \"$STRICT\" = 1 && sh ./check.sh", "sleep 30"));
    std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
    std::fs::write(
        dir.path().join(".forge/ci.toml"),
        "[ci]\nstep_timeout_minutes = 1\n",
    )
    .unwrap();
    let (plan, settings) = make_plan(dir.path());
    let mut short = settings.clone();
    short.step_timeout_secs = 1;
    let report = run_plan(
        dir.path(),
        &plan,
        &short,
        &RunOptions::default(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.results[0].status, StepStatus::Timeout);
    assert!(report.results[0].tail.contains("timed out"));
}

#[test]
fn fail_fast_marks_later_steps_not_run_and_keep_going_runs_them() {
    let two = "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: one\n        run: exit 1\n      - name: two\n        run: echo ok\n";
    let dir = repo(two);
    let (_, ff) = run(dir.path(), &RunOptions::default());
    assert_eq!(ff.results[1].status, StepStatus::NotRun);
    let (_, kg) = run(
        dir.path(),
        &RunOptions {
            keep_going: true,
            ..RunOptions::default()
        },
    );
    assert_eq!(kg.results[1].status, StepStatus::Pass);
}

#[test]
fn automatic_check_respects_settings_and_empty_plans() {
    let dir = repo(WF);
    std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
    std::fs::write(dir.path().join(".forge/ci.toml"), "[ci]\ngate = \"off\"\n").unwrap();
    assert!(matches!(
        automatic_check(dir.path(), "push"),
        Check::Skipped(_)
    ));
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        automatic_check(empty.path(), "push"),
        Check::Skipped(_)
    ));
    crate::ci_run::opt_in_ci_gate(dir.path());
    assert!(matches!(
        automatic_check(dir.path(), "push"),
        Check::Passed(_)
    ));
}

#[test]
fn a_missing_tool_is_skipped_not_failed() {
    let wf = "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: audit\n        run: definitely-not-installed-xyz --check\n      - name: ok\n        run: echo fine\n";
    let dir = repo(wf);
    let (_, report) = run(dir.path(), &RunOptions::default());
    assert_eq!(report.results[0].status, StepStatus::Skipped);
    assert_eq!(report.results[1].status, StepStatus::Pass);
    assert!(report.ok());
}

#[test]
fn a_plan_whose_every_step_is_not_installed_is_skipped_not_failed() {
    let wf = "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: test\n        run: definitely-not-installed-xyz test\n";
    let dir = repo(wf);
    match automatic_check(dir.path(), "push") {
        Check::Skipped(why) => assert!(why.contains("definitely-not-installed-xyz test"), "{why}"),
        Check::Failed(why) => panic!("blocked on a plan where nothing ran: {why}"),
        Check::Passed(why) => panic!("claimed a pass when nothing ran: {why}"),
    }
    assert!(push_blocker(dir.path()).is_none());
}

fn wf(cmd: &str) -> String {
    format!("on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: lint\n        run: {cmd}\n")
}

#[test]
fn a_second_run_on_a_running_root_is_refused_and_does_not_start() {
    let dir = repo(&wf("echo fine"));
    let (plan, settings) = make_plan(dir.path());
    let held = live::acquire(dir.path()).unwrap();
    live::begin(dir.path(), 2, 0);
    live::step_started(dir.path(), "lint", 1, 2);
    let err = run_plan(
        dir.path(),
        &plan,
        &settings,
        &RunOptions::default(),
        &mut |_| panic!("a refused run must not start a step"),
    )
    .unwrap_err();
    assert_eq!(
        err,
        "a local CI run is already in progress on this worktree (step 1/2: lint)"
    );
    assert!(matches!(
        automatic_check(dir.path(), "push"),
        Check::Failed(m) if m.contains("already in progress")
    ));
    drop(held);
    assert!(run_plan(
        dir.path(),
        &plan,
        &settings,
        &RunOptions::default(),
        &mut |_| {}
    )
    .is_ok());
}

#[test]
fn two_concurrent_runs_on_one_root_run_exactly_once() {
    let dir = repo(&wf("echo fine"));
    let (plan, settings) = make_plan(dir.path());
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let first = std::thread::scope(|s| {
        let (root, plan_ref, settings_ref) = (dir.path(), &plan, &settings);
        let first = s.spawn(move || {
            run_plan(
                root,
                plan_ref,
                settings_ref,
                &RunOptions::default(),
                &mut |e| {
                    if matches!(e, Event::Start { .. }) {
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    }
                },
            )
        });
        started_rx.recv().unwrap();
        let second = run_plan(
            dir.path(),
            &plan,
            &settings,
            &RunOptions::default(),
            &mut |_| panic!("the second run must not start a step"),
        );
        release_tx.send(()).unwrap();
        let refused = second.unwrap_err();
        assert!(
            refused.contains("already in progress") && refused.contains("step 1/1: lint"),
            "{refused}"
        );
        first.join().unwrap()
    });
    assert!(first.unwrap().ok());
}

#[test]
fn the_root_is_freed_after_a_failing_or_panicking_run() {
    let dir = repo(&wf("exit 1"));
    let (plan, settings) = make_plan(dir.path());
    let failed = run_plan(
        dir.path(),
        &plan,
        &settings,
        &RunOptions::default(),
        &mut |_| {},
    )
    .unwrap();
    assert!(!failed.ok());
    let panicked = std::thread::scope(|s| {
        s.spawn(|| {
            let _ = run_plan(
                dir.path(),
                &plan,
                &settings,
                &RunOptions::default(),
                &mut |e| {
                    if matches!(e, Event::Start { .. }) {
                        panic!("boom");
                    }
                },
            );
        })
        .join()
    });
    assert!(panicked.is_err());
    assert!(live::for_cwd(dir.path()).unwrap().running.is_none());
    assert!(run_plan(
        dir.path(),
        &plan,
        &settings,
        &RunOptions::default(),
        &mut |_| {}
    )
    .is_ok());
}
