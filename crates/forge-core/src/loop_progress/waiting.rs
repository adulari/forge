//! Calls that wait for something outside the turn, which is neither progress nor a loop.
//!
//! The harness tells a model to wait for a background job, a CI run or a server by polling, and
//! polling returns the same answer until the thing is ready. Judging each such answer on its own
//! would stop the turn for doing exactly what it was told, so a waiting call is set aside: the
//! ledger neither counts it as stale nor lets it start a cycle. How long a turn may wait is the
//! waiting lane's business, not this file's.

use std::sync::OnceLock;

/// Whether `call`, which returned `result`, was a wait.
///
/// * `shell` in poll mode that came back `not ready`: the sanctioned way to keep waiting.
/// * `shell` that hit its time limit: the command was still running when the cap cut it, and
///   calling it again is how the model keeps watching a run longer than the cap.
/// * a successful `shell` whose command sleeps or watches first (`sleep 60; gh run view`).
/// * `shell_job` status or log of a job that is still running.
pub(super) fn is_waiting(call: &forge_types::ToolCall, ok: bool, result: &str) -> bool {
    let head = result.trim_start();
    match call.name.as_str() {
        "shell" => {
            head.starts_with("shell: not ready after")
                || head.starts_with("shell: timed out after")
                || (ok && sleeps_first(call))
        }
        "shell_job" => head
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("job ") && line.contains(" is running")),
        _ => false,
    }
}

fn sleeps_first(call: &forge_types::ToolCall) -> bool {
    static SLEEPS: OnceLock<regex::Regex> = OnceLock::new();
    let Some(text) = forge_types::shell_command_text(&call.args) else {
        return false;
    };
    SLEEPS
        .get_or_init(|| {
            regex::Regex::new(
                r"(^|[\s;&|(])(sleep\s+[0-9]|watch\s|until\s)|\b(run|pr|workflow)\s+watch\b",
            )
            .expect("sleep pattern")
        })
        .is_match(&text)
}
