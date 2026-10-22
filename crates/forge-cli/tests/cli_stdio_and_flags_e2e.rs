//! Output-stream failures and `claude -p` compatibility flags, through the real `forge` binary.

use std::process::{Command, Stdio};

fn forge(workspace: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_forge"));
    command
        .current_dir(workspace)
        .env("FORGE_DB", workspace.join("forge.db"))
        .env("XDG_CONFIG_HOME", workspace.join("cfg"))
        .env("XDG_DATA_HOME", workspace.join("data"))
        .env("HOME", workspace);
    command
}

#[cfg(unix)]
#[test]
fn a_full_disk_on_stdout_is_one_error_line_not_a_panic() {
    let workspace = tempfile::tempdir().unwrap();
    let full = std::fs::OpenOptions::new().write(true).open("/dev/full");
    let Ok(full) = full else { return };
    let output = forge(workspace.path())
        .stdout(full)
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("failed writing to stdout"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn a_reader_that_went_away_ends_the_command_silently_with_141() {
    let workspace = tempfile::tempdir().unwrap();
    let mut child = forge(workspace.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(
        matches!(output.status.code(), Some(141) | Some(0)),
        "{:?} {stderr}",
        output.status
    );
}

#[test]
fn run_accepts_the_claude_print_flags() {
    let workspace = tempfile::tempdir().unwrap();
    let output = forge(workspace.path())
        .args([
            "run",
            "-p",
            "say hello",
            "--mock",
            "--no-publish-to-fleet",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "text",
            "--permission-mode",
            "acceptEdits",
            "--max-turns",
            "3",
            "--allowedTools",
            "Bash(git log:*) Read",
            "--disallowedTools",
            "Write",
            "--append-system-prompt",
            "be brief",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"type\":\"result\""));
}

#[test]
fn unsupported_claude_flags_are_rejected_not_ignored() {
    let workspace = tempfile::tempdir().unwrap();
    for flag in ["--fork-session", "--json-schema", "--settings"] {
        let output = forge(workspace.path())
            .args(["run", "hi", "--mock", flag, "x"])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{flag} must not be silently accepted"
        );
    }
}

#[test]
fn conflicting_permission_flags_are_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    let output = forge(workspace.path())
        .args([
            "run",
            "hi",
            "--mock",
            "--mode",
            "plan",
            "--permission-mode",
            "plan",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
