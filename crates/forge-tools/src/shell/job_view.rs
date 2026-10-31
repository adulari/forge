//! A read-and-stop view of a workspace's background jobs for the chat TUI's `/jobs` list. The
//! model reaches the same records through the `shell_job` tool; this is the person's door.

use std::path::{Path, PathBuf};

use super::background;

/// One background job, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobView {
    /// The job's id, which is its pid.
    pub id: u32,
    pub command: String,
    pub running: bool,
    /// `running`, `exited N` or `gone`.
    pub status: String,
    pub started_at: u64,
    pub log: PathBuf,
}

/// Every job recorded for `workspace`, newest first.
pub fn job_views(workspace: &Path) -> Vec<JobView> {
    let cwd = workspace.to_string_lossy();
    background::list(Some(workspace), &cwd)
        .into_iter()
        .map(|job| JobView {
            id: job.pid,
            running: job.is_running(),
            status: job.status(),
            command: job.command.clone(),
            started_at: job.started_at,
            log: job.log.clone(),
        })
        .collect()
}

/// The last `lines` lines of a job's log.
pub fn job_tail(log: &Path, lines: usize) -> Result<String, String> {
    background::tail(log, lines)
}

/// Stop a job (its whole process group).
pub async fn stop_job(workspace: &Path, id: u32) -> Result<String, String> {
    let cwd = workspace.to_string_lossy();
    background::stop(Some(workspace), &cwd, id).await
}
