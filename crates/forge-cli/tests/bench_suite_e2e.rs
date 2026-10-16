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

#[test]
fn bench_recall_runs_the_scenario_end_to_end_against_the_mock_provider() {
    let tmp = tempfile::tempdir().expect("temp workspace");
    let ws = tmp.path();
    let output = forge(ws)
        .args(["bench", "recall", "--mock", "--json", "--turns-scale", "0"])
        .output()
        .expect("spawn forge");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "forge failed: {stderr}");
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is the JSON report ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(v["model"], "mock");
    assert_eq!(v["resumed"], true, "{stderr}");
    assert_eq!(v["compactions"].as_array().unwrap().len(), 2);
    assert!(
        v["compactions"][0]["messages_after"].as_u64().unwrap()
            < v["compactions"][0]["messages_before"].as_u64().unwrap(),
        "{v}"
    );
    let total = v["after_compaction"]["total"]["total"].as_u64().unwrap();
    assert!(total >= 15, "one probe per planted fact: {v}");
    assert_eq!(v["after_resume"]["total"]["total"], total);
    assert!(v["usage"]["model_calls"].as_u64().unwrap() > 0, "{v}");
    assert!(v["session_id"].is_string());
    assert!(
        !ws.join("config").join("forge").join("config.toml").exists(),
        "the bench uses its own temp config, never the caller's"
    );
}

#[test]
fn bench_recall_hard_scenario_runs_against_the_mock_and_reports_compaction_stats() {
    let tmp = tempfile::tempdir().expect("temp workspace");
    let output = forge(tmp.path())
        .args([
            "bench",
            "recall",
            "--scenario",
            "hard",
            "--mock",
            "--json",
            "--skip-resume",
            "--turns-scale",
            "0",
        ])
        .output()
        .expect("spawn forge");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "forge failed: {stderr}");
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(v["scenario"], "hard");
    assert!(v.get("after_resume").is_none());
    assert_eq!(v["compaction_stats"]["manual"], 2, "{v}");
    assert!(v["compaction_stats"]["auto"].is_u64(), "{v}");
    let ids: Vec<&str> = v["probes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    for expected in [
        "work_state_files",
        "work_state_pending",
        "region_quota",
        "tasks_open",
        "obey_agents_doc_rule",
    ] {
        assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
    }
}
