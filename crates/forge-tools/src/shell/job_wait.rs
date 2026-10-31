//! `shell_job {action: "wait"}`: block up to N seconds for a background job to exit and return its
//! exit code with the tail of its log, so the model waits once instead of polling `log`.

use std::path::Path;
use std::time::{Duration, Instant};

use super::background::{get, note_waiting, tail, take_stopping, Job};

pub(super) const DEFAULT_WAIT_SECS: u64 = 60;
pub(super) const MAX_WAIT_SECS: u64 = 600;
const POLL: Duration = Duration::from_millis(100);
/// The exit code is written by the task that reaps the child, a beat after the pid disappears.
const EXIT_GRACE: Duration = Duration::from_millis(500);
const TAIL_LINES: usize = 30;

pub(super) async fn wait_for_job(
    workspace: Option<&Path>,
    cwd: &str,
    id: u32,
    timeout_secs: u64,
) -> Result<String, String> {
    let Some(job) = get(workspace, cwd, id) else {
        return Err(format!("no background job {id} in this workspace"));
    };
    let timeout = Duration::from_secs(timeout_secs.clamp(1, MAX_WAIT_SECS));
    if !job.is_running() {
        return Ok(report(&job, Duration::ZERO, false));
    }
    // This reply is the job's exit notice, so the session's job-wake must not announce it again.
    note_waiting(id);
    let started = Instant::now();
    let mut latest = job;
    while started.elapsed() < timeout && latest.is_running() {
        tokio::time::sleep(POLL).await;
        if let Some(job) = get(workspace, cwd, id) {
            latest = job;
        }
    }
    if !latest.is_running() && latest.exit_code().is_none() {
        let grace = Instant::now();
        while grace.elapsed() < EXIT_GRACE && latest.exit_code().is_none() {
            tokio::time::sleep(POLL).await;
            if let Some(job) = get(workspace, cwd, id) {
                latest = job;
            }
        }
    }
    // Still running: later exit must be announced as usual.
    if latest.is_running() {
        take_stopping(id);
    }
    Ok(report(&latest, started.elapsed(), latest.is_running()))
}

fn report(job: &Job, waited: Duration, timed_out: bool) -> String {
    let body = tail(&job.log, TAIL_LINES).unwrap_or_default();
    let body = if body.trim().is_empty() {
        "(no output)".to_string()
    } else {
        body
    };
    let head = if timed_out {
        format!(
            "job {} is still running after {}s; call wait again, or read its log",
            job.pid,
            waited.as_secs()
        )
    } else {
        format!(
            "job {} {} after {:.1}s",
            job.pid,
            job.status(),
            waited.as_secs_f64()
        )
    };
    format!("{head} — last {TAIL_LINES} lines:\n\n{body}")
}
