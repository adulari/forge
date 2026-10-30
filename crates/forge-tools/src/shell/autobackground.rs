//! `[harness] shell_autobackground`: a foreground command that is still running after the window
//! becomes a background job instead of blocking the turn (and instead of being killed at the
//! timeout). The command starts as a detached job writing to a log, which is what lets it
//! outlive the call; if it finishes inside the window the result reads exactly like a foreground
//! run and the job leaves no trace. Otherwise the call returns the job id and the output so far,
//! and the session's `job_wake` inbox delivers the exit, so the model does not poll.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::sandbox::SandboxPolicy;

use super::background::{self, release_inline};

pub(super) const WINDOW_SECS: u64 = 60;
const POLL: Duration = Duration::from_millis(100);
/// Time allowed for the exit watcher to record the code after the process has gone.
const RECORD_GRACE: Duration = Duration::from_millis(1500);
const TAIL_LINES: usize = 40;

impl super::ShellTool {
    /// A long-timeout plain command may outlive the turn: start it as a job and give it the
    /// window. A sandboxed run stays foreground, since a job does not carry the scoped build
    /// target.
    pub(super) async fn autobackground(
        &self,
        command: &str,
        cwd: &str,
        workspace: Option<&Path>,
        timeout_secs: u64,
    ) -> Option<crate::ToolRun> {
        if !crate::harness::shell_autobackground()
            || timeout_secs <= WINDOW_SECS
            || self.policy.enabled
        {
            return None;
        }
        let command = self
            .rtk
            .as_ref()
            .and_then(|r| r.rewrite(command))
            .unwrap_or_else(|| command.to_string());
        let window = Duration::from_secs(WINDOW_SECS);
        let out = run(&command, cwd, workspace, &self.policy, window).await;
        Some(crate::ToolRun::from(out))
    }
}

pub(super) async fn run(
    command: &str,
    cwd: &str,
    workspace: Option<&Path>,
    policy: &SandboxPolicy,
    window: Duration,
) -> String {
    let started = Instant::now();
    let job = match background::start_with(command, cwd, workspace, policy, true).await {
        Ok(job) => job,
        Err(error) => return format!("shell: failed to start (cwd {cwd}): {error}"),
    };
    let pid = job.pid;
    let mut gone_since: Option<Instant> = None;
    loop {
        let current = background::get(workspace, cwd, pid);
        let exit = current.as_ref().and_then(background::Job::exit_code);
        let alive = current.as_ref().is_some_and(background::Job::is_running);
        if let Some(code) = exit {
            release_inline(pid);
            return finish(&job, workspace, cwd, code, started.elapsed());
        }
        if !alive {
            let since = *gone_since.get_or_insert_with(Instant::now);
            if since.elapsed() > RECORD_GRACE {
                release_inline(pid);
                return finish(&job, workspace, cwd, -1, started.elapsed());
            }
        }
        if started.elapsed() >= window {
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    release_inline(pid);
    // The job may have ended in the instant before the release; its notice was skipped, so say so.
    if let Some(code) = background::get(workspace, cwd, pid).and_then(|j| j.exit_code()) {
        return finish(&job, workspace, cwd, code, started.elapsed());
    }
    let so_far = background::tail(&job.log, TAIL_LINES).unwrap_or_default();
    let mut out = format!(
        "shell: still running after {}s, moved to background job {pid}\n  log: {}\n  \
         You will be told when it exits; do not poll. Meanwhile do other work, or look now with \
         shell_job {{\"action\": \"log\", \"id\": {pid}}}; stop it with {{\"action\": \"stop\", \
         \"id\": {pid}}}.",
        window.as_secs(),
        job.log.display()
    );
    if !so_far.trim().is_empty() {
        out.push_str(&format!(
            "\n\nOutput so far (last {TAIL_LINES} lines):\n{so_far}"
        ));
    }
    out
}

/// The foreground-shaped result of a job that ended inside the window, then the job's files
/// removed: nothing about it needs to be found again.
fn finish(
    job: &background::Job,
    workspace: Option<&Path>,
    cwd: &str,
    code: i64,
    took: Duration,
) -> String {
    let raw = std::fs::read(&job.log).unwrap_or_default();
    let rendered = super::render_streams(&raw, &[]);
    let (body, truncated) = super::truncate_for_model(&rendered, super::MODEL_BUDGET);
    let status = if code < 0 {
        "signal".to_string()
    } else {
        code.to_string()
    };
    let mut header = format!("shell: exit {status} in {}ms", took.as_millis());
    if truncated {
        header.push_str(&format!(
            "  ({} bytes captured, output truncated)",
            raw.len()
        ));
    }
    let dir = background::job_dir(workspace, cwd, job.pid);
    let _ = std::fs::remove_file(&job.log);
    let _ = std::fs::remove_file(dir.join(format!("{}.json", job.pid)));
    if body.trim().is_empty() {
        header
    } else {
        format!("{header}\n\n{body}")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().display().to_string();
        (dir, cwd)
    }

    #[tokio::test]
    async fn a_command_that_finishes_inside_the_window_reads_like_a_foreground_run() {
        crate::test_state::isolate();
        let (dir, cwd) = fixture();
        let out = run(
            "echo hello; exit 3",
            &cwd,
            Some(dir.path()),
            &SandboxPolicy::default(),
            Duration::from_secs(10),
        )
        .await;
        assert!(out.starts_with("shell: exit 3 in "), "{out}");
        assert!(out.contains("hello"), "{out}");
        assert!(
            background::list(Some(dir.path()), &cwd).is_empty(),
            "an inline run leaves no job behind"
        );
    }

    #[tokio::test]
    async fn a_command_still_running_at_the_window_moves_to_the_background() {
        crate::test_state::isolate();
        let (dir, cwd) = fixture();
        let out = run(
            "echo started; sleep 30",
            &cwd,
            Some(dir.path()),
            &SandboxPolicy::default(),
            Duration::from_millis(700),
        )
        .await;
        assert!(out.contains("moved to background job"), "{out}");
        assert!(out.contains("do not poll"), "{out}");
        assert!(out.contains("started"), "output so far is returned: {out}");
        let jobs = background::list(Some(dir.path()), &cwd);
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].is_running());
        background::stop(Some(dir.path()), &cwd, jobs[0].pid)
            .await
            .unwrap();
    }
}
