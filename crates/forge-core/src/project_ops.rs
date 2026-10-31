//! Project onboarding shared by the daemon and the TUI: clone a repository, `git init` a plain
//! folder, the "No project (Home)" scope, a GitHub repository list, and a stash-aware pull.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::remote_url::{validate_remote_url, NETWORK_ENV};

const CLONE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const PULL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GH_TIMEOUT: Duration = Duration::from_secs(25);

struct Captured {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn capture(mut command: Command, timeout: Duration) -> Result<Captured, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut s) = stream {
                let _ = s.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = drain(child.stdout.take().map(|s| Box::new(s) as _));
    let err = drain(child.stderr.take().map(|s| Box::new(s) as _));
    let started = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok(Captured {
        ok,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

fn git_in(dir: &Path, args: &[&str], network: bool, timeout: Duration) -> Result<Captured, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args);
    if network {
        command.envs(NETWORK_ENV);
    }
    capture(command, timeout)
}

/// The directory name a clone of `url` gets: the last path segment without `.git`.
pub fn repo_name_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    ok.then(|| name.to_string())
}

/// `git clone` into `<parent>/<name>`. The URL goes through the same validator as every other
/// remote a client hands the daemon.
pub fn clone_repo(url: &str, parent: &Path, name: Option<&str>) -> Result<PathBuf, String> {
    validate_remote_url(url)?;
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if repo_name_from_url(&format!("x/{n}")).as_deref() == Some(n) => n.to_string(),
        Some(_) => return Err("folder name may only use letters, digits, '.', '_' and '-'".into()),
        None => repo_name_from_url(url).ok_or("could not derive a folder name from the URL")?,
    };
    if !parent.is_dir() {
        return Err(format!("{} is not a directory", parent.display()));
    }
    let dest = parent.join(&name);
    if dest.exists() && std::fs::read_dir(&dest).is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!(
            "{} already exists and is not empty",
            dest.display()
        ));
    }
    let mut command = Command::new("git");
    command
        .arg("clone")
        .arg("--no-recurse-submodules")
        .arg("--")
        .arg(url.trim())
        .arg(&dest)
        .envs(NETWORK_ENV);
    let result = capture(command, CLONE_TIMEOUT)?;
    if result.ok {
        Ok(dest.canonicalize().unwrap_or(dest))
    } else {
        Err(format!("git clone failed: {}", result.stderr.trim()))
    }
}

/// Turn a plain folder into a repository with an empty first commit, so worktrees can branch
/// from HEAD straight away.
pub fn init_repo(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    if path.join(".git").exists() {
        return Err("already a git repository".into());
    }
    if let Ok(outer) = git_in(
        path,
        &["rev-parse", "--show-toplevel"],
        false,
        Duration::from_secs(30),
    ) {
        if outer.ok {
            return Err(format!(
                "already inside the git repository at {}; a nested repository would hide this \
                 folder from it",
                outer.stdout.trim()
            ));
        }
    }
    let init = git_in(path, &["init", "-q"], false, Duration::from_secs(30))?;
    if !init.ok {
        return Err(format!("git init failed: {}", init.stderr.trim()));
    }
    let commit = git_in(
        path,
        &[
            "-c",
            "user.name=Forge",
            "-c",
            "user.email=forge@adulari.dev",
            "commit",
            "-q",
            "--allow-empty",
            "--no-gpg-sign",
            "-m",
            "Initial commit",
        ],
        false,
        Duration::from_secs(30),
    )?;
    if commit.ok {
        Ok(())
    } else {
        Err(format!("initial commit failed: {}", commit.stderr.trim()))
    }
}

