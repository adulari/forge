use super::*;
use crate::ci_run::{automatic_check, make_plan, run_plan, Check, RunOptions};

fn repo(run: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
    write_workflow(dir.path(), run);
    dir
}

fn write_workflow(root: &Path, run: &str) {
    std::fs::write(
        root.join(".github/workflows/ci.yml"),
        format!(
            "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: lint\n        run: {run}\n      - name: later\n        run: echo later\n  w:\n    runs-on: windows-latest\n    steps:\n      - run: dir\n"
        ),
    )
    .unwrap();
}

#[test]
fn a_finished_run_is_visible_per_step_with_the_plan_size() {
    let dir = repo("echo fine");
    assert!(for_cwd(dir.path()).is_none(), "no run yet, nothing to show");
    let (plan, settings) = make_plan(dir.path());
    run_plan(
        dir.path(),
        &plan,
        &settings,
        &RunOptions::default(),
        &mut |_| {},
    )
    .unwrap();
    let ci = for_cwd(dir.path()).expect("the run is recorded");
    assert_eq!((ci.runnable, ci.skipped), (2, 1));
    assert_eq!(ci.ok, Some(true));
    assert!(ci.running.is_none() && ci.blocked.is_none());
    let words: Vec<_> = ci.steps.iter().map(|s| s.status).collect();
    assert_eq!(words, ["pass", "pass"]);
    assert_eq!(ci.steps[0].name, "lint");
}

#[test]
fn a_run_in_flight_names_the_running_step_and_a_session_subdirectory_finds_it() {
    let dir = repo("echo fine");
    begin(dir.path(), 2, 0);
    step_started(dir.path(), "lint", 1, 2);
    let ci = for_cwd(&dir.path().join("crates/x")).expect("a subdirectory resolves to the repo");
    let running = ci.running.expect("a step is running");
    assert_eq!(
        (running.name.as_str(), running.index, running.total),
        ("lint", 1, 2)
    );
    assert!(ci.steps.is_empty() && ci.ok.is_none());
}

#[test]
fn a_failing_gate_names_the_blocking_step_until_a_later_run_passes() {
    let dir = repo("echo boom >&2; exit 3");
    let Check::Failed(_) = automatic_check(dir.path(), "goal") else {
        panic!("the plan should fail");
    };
    let ci = for_cwd(dir.path()).unwrap();
    let block = ci.blocked.expect("the gate blocked on the failing step");
    assert_eq!((block.name.as_str(), block.via), ("lint", "goal"));
    let statuses: Vec<_> = ci.steps.iter().map(|s| s.status).collect();
    assert_eq!(statuses, ["fail", "not_run"]);
    assert_eq!(ci.ok, Some(false));

    write_workflow(dir.path(), "echo fixed");
    let Check::Passed(_) = automatic_check(dir.path(), "push") else {
        panic!("the fixed plan should pass");
    };
    assert!(for_cwd(dir.path()).unwrap().blocked.is_none());
}

#[test]
fn a_saved_run_is_shown_after_a_restart_and_the_wire_form_is_additive() {
    let dir = repo("echo fine");
    let (plan, settings) = make_plan(dir.path());
    let report = run_plan(
        dir.path(),
        &plan,
        &settings,
        &RunOptions::default(),
        &mut |_| {},
    )
    .unwrap();
    let restored = from_report(&report);
    assert_eq!(restored.runnable, 2);
    assert_eq!(restored.ok, Some(true));
    let json = serde_json::to_value(&restored).unwrap();
    assert!(json.get("running").is_none() && json.get("blocked").is_none());
    assert_eq!(json["steps"][0]["status"], "pass");
}

#[test]
fn the_watch_reports_each_change_once() {
    let mut watch = Watch::default();
    watch.changed();
    assert!(!watch.changed());
    let dir = repo("echo fine");
    begin(dir.path(), 1, 0);
    assert!(watch.changed());
    assert!(!watch.changed());
}
