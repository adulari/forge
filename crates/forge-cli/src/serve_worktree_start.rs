//! Creating the worktree a new daemon session runs in: name, location and the `[worktree]`
//! provisioning (ignored files copied in, `setup` run).

use std::path::{Path, PathBuf};

use forge_core::worktree::WorktreeGuard;
use forge_core::worktree_setup;

/// A freshly added worktree whose provisioning (ignored files, `setup`) has not run yet.
pub(crate) struct Created {
    pub guard: WorktreeGuard,
    repo: PathBuf,
    settings: forge_config::worktree::WorktreeConfig,
}

/// Everything provisioning needs once the guard is gone; owned, so it outlives the request.
pub(crate) struct Pending {
    repo: PathBuf,
    path: PathBuf,
    branch: String,
    settings: forge_config::worktree::WorktreeConfig,
}

impl Created {
    /// `None` when `[worktree]` asks for nothing, so no status is shown for a plain worktree.
    pub fn pending(&self) -> Option<Pending> {
        let wanted = !self.settings.copy_ignored.is_empty()
            || self
                .settings
                .setup
                .as_deref()
                .is_some_and(|c| !c.trim().is_empty());
        wanted.then(|| Pending {
            repo: self.repo.clone(),
            path: self.guard.path().to_path_buf(),
            branch: self.guard.branch().to_string(),
            settings: self.settings.clone(),
        })
    }
}

/// `git worktree add` for a new session at `repo`, named `requested` or a readable random name.
/// Blocking but quick: provisioning is deliberately NOT done here (see [`spawn_provisioning`]),
/// because a `setup` such as `npm ci` takes longer than a client waits for the create call.
pub(crate) fn create_only(repo: &Path, requested: Option<&str>) -> Result<Created, String> {
    let settings = forge_config::load_in(repo)
        .map(|config| config.worktree)
        .unwrap_or_default();
    let requested = requested.map(str::trim).filter(|n| !n.is_empty());
    let guard = match requested {
        Some(raw) => {
            let name = forge_core::worktree_ops::slug(raw);
            if name.is_empty() {
                return Err(format!("{raw:?} is not a usable branch name"));
            }
            let path = worktree_setup::worktree_path(repo, &settings, &name);
            WorktreeGuard::create_at(repo, &name, path).map_err(|e| e.to_string())?
        }
        // The generated space is large, not unique: a kept branch from an earlier session can
        // already own the name.
        None => {
            let mut attempt = 0;
            loop {
                let name = worktree_setup::friendly_name();
                let path = worktree_setup::worktree_path(repo, &settings, &name);
                match WorktreeGuard::create_at(repo, &name, path) {
                    Ok(guard) => break guard,
                    Err(e) if attempt >= 5 => return Err(e.to_string()),
                    Err(_) => attempt += 1,
                }
            }
        }
    };
    Ok(Created {
        guard,
        repo: repo.to_path_buf(),
        settings,
    })
}

pub(crate) async fn create_off_thread(
    repo: &Path,
    branch: Option<String>,
) -> Result<Created, String> {
    let repo = repo.to_path_buf();
    tokio::task::spawn_blocking(move || create_only(&repo, branch.as_deref()))
        .await
        .unwrap_or_else(|e| Err(format!("worktree task failed: {e}")))
}

pub(crate) type SetupCell =
    std::sync::Arc<std::sync::Mutex<Option<crate::remote::SnapWorktreeSetup>>>;

/// Where a worktree's setup state is kept between daemon runs: inside its git dir, so it never
/// shows up in `git status`.
fn marker_file(worktree: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("forge-setup-state"))
}

fn write_marker(worktree: &Path, state: &str, message: &str) {
    if let Some(file) = marker_file(worktree) {
        let _ = std::fs::write(
            file,
            serde_json::json!({ "state": state, "message": message }).to_string(),
        );
    }
}

/// Forget a stored `failed`/`interrupted` setup once the agent has been told, so later resumes
/// do not repeat it forever. A new setup writes its own marker.
pub(crate) fn clear_marker(worktree: &str) {
    if let Some(file) = marker_file(Path::new(worktree)) {
        let _ = std::fs::remove_file(file);
    }
}

/// What a resumed session should show about a setup the daemon never saw finish: a `running`
/// marker means the daemon went down mid-setup, so the worktree may be incomplete.
pub(crate) fn interrupted_status(worktree: &str) -> Option<crate::remote::SnapWorktreeSetup> {
    let text = std::fs::read_to_string(marker_file(Path::new(worktree))?).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let message = value["message"].as_str().unwrap_or_default().to_string();
    match value["state"].as_str()? {
        "running" => Some(crate::remote::SnapWorktreeSetup {
            state: "interrupted".into(),
            message: "the daemon stopped while this worktree's setup was running".into(),
        }),
        "failed" => Some(crate::remote::SnapWorktreeSetup {
            state: "failed".into(),
            message,
        }),
        _ => None,
    }
}

/// Start the background setup of a new worktree, or restore what a resumed one's last setup left.
pub(crate) fn start_provisioning(
    handle: &crate::cli::commands::run::driver::SessionDriverHandle,
    pending: Option<Pending>,
    resumed_worktree: Option<&str>,
) {
    restore_status(handle, resumed_worktree);
    if let Some(pending) = pending {
        spawn_provisioning(
            pending,
            handle.worktree_setup_cell(),
            handle.setup_control(),
        );
    }
}

