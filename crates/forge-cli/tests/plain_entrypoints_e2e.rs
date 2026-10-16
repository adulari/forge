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
fn run_with_unfinished_tasks_fails_with_a_snake_case_stop_reason() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let store = workspace.path().join("forge.db");
    let output = Command::new(env!("CARGO_BIN_EXE_forge"))
        .args([
            "run",
            "mock:tasks track the work",
            "--mock",
            "--no-publish-to-fleet",
            "--output-format",
            "stream-json",
        ])
        .current_dir(workspace.path())
        .env("FORGE_DB", &store)
        .output()
        .expect("spawn forge run");
    assert!(!output.status.success(), "unfinished tasks must not exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = stdout
        .lines()
        .find(|line| line.contains("\"type\":\"result\""))
        .expect("a result event");
    assert!(
        result.contains("\"stop_reason\":\"tasks_unfinished\""),
        "{result}"
    );
}

#[cfg(unix)]
#[test]
fn a_new_store_is_not_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let store = workspace.path().join("forge.db");
    Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["run", "hi", "--mock", "--no-publish-to-fleet"])
        .current_dir(workspace.path())
        .env("FORGE_DB", &store)
        .output()
        .expect("spawn forge run");
    let mode = std::fs::metadata(&store).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
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

/// The bin under `CARGO_BIN_EXE_forge` is compiled with the `test-secrets` feature unified in and
/// uplifted to `target/debug/forge`. Its store is an empty in-memory map, so it MUST say so: a
/// bare "no keys configured" from it has been mistaken for a lost provider key more than once.
#[test]
fn test_built_binary_announces_its_blind_secret_store() {
    let output = Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["auth", "opencode_go", "--list"])
        .env_remove("OPENCODE_GO_API_KEY")
        .env_remove("OPENCODE_GO_API_KEY_2")
        .env_remove("OPENCODE_GO_API_KEY_3")
        .output()
        .expect("spawn forge auth");
    assert!(
        output.status.success(),
        "forge auth failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("reading keys from: in-memory test store"),
        "backend line missing: {stdout}"
    );
    assert!(stdout.contains("test-secrets"), "{stdout}");
    assert!(stdout.contains("cargo build --bin forge"), "{stdout}");
    assert!(
        stdout.contains("no opencode_go keys configured"),
        "the in-memory store must be empty: {stdout}"
    );
}
