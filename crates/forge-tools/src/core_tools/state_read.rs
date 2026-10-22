//! Read access to the session's background-job logs.
//!
//! Job logs used to sit under `<workspace>/.forge/jobs/`, inside the confinement `read_file`
//! enforces. They now live in Forge's data directory, outside the project, and the job's start
//! message still prints the log path, so a model that reads it with `read_file` must not be
//! refused. Only this workspace's own `jobs` folder is opened, and only for reading: checkpoints
//! stay out of reach, and so does every other workspace's state.

use std::path::{Path, PathBuf};

use super::{confine, resolve_target};
use crate::ToolError;

/// [`confine`], plus this workspace's background-job logs.
pub(super) fn confine_read(path_str: &str) -> Result<PathBuf, ToolError> {
    confine(path_str).or_else(|refused| {
        let target = resolve_target(Path::new(path_str));
        let workspace = crate::SESSION_WORKSPACE
            .try_with(Clone::clone)
            .or_else(|_| std::env::current_dir());
        let jobs = workspace
            .ok()
            .map(|ws| forge_config::state_subdir(&ws, "jobs"))
            .map(|jobs| jobs.canonicalize().unwrap_or(jobs));
        match jobs {
            Some(jobs) if target.starts_with(&jobs) => Ok(target),
            _ => Err(refused),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_job_log_in_the_state_folder_is_readable_but_nothing_else_there_is() {
        crate::test_state::isolate();
        let ws = tempfile::tempdir().unwrap();
        let ws_path = ws.path().canonicalize().unwrap();
        let jobs = forge_config::state_subdir(&ws_path, "jobs");
        let checkpoints = forge_config::state_subdir(&ws_path, "checkpoints");
        std::fs::create_dir_all(&jobs).unwrap();
        std::fs::create_dir_all(&checkpoints).unwrap();
        std::fs::write(jobs.join("42.log"), "listening on :3000\n").unwrap();
        std::fs::write(checkpoints.join("x"), "blob").unwrap();

        crate::SESSION_WORKSPACE
            .scope(ws_path.clone(), async {
                let log = jobs.join("42.log");
                assert!(confine_read(log.to_str().unwrap()).is_ok());
                assert!(confine_read(checkpoints.join("x").to_str().unwrap()).is_err());
                assert!(confine_read("/etc/passwd").is_err());
                let escaped = format!("{}/../checkpoints/x", jobs.display());
                assert!(confine_read(&escaped).is_err(), "no traversal out of jobs/");
            })
            .await;
    }
}
