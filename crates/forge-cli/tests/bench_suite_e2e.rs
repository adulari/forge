//! `forge bench tasks` over the example suite with the mock provider, then `forge bench export`
//! over its metrics, and `forge run --usage-json` — all through the real binary against a
//! disposable store and config dir.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn forge(workspace: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    cmd.current_dir(workspace)
        .env("FORGE_DB", workspace.join("forge.db"))
        .env("XDG_CONFIG_HOME", workspace.join("config"))
        .env("XDG_DATA_HOME", workspace.join("data"))
        .env("XDG_STATE_HOME", workspace.join("state"))
        .env("XDG_CACHE_HOME", workspace.join("cache"));
    cmd
}

fn run_ok(cmd: &mut Command) -> String {
    let output = cmd.output().expect("spawn forge");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "forge failed: {stderr}");
    stderr
}

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bench-suite")
}

#[test]
fn bench_tasks_runs_the_example_suite_and_export_writes_redacted_trajectories() {
    let tmp = tempfile::tempdir().expect("temp workspace");
    let ws = tmp.path();
    // A relative --out, like the default: the scripts run with the task workspace as CWD, so
    // the log and result paths must not resolve against it.
    run_ok(forge(ws).args([
        "bench",
        "tasks",
        "--suite",
        suite().to_str().unwrap(),
        "--mock",
        "--timeout-secs",
        "120",
        "--out",
        "tasks.jsonl",
    ]));

    let eval: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ws.join("tasks.eval.json")).unwrap())
            .unwrap();
    assert_eq!(eval["resolved_ids"], serde_json::json!(["write-note"]));
    assert_eq!(eval["unresolved_ids"], serde_json::json!(["needs-fix"]));

    let metrics_file = ws.join("tasks.metrics.jsonl");
    let metrics: Vec<serde_json::Value> = std::fs::read_to_string(&metrics_file)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(metrics.len(), 2);
    for m in &metrics {
        assert!(m["session_id"].is_string(), "{m}");
        assert_eq!(m["attempt"], 1);
        assert!(m["model_calls"].as_u64().unwrap() >= 1, "{m}");
    }
    assert!(ws.join("tasks.logs/write-note.attempt1.setup.log").exists());
    assert!(ws.join("tasks.logs/write-note.attempt1.test.log").exists());

    let traj = ws.join("traj");
    run_ok(forge(ws).args([
        "bench",
        "export",
        "--metrics",
        metrics_file.to_str().unwrap(),
        "--out",
        traj.to_str().unwrap(),
    ]));
    let body = std::fs::read_to_string(traj.join("write-note.attempt1.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = body
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["type"], "attempt");
    assert_eq!(lines[0]["task"], "write-note");
    assert_eq!(lines[0]["trajectory"], "forge_session");
    assert!(lines.len() > 2, "steps follow the header: {body}");
    assert!(
        lines[1..].iter().any(|l| l["tool_calls"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c["name"] == "write_file"))),
        "the mock's write_file call is in the trajectory: {body}"
    );
    assert!(traj.join("needs-fix.attempt1.jsonl").exists());
}

#[test]
fn run_usage_json_reports_the_sessions_consumption() {
    let tmp = tempfile::tempdir().expect("temp workspace");
    let ws = tmp.path();
    let usage = ws.join("usage.json");
    run_ok(forge(ws).args([
        "run",
        "say hello",
        "--mock",
        "--no-publish-to-fleet",
        "--usage-json",
        usage.to_str().unwrap(),
    ]));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&usage).unwrap()).expect("usage JSON parses");
    assert!(v["session_id"].is_string(), "{v}");
    assert!(v["model_calls"].as_u64().unwrap() >= 1, "{v}");
    assert!(v["input_tokens"].as_u64().unwrap() > 0, "{v}");
    assert!(v["wall_secs"].as_f64().is_some(), "{v}");
}
