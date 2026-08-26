//! Smoke the two primary non-TUI entrypoints through the real `forge` binary.
//!
//! Each test uses the deterministic mock provider and a disposable store, so it exercises CLI
//! argument wiring, session construction, presenter setup, and one completed turn without network
//! access or touching the developer's real Forge database.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn run_stream_json_completes_a_real_mock_turn() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let store = workspace.path().join("forge.db");
    let output = Command::new(env!("CARGO_BIN_EXE_forge"))
        .args([
            "run",
            "say hello",
            "--mock",
            "--output-format",
            "stream-json",
        ])
        .current_dir(workspace.path())
        .env("FORGE_DB", &store)
        .output()
        .expect("spawn forge run");

    assert!(
        output.status.success(),
        "forge run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout
        .lines()
        .any(|line| line.contains("\"subtype\":\"init\"")));
    assert!(stdout
        .lines()
        .any(|line| line.contains("\"type\":\"result\"")));
    assert!(
        store.exists(),
        "the run must persist into the requested store"
    );
}

#[test]
fn chat_plain_completes_a_real_mock_turn_and_exits() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let store = workspace.path().join("forge.db");
    let mut child = Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["chat", "--mock", "--plain"])
        .current_dir(workspace.path())
        .env("FORGE_DB", &store)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn forge chat");
    child
        .stdin
        .take()
        .expect("chat stdin")
        .write_all(b"say hello\n/quit\n")
        .expect("send chat prompt");
    let output = child.wait_with_output().expect("wait for forge chat");

    assert!(
        output.status.success(),
        "forge chat failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("session"),
        "session banner missing: {stdout}"
    );
    assert!(
        stdout.contains("Done"),
        "mock turn did not complete: {stdout}"
    );
    assert!(
        store.exists(),
        "the chat must persist into the requested store"
    );
}