/// A resumed session starts with whatever its worktree's last setup left behind.
pub(crate) fn restore_status(
    handle: &crate::cli::commands::run::driver::SessionDriverHandle,
    resumed_worktree: Option<&str>,
) {
    if let Some(status) = resumed_worktree.and_then(interrupted_status) {
        *handle
            .worktree_setup_cell()
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(status);
    }
}

/// Run the copy and `setup` on a blocking thread and publish `running` then `done`/`failed` in
/// `cell`, which the session driver puts in every snapshot. Call only once the session's driver
/// is up and the creation guard has been released: nothing here may be dropped with a request.
/// `control` lets the session stop it (archive, delete, daemon shutdown).
pub(crate) fn spawn_provisioning(
    pending: Pending,
    cell: SetupCell,
    control: std::sync::Arc<worktree_setup::SetupControl>,
) {
    let path = pending.path.clone();
    let publish = move |state: &str, message: String| {
        write_marker(&path, state, &message);
        *cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(crate::remote::SnapWorktreeSetup {
            state: state.to_string(),
            message,
        });
    };
    control.set_running(true);
    publish(
        "running",
        "Setting up the worktree (copying files, running setup)".into(),
    );
    tokio::task::spawn_blocking(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worktree_setup::provision_with(
                &pending.repo,
                &pending.path,
                &pending.branch,
                &pending.settings,
                Some(&control),
            )
        }));
        control.set_running(false);
        match result {
            Ok(provisioned) => match provisioning_problem(&provisioned) {
                Some(problem) => {
                    tracing::warn!(worktree = %pending.path.display(), "{problem}");
                    publish("failed", problem);
                }
                None => publish("done", provisioned.summary()),
            },
            Err(_) => publish("failed", "the worktree setup crashed unexpectedly".into()),
        }
    });
}

fn provisioning_problem(provisioned: &worktree_setup::Provisioned) -> Option<String> {
    let mut parts = Vec::new();
    if provisioned.setup.ran && !provisioned.setup.ok {
        parts.push(format!(
            "the `setup` command failed: {}",
            provisioned.setup.output.trim()
        ));
    }
    if !provisioned.skipped.is_empty() {
        parts.push(format!(
            "not copied from the main checkout: {}",
            provisioned.skipped.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Run the `cleanup` command of the repository's `[worktree]` config in `worktree`.
pub(crate) fn run_cleanup(repo: &Path, worktree: &Path, branch: &str) {
    let settings = forge_config::load_in(repo)
        .map(|config| config.worktree)
        .unwrap_or_default();
    let result = worktree_setup::teardown(repo, worktree, branch, &settings);
    if result.ran && !result.ok {
        tracing::warn!(worktree = %worktree.display(), "worktree cleanup failed: {}", result.output);
    }
}

/// The main checkout a linked worktree belongs to, from git's common dir.
pub(crate) fn repo_root_of(worktree: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let common = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    common.parent().map(Path::to_path_buf)
}

/// Derive the base repo root and branch for a daemon worktree session from its stored worktree
/// path. Worktrees are always created at `<repo_root>/.forge/worktrees/<child_id>` on branch
/// `forge/subagent/<child_id>` (see [`forge_core::worktree::WorktreeGuard::create`]), so both are
/// recoverable from the path alone — no extra state, and it matches exactly what the guard's own
/// removal uses. Returns `None` if the path is too shallow to be one of ours.
pub(crate) fn worktree_repo_and_branch(worktree: &str) -> Option<(std::path::PathBuf, String)> {
    let wt = std::path::Path::new(worktree);
    let child_id = wt.file_name()?.to_str()?.to_string();
    let in_repo = wt.parent().and_then(|p| p.parent()).is_some_and(|p| {
        p.file_name().is_some_and(|n| n == ".forge")
            && wt.parent().is_some_and(|p| p.ends_with("worktrees"))
    });
    // `<repo_root>/.forge/worktrees/<child_id>` → strip three components back to `<repo_root>`.
    // A `[worktree] root = "global"` checkout lives outside its repository, so ask git instead.
    let repo_root = if in_repo {
        wt.parent()?.parent()?.parent()?.to_path_buf()
    } else {
        crate::serve_worktree_start::repo_root_of(wt)?
    };
    Some((repo_root, format!("forge/subagent/{child_id}")))
}

/// The `effort` a create-session request asks for; `Err(())` for an unknown value.
pub(crate) fn parse_effort(raw: Option<&str>) -> Result<Option<forge_types::EffortLevel>, ()> {
    match raw.map(str::trim).filter(|e| !e.is_empty()) {
        Some(raw) => forge_types::EffortLevel::parse(raw).map(Some).ok_or(()),
        None => Ok(None),
    }
}

pub(crate) fn bad_effort(raw: Option<&str>) -> axum::response::Response {
    crate::serve::err_response(
        axum::http::StatusCode::BAD_REQUEST,
        &format!("effort: unknown value {:?}", raw.unwrap_or_default().trim()),
    )
}

/// Apply a requested effort to a freshly created session before its first prompt.
pub(crate) async fn apply_effort(
    handle: &crate::cli::commands::run::driver::SessionDriverHandle,
    level: Option<forge_types::EffortLevel>,
) {
    if let Some(level) = level {
        let _ = handle
            .input_tx
            .send(crate::remote::RemoteInput::Prompt {
                text: format!("/effort {}", level.as_str()),
                attachments: Vec::new(),
            })
            .await;
    }
}

#[cfg(test)]
#[path = "serve_worktree_start/tests.rs"]
mod tests;
