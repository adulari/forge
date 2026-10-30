//! The `gh` runner and its failure classification.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::response::Response;
use tokio::io::AsyncWriteExt as _;

use crate::serve::err_response;

const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `gh`; the program is injectable so tests drive a fake.
#[derive(Clone)]
pub(crate) struct Gh {
    program: PathBuf,
}

impl Gh {
    pub(crate) fn system() -> Self {
        Self {
            program: PathBuf::from("gh"),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_program(program: PathBuf) -> Self {
        Self { program }
    }

    pub(crate) async fn run(
        &self,
        cwd: &Path,
        args: &[&str],
        stdin: Option<&str>,
    ) -> Result<String, GhError> {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .args(args)
            .current_dir(cwd)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("PAGER", "cat")
            .env("NO_COLOR", "1")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GhError::Missing
            } else {
                GhError::Failed(format!("could not run gh: {e}"))
            }
        })?;
        let pipe = child.stdin.take();
        let work = async move {
            if let (Some(text), Some(mut pipe)) = (stdin, pipe) {
                let _ = pipe.write_all(text.as_bytes()).await;
            }
            child.wait_with_output().await
        };
        let output = tokio::time::timeout(GH_TIMEOUT, work)
            .await
            .map_err(|_| GhError::Failed("gh timed out".into()))?
            .map_err(|e| GhError::Failed(format!("gh: {e}")))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(GhError::classify(stderr))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GhError {
    Missing,
    SignedOut(String),
    NoPr,
    Failed(String),
}

impl GhError {
    pub(crate) fn classify(stderr: String) -> Self {
        let lower = stderr.to_lowercase();
        if lower.contains("no pull requests found") {
            Self::NoPr
        } else if lower.contains("gh auth login") || lower.contains("not logged in") {
            Self::SignedOut(stderr)
        } else {
            Self::Failed(if stderr.is_empty() {
                "gh failed".into()
            } else {
                stderr
            })
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Missing => "the GitHub CLI (gh) is not installed on this host".into(),
            Self::SignedOut(_) => "gh is not signed in; run `gh auth login` on this host".into(),
            Self::NoPr => "no pull request for this branch".into(),
            Self::Failed(m) => m.clone(),
        }
    }

    pub(crate) fn unavailable(&self) -> bool {
        matches!(self, Self::Missing | Self::SignedOut(_))
    }
}

pub(crate) fn fail(error: &GhError) -> Response {
    let status = match error {
        GhError::Missing | GhError::SignedOut(_) => axum::http::StatusCode::FAILED_DEPENDENCY,
        _ => axum::http::StatusCode::BAD_REQUEST,
    };
    err_response(status, &error.message())
}
