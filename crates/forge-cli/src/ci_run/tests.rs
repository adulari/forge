use super::*;

fn repo(workflow: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
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
    let report = run_plan(root, &plan, &settings, opts, &mut |_| {});
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
    );
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
    std::fs::remove_file(dir.path().join(".forge/ci.toml")).unwrap();
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
