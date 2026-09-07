//! The PTY path: `shell{pty:true}`, for programs that change behaviour when `isatty(1)` is true.
//!
//! Split from `shell.rs` because it shares almost nothing with the piped path — a different spawn
//! (portable-pty owns it), a different kill strategy (drop the master fd), a different capture
//! (the OS merges stdout and stderr), and, importantly, a different security posture: the Landlock
//! sandbox cannot reach a spawn it does not own, which is why `pty:true` is refused while the
//! sandbox is enabled. Keeping that caveat in its own file makes it harder to forget.

use std::time::Duration;

use super::{pty_kill_child, CAPTURE_CAP, MODEL_BUDGET};
use super::{shell_invocation, stream_text, truncate_for_model};
/// Execute `command` under a pseudo-terminal (PTY) so `isatty(stdout)` returns true in the child.
///
/// This is the opt-in path (`pty: true`). Key differences from [`run_command`]:
///
/// - Uses `portable-pty` to open a native PTY (Unix `openpty`, Windows ConPTY).
/// - Stdin is the slave end — the OS sees a real tty, but we do not write to it, so the child
///   receives EOF on any stdin read (programs prompting on stdin exit rather than hanging).
/// - Combined stdout+stderr comes from the PTY master (the OS merges both streams).
/// - **Sandbox**: the Landlock sandbox does NOT apply here. `portable-pty` owns the spawn and
///   does not expose a `pre_exec` hook. V1 limitation — see shell-sandbox docs.
/// - Timeout + kill: on timeout the master fd is dropped (closing the PTY) and the child PID is
///   killed with SIGKILL (Unix) / TerminateProcess (Windows) via a dedicated blocking task.
///   Dropping the master makes the reader's blocking `read()` return immediately (EIO/EOF),
///   so the reader task unblocks within milliseconds without needing cancellation.
///
/// Output format is identical to [`run_command`]: `shell: <status> in <ms>ms\n\n<body>`.
pub async fn run_command_pty(command: &str, cwd: &str, timeout_secs: u64) -> String {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::time::Instant;

    let start = Instant::now();
    let pty_system = native_pty_system();

    let pair = match pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => return format!("shell(pty): failed to open pty: {e}"),
    };

    let (shell, flag) = shell_invocation();
    let mut cb = CommandBuilder::new(shell);
    cb.arg(flag);
    cb.arg(command);
    cb.cwd(cwd);

    // Spawn into the slave end.
    let mut child = match pair.slave.spawn_command(cb) {
        Ok(c) => c,
        Err(e) => return format!("shell(pty): failed to spawn (cwd {cwd}): {e}"),
    };
    // Drop the slave fd after spawn — when the child exits the master side will see EOF.
    drop(pair.slave);

    // Clone a reader from the master before we need to move `pair.master` for the kill path.
    let mut master_reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = child.kill();
            return format!("shell(pty): failed to clone pty reader: {e}");
        }
    };

    // Read the PTY master in a blocking task. The loop exits on EOF or error (EIO after the
    // master fd is closed — which we trigger on timeout by dropping `pair.master`).
    let read_task: tokio::task::JoinHandle<(Vec<u8>, bool)> =
        tokio::task::spawn_blocking(move || {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let mut capped = false;
            loop {
                match std::io::Read::read(&mut master_reader, &mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        if buf.len() < CAPTURE_CAP {
                            let take = n.min(CAPTURE_CAP - buf.len());
                            buf.extend_from_slice(&tmp[..take]);
                            if buf.len() >= CAPTURE_CAP {
                                capped = true;
                            }
                        } else {
                            capped = true;
                        }
                    }
                    Err(_) => break, // EIO when master fd is closed
                }
            }
            (buf, capped)
        });

    // Wait for the child with a timeout.
    //
    // Kill strategy on timeout:
    //   1. Drop the PTY master — this sends HUP/EIO to the child and unblocks the reader task.
    //   2. Kill the child's OS process directly via its PID (SIGKILL on Unix).
    //
    // We cannot move `child` into `spawn_blocking` and also keep it accessible for killing,
    // so we wait on a channel: the blocking task sends the exit status back.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut child_for_kill = child;
    // Extract PID before moving into the blocking task for use in the kill path.
    let child_pid = child_for_kill.process_id();
    tokio::task::spawn_blocking(move || {
        let result = child_for_kill.wait();
        let _ = tx.send(result);
    });

    let (status_line, _exit_code) =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(Ok(status))) => {
                let code = status.exit_code();
                (
                    format!(
                        "exit {}",
                        if status.success() {
                            "0".to_string()
                        } else {
                            code.to_string()
                        }
                    ),
                    Some(code),
                )
            }
            Ok(Ok(Err(e))) => (format!("error: {e}"), None),
            Ok(Err(_)) => ("error: wait channel dropped".to_string(), None),
            Err(_) => {
                // Timeout: close the master (sends EIO to the reader task) then kill the process.
                drop(pair.master);
                pty_kill_child(child_pid);
                (format!("timed out after {timeout_secs}s (killed)"), None)
            }
        };

    let duration_ms = start.elapsed().as_millis();

    // The reader task unblocks as soon as the master is closed (on timeout) or the child exits
    // (normal path). Give it a short extra window to flush any buffered bytes.
    let (raw_bytes, capped) = tokio::time::timeout(Duration::from_secs(5), read_task)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();

    // PTY merges stdout+stderr; render the combined bytes as a single stream.
    let body = stream_text(&raw_bytes)
        .map(|s| {
            if s.trim().is_empty() {
                String::new()
            } else {
                s
            }
        })
        .unwrap_or_default();
    let (body, truncated) = truncate_for_model(&body, MODEL_BUDGET);
    let total = raw_bytes.len();
    let mut header = format!("shell: {status_line} in {duration_ms}ms");
    if truncated || capped {
        header.push_str(&format!("  ({total} bytes captured, output truncated)"));
    }
    if body.trim().is_empty() {
        header
    } else {
        format!("{header}\n\n{body}")
    }
}