/// The "No project (Home)" scope: `~/.forge/home`, created on demand.
pub fn home_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or("no home directory")?;
    let dir = home.join(".forge").join("home");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct GhRepo {
    pub name_with_owner: String,
    pub url: String,
    pub description: String,
    pub private: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct GhRepos {
    /// False when `gh` is missing or not signed in; `note` says which.
    pub available: bool,
    pub note: String,
    pub repos: Vec<GhRepo>,
}

/// Repositories of the signed-in `gh` account, newest push first.
pub fn gh_repos() -> GhRepos {
    let mut command = Command::new("gh");
    command.args([
        "repo",
        "list",
        "--limit",
        "100",
        "--json",
        "nameWithOwner,url,description,isPrivate",
    ]);
    let unavailable = |note: &str| GhRepos {
        available: false,
        note: note.to_string(),
        repos: Vec::new(),
    };
    let out = match capture(command, GH_TIMEOUT) {
        Ok(out) => out,
        Err(e) if e.contains("No such file") || e.contains("not found") => {
            return unavailable("the GitHub CLI (gh) is not installed on this host")
        }
        Err(e) => return unavailable(&format!("gh repo list failed: {e}")),
    };
    if !out.ok {
        return unavailable("gh is not signed in on this host (run `gh auth login`)");
    }
    parse_gh_repos(&out.stdout)
        .map(|repos| GhRepos {
            available: true,
            note: String::new(),
            repos,
        })
        .unwrap_or_else(|| unavailable("gh returned something unreadable"))
}

pub fn parse_gh_repos(json: &str) -> Option<Vec<GhRepo>> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    Some(
        rows.iter()
            .filter_map(|row| {
                let text = |k: &str| row.get(k).and_then(|v| v.as_str()).map(str::to_string);
                Some(GhRepo {
                    name_with_owner: text("nameWithOwner")?,
                    url: text("url")?,
                    description: text("description").unwrap_or_default(),
                    private: row
                        .get("isPrivate")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                })
            })
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullStatus {
    UpToDate,
    Updated,
    /// The pull stopped on merge/rebase conflicts that are still in the working tree.
    Conflict,
    /// The pull worked but re-applying the stashed local changes conflicted; the stash is kept.
    StashConflict,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PullOutcome {
    pub status: PullStatus,
    pub message: String,
    pub conflicts: Vec<String>,
    /// For `Conflict`/`StashConflict`: a prompt that hands the resolution to the agent.
    pub handoff_prompt: Option<String>,
    pub stashed: bool,
}

impl PullOutcome {
    fn failed(message: impl Into<String>, stashed: bool) -> Self {
        Self {
            status: PullStatus::Failed,
            message: message.into(),
            conflicts: Vec::new(),
            handoff_prompt: None,
            stashed,
        }
    }
}

fn conflicted_files(dir: &Path) -> Vec<String> {
    git_in(
        dir,
        &["diff", "--name-only", "--diff-filter=U"],
        false,
        PULL_TIMEOUT,
    )
    .map(|o| o.stdout.lines().map(str::to_string).collect())
    .unwrap_or_default()
}

/// `git pull --rebase` that first stashes local changes and re-applies them afterwards. A
/// conflict is left in the working tree and described in `handoff_prompt` for the agent.
pub fn pull(dir: &Path) -> PullOutcome {
    let upstream = git_in(
        dir,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        false,
        PULL_TIMEOUT,
    );
    if !upstream.is_ok_and(|o| o.ok) {
        return PullOutcome::failed(
            "this branch has no upstream to pull from (a Forge worktree branch is local); merge \
             or rebase it onto the main branch instead",
            false,
        );
    }
    let dirty = git_in(dir, &["status", "--porcelain"], false, PULL_TIMEOUT)
        .map(|o| !o.stdout.trim().is_empty())
        .unwrap_or(false);
    let mut stashed = false;
    let mut stash_commit: Option<String> = None;
    if dirty {
        match git_in(
            dir,
            &["stash", "push", "-u", "-m", "forge: before pull"],
            false,
            PULL_TIMEOUT,
        ) {
            Ok(o) if o.ok => {
                stashed = true;
                stash_commit = crate::worktree_ops::top_stash(dir);
            }
            Ok(o) => {
                return PullOutcome::failed(format!("stash failed: {}", o.stderr.trim()), false)
            }
            Err(e) => return PullOutcome::failed(format!("stash failed: {e}"), false),
        }
    }
    let before = git_in(dir, &["rev-parse", "HEAD"], false, PULL_TIMEOUT)
        .map(|o| o.stdout.trim().to_string())
        .unwrap_or_default();
    let pulled = git_in(dir, &["pull", "--rebase", "--no-edit"], true, PULL_TIMEOUT);
    match pulled {
        Err(e) => {
            return restore_after_failure(
                dir,
                stash_commit.as_deref(),
                stashed,
                format!("pull failed: {e}"),
            )
        }
        Ok(o) if !o.ok => {
            let conflicts = conflicted_files(dir);
            if conflicts.is_empty() {
                let _ = git_in(dir, &["rebase", "--abort"], false, PULL_TIMEOUT);
                return restore_after_failure(
                    dir,
                    stash_commit.as_deref(),
                    stashed,
                    format!("pull failed: {}", o.stderr.trim()),
                );
            }
            let prompt = format!(
                "`git pull --rebase` stopped on conflicts in: {}. Resolve them (keep both sides' \
                 intent), `git add` the files and run `git rebase --continue`.{}",
                conflicts.join(", "),
                if stashed {
                    " Local changes were stashed before the pull; run `git stash pop` afterwards."
                } else {
                    ""
                }
            );
            return PullOutcome {
                status: PullStatus::Conflict,
                message: format!("pull stopped on {} conflicted file(s)", conflicts.len()),
                conflicts,
                handoff_prompt: Some(prompt),
                stashed,
            };
        }
        Ok(_) => {}
    }
    let after = git_in(dir, &["rev-parse", "HEAD"], false, PULL_TIMEOUT)
        .map(|o| o.stdout.trim().to_string())
        .unwrap_or_default();
    if stashed {
        let popped = stash_commit
            .as_deref()
            .is_some_and(|c| crate::worktree_ops::pop_stash(dir, c, false).is_ok());
        if !popped {
            let conflicts = conflicted_files(dir);
            let prompt = format!(
                "The pull succeeded but re-applying the local changes I had stashed conflicts in: \
                 {}. Resolve the conflict markers; the stash is kept until `git stash drop`.",
                conflicts.join(", ")
            );
            return PullOutcome {
                status: PullStatus::StashConflict,
                message: "pulled, but your local changes conflict with the update".into(),
                conflicts,
                handoff_prompt: Some(prompt),
                stashed: true,
            };
        }
    }
    PullOutcome {
        status: if before == after {
            PullStatus::UpToDate
        } else {
            PullStatus::Updated
        },
        message: if before == after {
            "already up to date".into()
        } else {
            "pulled the latest changes".into()
        },
        conflicts: Vec::new(),
        handoff_prompt: None,
        stashed,
    }
}

fn restore_after_failure(
    dir: &Path,
    stash_commit: Option<&str>,
    stashed: bool,
    message: String,
) -> PullOutcome {
    if let Some(commit) = stash_commit {
        let _ = crate::worktree_ops::pop_stash(dir, commit, false);
    }
    PullOutcome::failed(message, stashed)
}

#[cfg(test)]
#[path = "tests/project_ops.rs"]
mod tests;
