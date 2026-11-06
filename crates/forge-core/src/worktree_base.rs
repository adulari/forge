//! Which commit a new session worktree branches from.
//!
//! On a box where PRs merge on GitHub, the local default branch goes stale between pulls, so a
//! worktree cut from `HEAD` starts dozens of commits behind and the conflicts only show at PR
//! time. [`fresh_base`] runs one bounded `git fetch origin <default>` and, when the checkout is on
//! its default branch and `origin/<default>` is strictly ahead, branches from that commit instead.
//! It only ever updates the remote-tracking ref: the checkout's branch, index and working tree
//! are never touched.
//!
//! Only worktrees for user-facing sessions use this (daemon sessions, dispatch, `/worktree
//! create`). Subagent, duel and queue worktrees keep branching from `HEAD` on purpose: their
//! results are merged back as a `HEAD..branch` patch onto the main tree, which must share their
//! base.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

/// Long enough for a slow remote, short enough that creating a session never feels hung.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseChoice {
    /// What to pass to `git worktree add` as the start point: a commit id or `HEAD`.
    pub start_point: String,
    /// One line for the session: what the worktree is based on and why.
    pub note: String,
}

impl BaseChoice {
    fn head(note: String) -> Self {
        Self {
            start_point: "HEAD".into(),
            note,
        }
    }
}

pub fn fresh_base(repo: &Path) -> BaseChoice {
    fresh_base_with(repo, FETCH_TIMEOUT)
}

pub fn fresh_base_with(repo: &Path, timeout: Duration) -> BaseChoice {
    let Some(current) = git(repo, &["symbolic-ref", "--short", "-q", "HEAD"]) else {
        return BaseChoice::head("based on a detached HEAD (not on a branch)".into());
    };
    if git(repo, &["remote", "get-url", "origin"]).is_none() {
        return BaseChoice::head(format!("based on local {current} (no origin remote)"));
    }
    let Some(default) = git(
        repo,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    )
    .and_then(|r| r.strip_prefix("origin/").map(str::to_string)) else {
        return BaseChoice::head(format!(
            "based on local {current} (origin's default branch is unknown: origin/HEAD is not set)"
        ));
    };
    if current != default {
        return BaseChoice::head(format!(
            "based on local {current} (not the default branch {default}, so origin was not consulted)"
        ));
    }

    if let Err(reason) = fetch(repo, &default, timeout) {
        let age = last_fetch_age(repo, &default)
            .map(|d| format!(", {} since last fetch", describe_age(d)))
            .unwrap_or_default();
        return BaseChoice::head(format!(
            "based on local {current}; fetch failed: {reason}{age}"
        ));
    }

    let remote_ref = format!("refs/remotes/origin/{default}");
    let Some(tip) = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{remote_ref}^{{commit}}"),
        ],
    ) else {
        return BaseChoice::head(format!(
            "based on local {current} (origin/{default} not found)"
        ));
    };
    let short = git(repo, &["rev-parse", "--short", &tip]).unwrap_or_else(|| tip.clone());
    let count = |range: String| -> u64 {
        git(repo, &["rev-list", "--count", &range])
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    let behind = count(format!("{current}..{remote_ref}"));
    let ahead = count(format!("{remote_ref}..{current}"));
    match (behind, ahead) {
        (0, 0) => BaseChoice::head(format!(
            "based on {current} @{short} (up to date with origin/{default})"
        )),
        (0, n) => BaseChoice::head(format!(
            "based on local {current} (origin/{default} has nothing new; local is {n} ahead)"
        )),
        (n, 0) => BaseChoice {
            start_point: tip,
            note: format!("based on origin/{default} @{short} (local {current} was {n} behind)"),
        },
        (b, a) => BaseChoice::head(format!(
            "based on local {current}; it diverged from origin/{default} ({b} behind, {a} ahead), \
             so origin's tip was not used"
        )),
    }
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// `git fetch origin <branch>` that can neither prompt nor outlive `timeout`.
fn fetch(repo: &Path, branch: &str, timeout: Duration) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "origin",
        ])
        .arg(format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"))
        .envs(crate::remote_url::NETWORK_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if std::env::var_os("GIT_SSH_COMMAND").is_none()
        && git(repo, &["config", "core.sshCommand"]).is_none()
    {
        cmd.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=5",
        );
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    // Read stderr on a thread: an ssh grandchild can keep the pipe open after git is killed.
    let mut pipe = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(p) = pipe.as_mut() {
            let _ = p.read_to_string(&mut text);
        }
        let _ = tx.send(text);
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", timeout.as_secs().max(1)));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    };
    if status.success() {
        return Ok(());
    }
    let stderr = rx
        .recv_timeout(Duration::from_millis(500))
        .unwrap_or_default();
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .map(|l| {
            l.trim_start_matches("fatal:")
                .trim_start_matches("error:")
                .trim()
        })
        .or_else(|| stderr.lines().map(str::trim).rfind(|l| !l.is_empty()))
        .unwrap_or("git fetch failed");
    Err(line.chars().take(160).collect())
}

fn last_fetch_age(repo: &Path, branch: &str) -> Option<Duration> {
    let dir = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let tracking = format!("refs/remotes/origin/{branch}");
    let modified = [
        "FETCH_HEAD",
        tracking.as_str(),
        "refs/remotes/origin/HEAD",
        "packed-refs",
    ]
    .iter()
    .filter_map(|f| {
        std::fs::metadata(Path::new(&dir).join(f))
            .ok()?
            .modified()
            .ok()
    })
    .max()?;
    SystemTime::now().duration_since(modified).ok()
}

fn describe_age(age: Duration) -> String {
    let days = age.as_secs() / 86_400;
    match days {
        0 => "under a day".into(),
        1 => "1 day".into(),
        n => format!("{n} days"),
    }
}

/// The commit a session branch forked from, for diffing what the session itself changed.
///
/// A worktree started from a freshly fetched `origin/<default>` ([`fresh_base`]) forks AFTER the
/// checkout's own `HEAD`, so `merge-base HEAD <branch>` alone lands on the stale local tip and
/// the diff would carry every upstream commit the checkout has not pulled yet as if the session
/// wrote it. Of the two fork points (against `HEAD` and against `origin/<default>`), the newer one
/// is the real start: for a branch made from `HEAD` the origin candidate is older or the same.
pub fn fork_point(repo: &Path, branch: &str) -> Option<String> {
    let from_head = git(repo, &["merge-base", "HEAD", branch]);
    let from_origin = git(
        repo,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    )
    .and_then(|upstream| git(repo, &["merge-base", &upstream, branch]));
    match (from_head, from_origin) {
        (Some(head), Some(origin)) if head != origin && is_ancestor(repo, &head, &origin) => {
            Some(origin)
        }
        (Some(head), _) => Some(head),
        (None, origin) => origin,
    }
}

fn is_ancestor(repo: &Path, ancestor: &str, descendant: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Where a worktree's base note is kept: inside its git dir, so it never shows in `git status`
/// and survives a daemon restart.
fn note_file(worktree: &Path) -> Option<std::path::PathBuf> {
    git(worktree, &["rev-parse", "--absolute-git-dir"])
        .map(|d| std::path::PathBuf::from(d).join("forge-base-note"))
}

pub fn store_note(worktree: &Path, note: &str) {
    if let Some(file) = note_file(worktree) {
        let _ = std::fs::write(file, note);
    }
}

pub fn stored_note(worktree: &Path) -> Option<String> {
    let text = std::fs::read_to_string(note_file(worktree)?).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
#[path = "tests/worktree_base.rs"]
mod tests;
