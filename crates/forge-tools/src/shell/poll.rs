//! Poll mode: re-run a status command until it succeeds, the per-call budget runs out, or a
//! background job the poll is waiting on ends.

use std::time::{Duration, Instant};

use super::{background, run_command_inner};
use crate::sandbox::SandboxPolicy;

/// The background jobs a poll waits on.
pub(super) struct PollJobs<'a> {
    pub(super) workspace: Option<&'a std::path::Path>,
    pub(super) pids: &'a [u32],
}

/// Background jobs a poll is waiting on: the running jobs whose log path or pid the command
/// names, or the only running job when it names none. A poll on a build's log kept waiting for
/// three more rounds after the build had died, because nothing told it the job was gone.
pub(super) fn watched_jobs(
    command: &str,
    workspace: Option<&std::path::Path>,
    cwd: &str,
) -> Vec<u32> {
    let running: Vec<background::Job> = background::list(workspace, cwd)
        .into_iter()
        .filter(background::Job::is_running)
        .collect();
    let named: Vec<u32> = running
        .iter()
        .filter(|job| {
            command.contains(job.log.to_string_lossy().as_ref())
                || command.contains(&job.pid.to_string())
        })
        .map(|job| job.pid)
        .collect();
    match (named.is_empty(), running.as_slice()) {
        (false, _) => named,
        (true, [only]) => vec![only.pid],
        _ => Vec::new(),
    }
}

/// A watched job that has ended, as the poll's result: the condition it waits for may never come.
fn ended_job(cwd: &str, jobs: &PollJobs<'_>, elapsed: u64) -> Option<String> {
    let job = jobs.pids.iter().find_map(|pid| {
        background::get(jobs.workspace, cwd, *pid).filter(|job| !job.is_running())
    })?;
    let how = match job.exit_code() {
        Some(-1) => "was killed (no exit code: a signal ended it)".to_string(),
        Some(code) => format!("exited {code}"),
        None => "is gone".to_string(),
    };
    let tail = background::tail(&job.log, 8).unwrap_or_default();
    Some(format!(
        "shell: stopped waiting after {elapsed}s — background job {} (`{}`) {how} while this \
         poll ran, so what it waits for may never happen. Check the job (shell_job log) before \
         polling again.\n\n{}",
        job.pid,
        job.command,
        tail.trim_end()
    ))
}

pub(super) async fn run_poll(
    command: &str,
    cwd: &str,
    budget_secs: u64,
    interval_secs: u64,
    policy: &SandboxPolicy,
    jobs: PollJobs<'_>,
) -> String {
    let budget = Duration::from_secs(budget_secs);
    let interval = Duration::from_secs(interval_secs);
    let start = Instant::now();
    loop {
        // Give each attempt the remaining budget as its own timeout so a single hung attempt
        // can never outlive the poll call.
        let attempt_timeout = budget.saturating_sub(start.elapsed()).as_secs().max(1);
        let (result, code, _) = run_command_inner(command, cwd, attempt_timeout, policy).await;
        let elapsed = start.elapsed().as_secs();
        let body = result.split_once("\n\n").map(|(_, b)| b).unwrap_or("");
        if code == Some(0) {
            return if body.trim().is_empty() {
                format!("shell: ready after {elapsed}s (exit 0)")
            } else {
                format!("shell: ready after {elapsed}s (exit 0)\n\n{body}")
            };
        }
        if let Some(ended) = ended_job(cwd, &jobs, elapsed) {
            return ended;
        }
        // Not ready yet. If the next sleep would cross the budget, yield a resumable result so
        // the model can keep waiting by calling again — never the hard killed-timeout path.
        if start.elapsed() + interval >= budget {
            let code_str = code
                .map(|c| format!("exit {c}"))
                .unwrap_or_else(|| "no exit".into());
            let tail = if body.trim().is_empty() {
                String::new()
            } else {
                format!("\n\n{body}")
            };
            return format!(
                "shell: not ready after {elapsed}s (last {code_str}) — call shell again with the \
                 same poll args to keep waiting.{tail}"
            );
        }
        tokio::time::sleep(interval).await;
    }
}
