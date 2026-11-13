//! The last local CI run, persisted so every surface can show it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pass,
    Fail,
    Timeout,
    /// A step that was never reached because an earlier one failed.
    NotRun,
    /// Same tree as a previous passing run.
    Cached,
    /// The command is not installed here (exit 127, or a missing cargo subcommand).
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    pub key: String,
    pub name: String,
    pub command: String,
    pub status: StepStatus,
    pub secs: u64,
    /// Last lines of combined output, kept for failures only.
    pub tail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiReport {
    pub started_unix: i64,
    pub finished_unix: i64,
    pub tree: String,
    pub results: Vec<StepResult>,
    pub skipped: Vec<String>,
    /// Why the run stopped before the end ("total budget spent", "step failed"), if it did.
    pub stopped: Option<String>,
}

impl CiReport {
    pub fn ok(&self) -> bool {
        self.results
            .iter()
            .any(|r| matches!(r.status, StepStatus::Pass | StepStatus::Cached))
            && self.results.iter().all(|r| {
                matches!(
                    r.status,
                    StepStatus::Pass | StepStatus::Cached | StepStatus::Skipped
                )
            })
    }

    pub fn first_failure(&self) -> Option<&StepResult> {
        self.results
            .iter()
            .find(|r| matches!(r.status, StepStatus::Fail | StepStatus::Timeout))
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let pass = self
            .results
            .iter()
            .filter(|r| matches!(r.status, StepStatus::Pass | StepStatus::Cached))
            .count();
        let fail = self
            .results
            .iter()
            .filter(|r| matches!(r.status, StepStatus::Fail | StepStatus::Timeout))
            .count();
        (pass, fail, self.results.len() - pass - fail)
    }
}

/// The git directory of `root` (a worktree's own, not the shared one), where the report lives so
/// it is never tracked and never shared between worktrees.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let target = text.trim().strip_prefix("gitdir:")?.trim();
    let path = PathBuf::from(target);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn report_path(root: &Path) -> PathBuf {
    git_dir(root)
        .unwrap_or_else(|| root.join(".forge"))
        .join("forge-ci-last.json")
}

pub fn load_report(root: &Path) -> Option<CiReport> {
    serde_json::from_str(&std::fs::read_to_string(report_path(root)).ok()?).ok()
}

pub fn save_report(root: &Path, report: &CiReport) -> std::io::Result<()> {
    let path = report_path(root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(report).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}
