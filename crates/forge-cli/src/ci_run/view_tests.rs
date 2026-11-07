use forge_config::ci_parity::plan::{CiSkip, CiStep};
use forge_config::ci_parity::report::StepResult;

use super::*;

fn step(name: &str) -> CiStep {
    CiStep {
        key: format!("ci.yml::lint::{name}"),
        workflow: "ci.yml".into(),
        job: "lint".into(),
        name: name.into(),
        run: "cargo clippy\n".into(),
        env: Default::default(),
        cwd: None,
        shell: None,
        timeout_secs: None,
        allow_failure: false,
    }
}

#[test]
fn rows_pair_steps_with_the_last_result_and_list_skips_with_their_reason() {
    let plan = CiPlan {
        steps: vec![step("clippy"), step("fmt"), step("test")],
        skips: vec![CiSkip {
            workflow: "ci.yml".into(),
            job: Some("win".into()),
            step: None,
            reason: "runs on windows-latest (not Linux)".into(),
            candidate: None,
        }],
        ..CiPlan::default()
    };
    let result = |name: &str, status| StepResult {
        key: format!("ci.yml::lint::{name}"),
        name: name.into(),
        command: String::new(),
        status,
        secs: 4,
        tail: String::new(),
    };
    let report = CiReport {
        results: vec![
            result("clippy", StepStatus::Fail),
            result("fmt", StepStatus::NotRun),
        ],
        ..CiReport::default()
    };
    let rows = rows_for(&plan, Some(&report));
    let states: Vec<_> = rows.iter().map(|r| r.state).collect();
    assert_eq!(
        states,
        [
            ViewState::Fail,
            ViewState::NotRun,
            ViewState::Pending,
            ViewState::Skip
        ]
    );
    assert_eq!(rows[0].detail, "cargo clippy");
    assert_eq!(rows[3].name, "win");
    assert!(rows[3].detail.contains("windows-latest"));
}
