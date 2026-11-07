use super::*;
use crate::ci_parity::plan::CiStep;
use std::collections::BTreeMap;

fn step(job: &str, name: &str) -> CiStep {
    CiStep {
        key: format!("ci.yml::{job}::{name}"),
        workflow: "ci.yml".into(),
        job: job.into(),
        name: name.into(),
        run: "true".into(),
        env: BTreeMap::new(),
        cwd: None,
        shell: None,
        timeout_secs: None,
        allow_failure: false,
    }
}

#[test]
fn defaults_and_parsing() {
    assert_eq!(parse_settings("x = 1").unwrap(), None);
    let s = parse_settings("[ci]\ngate = \"off\"\nstep_timeout_minutes = 5\ntotal_budget_minutes = 9\nexclude = [\"a\"]\n")
        .unwrap()
        .unwrap();
    assert_eq!(s.gate, GateMode::Off);
    assert_eq!((s.step_timeout_secs, s.total_budget_secs), (300, 540));
    assert_eq!(s.exclude, ["a"]);
    assert_eq!(CiSettings::default().step_timeout_secs, 1800);
    assert!(parse_settings("[ci]\ngate = \"maybe\"\n").is_err());
}

#[test]
fn a_bad_file_falls_back_to_defaults_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".forge")).unwrap();
    std::fs::write(dir.path().join(".forge/ci.toml"), "[ci\n").unwrap();
    let s = load_settings(dir.path());
    assert_eq!(s.gate, GateMode::Auto);
    assert!(s.warning.is_some());
}

#[test]
fn exclude_drops_and_include_restores() {
    let mut plan = CiPlan::default();
    plan.steps.push(step("test", "slow"));
    plan.steps.push(step("fmt", "fmt"));
    plan.skips.push(CiSkip {
        workflow: "ci.yml".into(),
        job: Some("docs".into()),
        step: Some("Docs freshness".into()),
        reason: "env X: needs GitHub context".into(),
        candidate: Some(step("docs", "Docs freshness")),
    });
    let settings = CiSettings {
        exclude: vec!["ci.yml::test::*".into()],
        include: vec!["Docs freshness".into()],
        ..CiSettings::default()
    };
    apply_overrides(&mut plan, &settings);
    let names: Vec<_> = plan.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["fmt", "Docs freshness"]);
    assert_eq!(plan.skips.len(), 1);
    assert_eq!(plan.skips[0].reason, "excluded by .forge/ci.toml");
}
