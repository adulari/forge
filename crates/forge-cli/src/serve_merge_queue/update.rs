//! The queue head's git work: merge `origin/<base>` into the PR branch in the session's own
//! worktree, resolve changelog-only conflicts, run the project's verification, push.
//!
//! The merge commit is made by the worktree's configured git identity. `gh pr update-branch`
//! is never used: its commits are authored by github-actions[bot], which parks CI at
//! `action_required` and deadlocks the PR.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::changelog::{is_changelog_path, resolve, Resolution};

const VERIFY_TIMEOUT: Duration = Duration::from_secs(900);
const TAIL_BYTES: usize = 3000;

/// One verification command, from the project's actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verify {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) cwd: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Contains the base and everything is pushed; PR Watch carries on.
    UpToDate,
    /// Not now (dirty tree, fetch failed); try again on a later tick.
    Wait(String),
    /// Out of the queue until the branch moves, no turn needed.
    Skip(String),
    /// Out of the queue until the branch moves, and the session is told what to do.
    Handoff {
        reason: String,
        paths: Vec<String>,
        prompt: String,
    },
    Pushed {
        head: String,
        merged: bool,
        resolved: Vec<String>,
        verified: Vec<String>,
    },
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn git(root: &Path, args: &[&str]) -> Out {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(Stdio::null())
        .output();
    match result {
        Ok(o) => Out {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).trim_end().to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => Out {
            ok: false,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

fn merge_in_progress(root: &Path) -> bool {
    git(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).ok
}

fn abort_merge(root: &Path) {
    if merge_in_progress(root) {
        git(root, &["merge", "--abort"]);
    }
}

fn tail(text: &str) -> String {
    if text.len() <= TAIL_BYTES {
        return text.to_string();
    }
    let mut start = text.len() - TAIL_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// The last lines of a failing command, on one line, for the queue row's reason.
fn reason_tail(output: &str) -> String {
    const LINES: usize = 3;
    const CHARS: usize = 240;
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let joined = lines[lines.len().saturating_sub(LINES)..].join(" | ");
    if joined.chars().count() <= CHARS {
        return joined;
    }
    let keep: String = joined
        .chars()
        .skip(joined.chars().count() - CHARS)
        .collect();
    format!("...{keep}")
}

fn conflict_prompt(base: &str, paths: &[String]) -> String {
    format!(
        "Merge queue: this pull request is next to land, but `origin/{base}` conflicts with the \
         branch in:\n{}\n\nRun `git fetch origin && git merge origin/{base}` in this worktree, \
         resolve those paths by hand (keep both sides' intent, leave no conflict markers), run \
         the relevant checks, commit, and push with a normal `git push`. Never force-push and \
         never use `gh pr update-branch`. The queue carries on with the other pull requests \
         meanwhile and takes this one back once you push.",
        paths
            .iter()
            .map(|p| format!("- {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn verify_prompt(name: &str, output: &str) -> String {
    format!(
        "Merge queue: this branch now contains the latest base, but the verification `{name}` \
         fails on the merged result (the merge commit is local and not pushed):\n\n{}\n\nFix it \
         in this worktree, commit, and push with a normal `git push`. Never force-push. The \
         queue carries on with the other pull requests meanwhile and takes this one back once \
         you push.",
        tail(output)
    )
}

/// Resolve the unmerged paths of an in-progress merge, or say which ones cannot be resolved.
fn resolve_changelogs(root: &Path, paths: &[String]) -> Result<(), Vec<String>> {
    let mut unresolved = Vec::new();
    for path in paths {
        let raw = |n: u8| -> Option<String> {
            let out = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["show", &format!(":{n}:{path}")])
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let (Some(base), Some(ours), Some(theirs)) = (raw(1), raw(2), raw(3)) else {
            unresolved.push(path.clone());
            continue;
        };
        match resolve(&base, &ours, &theirs) {
            Resolution::Merged(text) => {
                let target: PathBuf = root.join(path);
                if std::fs::write(&target, text).is_err() || !git(root, &["add", "--", path]).ok {
                    unresolved.push(path.clone());
                }
            }
            Resolution::Refused(_) => unresolved.push(path.clone()),
        }
    }
    if unresolved.is_empty() {
        Ok(())
    } else {
        Err(unresolved)
    }
}

fn run_verify(root: &Path, v: &Verify) -> Result<(), String> {
    let dir = v
        .cwd
        .as_deref()
        .map_or_else(|| root.to_path_buf(), |c| root.join(c));
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&v.command)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > VERIFY_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", VERIFY_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Err(text)
}

/// Bring the checked-out PR branch up to `origin/<base>` and push it. Blocking.
pub(crate) fn run_step(root: &Path, base: &str, verify: &[Verify]) -> Step {
    let branch = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    if !branch.ok || branch.stdout.is_empty() {
        return Step::Skip("the session is on a detached HEAD".into());
    }
    let branch = branch.stdout;
    if branch == base {
        return Step::Skip(format!("the session is on the base branch {base}"));
    }
    if merge_in_progress(root) {
        return Step::Wait("a merge is already in progress in the worktree".into());
    }
    if !git(root, &["status", "--porcelain", "--untracked-files=no"])
        .stdout
        .is_empty()
    {
        return Step::Wait("the worktree has uncommitted changes".into());
    }
    let fetch = git(root, &["fetch", "--quiet", "origin"]);
    if !fetch.ok {
        return Step::Wait(format!("git fetch failed: {}", fetch.stderr));
    }
    let origin_base = format!("origin/{base}");
    let needs_merge = !git(root, &["merge-base", "--is-ancestor", &origin_base, "HEAD"]).ok;
    let remote_branch = format!("origin/{branch}..HEAD");
    let unpushed = git(root, &["rev-list", "--count", &remote_branch])
        .stdout
        .parse::<u64>()
        .map_or(true, |n| n > 0);
    if !needs_merge && !unpushed {
        return Step::UpToDate;
    }

    let mut resolved = Vec::new();
    if needs_merge {
        let merge = git(root, &["merge", "--no-edit", &origin_base]);
        if !merge.ok {
            let unmerged: Vec<String> = git(root, &["diff", "--name-only", "--diff-filter=U"])
                .stdout
                .lines()
                .map(str::to_string)
                .collect();
            if unmerged.is_empty() {
                abort_merge(root);
                let detail = if merge.stderr.is_empty() {
                    merge.stdout
                } else {
                    merge.stderr
                };
                if detail.contains("tell me who you are") || detail.contains("empty ident") {
                    return Step::Skip("git user.name/user.email are not configured".into());
                }
                return Step::Wait(format!("git merge failed: {detail}"));
            }
            let hard: Vec<String> = unmerged
                .iter()
                .filter(|p| !is_changelog_path(p))
                .cloned()
                .collect();
            let blocked = if hard.is_empty() {
                resolve_changelogs(root, &unmerged)
                    .err()
                    .unwrap_or_default()
            } else {
                hard
            };
            if !blocked.is_empty() {
                abort_merge(root);
                return Step::Handoff {
                    reason: format!("conflicts in {}", blocked.join(", ")),
                    prompt: conflict_prompt(base, &blocked),
                    paths: blocked,
                };
            }
            resolved = unmerged;
            let commit = git(root, &["commit", "--no-edit"]);
            if !commit.ok {
                abort_merge(root);
                return Step::Skip(format!("committing the merge failed: {}", commit.stderr));
            }
        }
    }

    let mut verified = Vec::new();
    for v in verify {
        if let Err(output) = run_verify(root, v) {
            let tail = reason_tail(&output);
            return Step::Handoff {
                reason: if tail.is_empty() {
                    format!("verification `{}` failed", v.name)
                } else {
                    format!("verification `{}` failed: {tail}", v.name)
                },
                paths: Vec::new(),
                prompt: verify_prompt(&v.name, &output),
            };
        }
        verified.push(v.name.clone());
    }

    let refspec = format!("HEAD:refs/heads/{branch}");
    let push = git(root, &["push", "--quiet", "origin", &refspec]);
    if !push.ok {
        return Step::Skip(format!("push was refused: {}", push.stderr));
    }
    Step::Pushed {
        head: git(root, &["rev-parse", "HEAD"]).stdout,
        merged: needs_merge,
        resolved,
        verified,
    }
}
