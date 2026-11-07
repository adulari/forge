//! Merge queue: the PRs Forge watches in `merge` mode land one at a time, each tested against
//! the main the previous one produced. See docs/features/merge-queue.md.
//!
//! PRs of one repository (grouped by git common dir) queue in the order their watches were
//! armed. Only the head may merge; every other PR is held by [`Watch::hold`]. Before PR Watch
//! looks at the head, [`before_poll`] brings it up to date: merge `origin/<base>` into the
//! session's worktree as the session's own git identity, resolve changelog-only conflicts,
//! run the project's `verify`/`check` actions, push. Any other conflict, or a failing
//! verification, hands the PR to its session as a turn and moves it out of the way (`skipped`)
//! so the queue behind it keeps going; a new push puts it back at its place.

mod changelog;
mod update;
pub(crate) use update::Verify;
#[cfg(test)]
mod tests;

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::serve_pr_watch::{Mode, Phase, Watch};
use update::{run_step, Step};

/// Seconds after a push before the head may merge, so a PR whose checks have not registered yet
/// is not mistaken for a green one.
const SETTLE_SECS: i64 = 45;
/// Longest wait for GitHub to report the pushed commit as the PR head.
const CATCH_UP_SECS: i64 = 600;
/// Merged and blocked rows kept per repository in the listing.
const FINISHED_ROWS: usize = 8;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum QueueState {
    /// Waiting for the PRs ahead.
    Queued,
    /// Next to land: updating, or being watched by PR Watch.
    Head,
    /// The update was pushed; waiting for its checks.
    Checks,
    /// Handed to its session or refused; out of the way until its branch moves.
    Skipped,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueueInfo {
    pub(crate) repo: String,
    pub(crate) state: QueueState,
    pub(crate) reason: Option<String>,
    pub(crate) pushes: u32,
    pub(crate) last_update: Option<String>,
    skipped_head: Option<String>,
    pushed_head: Option<String>,
    pushed_at: Option<i64>,
}

impl QueueInfo {
    fn new(repo: String) -> Self {
        Self {
            repo,
            state: QueueState::Queued,
            reason: None,
            pushes: 0,
            last_update: None,
            skipped_head: None,
            pushed_head: None,
            pushed_at: None,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Prelude {
    /// A hand-off turn for the session.
    pub(crate) prompt: Option<String>,
    /// The update was pushed this tick; the PR's state is stale until the next poll.
    pub(crate) acted: bool,
}

fn queued(w: &Watch) -> bool {
    w.mode == Mode::Merge && w.state == Phase::Watching && w.queue.is_some()
}

/// The watches queued in `repo`, in order.
fn members<'a>(all: &'a [Watch], repo: &str) -> Vec<&'a Watch> {
    let mut rows: Vec<&Watch> = all
        .iter()
        .filter(|w| queued(w) && w.queue.as_ref().is_some_and(|q| q.repo == repo))
        .collect();
    rows.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then(a.session_id.cmp(&b.session_id))
    });
    rows
}

fn waiting(w: &Watch) -> bool {
    w.queue
        .as_ref()
        .is_some_and(|q| q.state != QueueState::Skipped)
}

fn pr_label(w: &Watch) -> String {
    w.pr_number
        .map_or_else(|| "an earlier PR".to_string(), |n| format!("PR #{n}"))
}

/// One queue per remote repository and base branch: checkouts and worktrees of the same remote
/// share it, a PR into another base does not.
fn repo_key(root: &Path, base: &str) -> String {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let origin = git(&["remote", "get-url", "origin"]).unwrap_or_else(|| {
        let dir = git(&["rev-parse", "--git-common-dir"]).map_or_else(
            || root.to_path_buf(),
            |d| {
                let p = std::path::PathBuf::from(&d);
                if p.is_absolute() {
                    p
                } else {
                    root.join(p)
                }
            },
        );
        dir.canonicalize().unwrap_or(dir).display().to_string()
    });
    format!("{origin} @ {base}")
}

/// The project's `verify` and `check` actions, in declared order.
fn verify_for(root: &Path) -> Vec<Verify> {
    forge_config::project_actions::actions_for(root)
        .into_iter()
        .filter(|a| matches!(a.id.as_str(), "verify" | "check"))
        .map(|a| Verify {
            name: a.name,
            command: a.command,
            cwd: a.cwd,
        })
        .collect()
}

/// Queue bookkeeping and, for the head, the update step. Sets `w.hold` for PR Watch.
pub(crate) async fn before_poll(
    root: &Path,
    w: &mut Watch,
    mut all: Vec<Watch>,
    free: bool,
    now: i64,
    verify: Option<Vec<Verify>>,
) -> Prelude {
    let mut pre = Prelude::default();
    let Some(base) = w.base_branch.clone() else {
        w.hold = Some("waiting for the first PR poll".into());
        return pre;
    };
    let repo = repo_key(root, &base);
    let head_sha = w.head_sha.clone();
    let info = w.queue.get_or_insert_with(|| QueueInfo::new(repo.clone()));
    info.repo = repo.clone();
    if info.state == QueueState::Skipped && head_sha != info.skipped_head {
        info.state = QueueState::Queued;
        info.reason = None;
        info.skipped_head = None;
    }
    if let Some(slot) = all.iter_mut().find(|x| x.session_id == w.session_id) {
        *slot = w.clone();
    }
    let members = members(&all, &repo);
    let ahead: Vec<&Watch> = members
        .iter()
        .copied()
        .filter(|m| waiting(m))
        .take_while(|m| m.session_id != w.session_id)
        .collect();
    let info = w.queue.as_mut().expect("set above");

    if info.state == QueueState::Skipped {
        let reason = info.reason.clone().unwrap_or_default();
        w.hold = Some(format!("out of the merge queue: {reason}"));
        return pre;
    }
    if let Some(first) = ahead.first() {
        let reason = format!(
            "#{} in the merge queue, behind {}",
            ahead.len() + 1,
            pr_label(first)
        );
        info.state = QueueState::Queued;
        info.reason = Some(reason.clone());
        w.hold = Some(reason);
        return pre;
    }

    if let (Some(at), Some(pushed)) = (info.pushed_at, info.pushed_head.clone()) {
        let caught_up = head_sha.as_deref() == Some(pushed.as_str());
        if (!caught_up && now - at < CATCH_UP_SECS) || now - at < SETTLE_SECS {
            info.state = QueueState::Checks;
            info.reason = Some("waiting for checks on the updated branch".into());
            w.hold = info.reason.clone();
            return pre;
        }
        info.pushed_at = None;
        info.pushed_head = None;
    }
    info.state = QueueState::Head;
    info.reason = None;
    w.hold = None;
    if !free {
        info.reason = Some("waiting for the session to go idle".into());
        w.hold = info.reason.clone();
        return pre;
    }

    let dir = root.to_path_buf();
    let step = tokio::task::spawn_blocking(move || {
        let verify = verify.unwrap_or_else(|| verify_for(&dir));
        run_step(&dir, &base, &verify)
    })
    .await
    .unwrap_or_else(|e| Step::Wait(format!("update step failed: {e}")));

    let info = w.queue.as_mut().expect("set above");
    match step {
        Step::UpToDate => {}
        Step::Wait(reason) => {
            info.reason = Some(reason.clone());
            w.hold = Some(reason);
        }
        Step::Skip(reason) => {
            info.state = QueueState::Skipped;
            info.skipped_head = head_sha;
            info.reason = Some(reason.clone());
            w.hold = Some(format!("out of the merge queue: {reason}"));
            w.last_action = Some(format!("skipped by the merge queue: {reason}"));
            pre.acted = true;
        }
        Step::Handoff {
            reason,
            paths: _,
            prompt,
        } => {
            info.state = QueueState::Skipped;
            info.skipped_head = head_sha;
            info.reason = Some(reason.clone());
            w.hold = Some(format!("out of the merge queue: {reason}"));
            w.last_action = Some(format!("handed to its session: {reason}"));
            pre.prompt = Some(prompt);
            pre.acted = true;
        }
        Step::Pushed {
            head,
            merged,
            resolved,
            verified,
        } => {
            info.pushes += 1;
            info.state = QueueState::Checks;
            info.pushed_head = Some(head);
            info.pushed_at = Some(now);
            let mut parts = Vec::new();
            if merged {
                parts.push("merged the base".to_string());
            }
            if !resolved.is_empty() {
                parts.push(format!("resolved {}", resolved.join(", ")));
            }
            parts.push(if verified.is_empty() {
                "no verify action configured".to_string()
            } else {
                format!("verified: {}", verified.join(", "))
            });
            parts.push("pushed".into());
            info.last_update = Some(parts.join("; "));
            info.reason = Some("waiting for checks on the updated branch".into());
            w.hold = info.reason.clone();
            w.last_action = Some(format!("merge queue: {}", parts.join("; ")));
            w.blocked_reason = None;
            pre.acted = true;
        }
    }
    pre
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct QueueRow {
    pub(crate) session_id: String,
    pub(crate) pr_number: Option<u64>,
    pub(crate) pr_url: Option<String>,
    /// 1-based place among the PRs still waiting; `None` for skipped and finished rows.
    pub(crate) position: Option<usize>,
    /// `head` | `queued` | `checks` | `skipped` | `merged` | `blocked`
    pub(crate) state: String,
    pub(crate) reason: Option<String>,
    pub(crate) pushes: u32,
    pub(crate) last_update: Option<String>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct RepoQueue {
    pub(crate) repo: String,
    pub(crate) rows: Vec<QueueRow>,
}

fn state_name(s: QueueState) -> &'static str {
    match s {
        QueueState::Queued => "queued",
        QueueState::Head => "head",
        QueueState::Checks => "checks",
        QueueState::Skipped => "skipped",
    }
}

/// The merge queue as one brief line, or `None` when nothing is queued or recently handed back.
pub(crate) fn brief_line(all: &[Watch]) -> Option<crate::serve_brief::QueueBrief> {
    let rows: Vec<QueueRow> = snapshot(all).into_iter().flat_map(|q| q.rows).collect();
    let waiting = rows.iter().filter(|r| r.position.is_some()).count();
    let skipped = rows.iter().filter(|r| r.state == "skipped").count();
    if waiting + skipped == 0 {
        return None;
    }
    let mut parts = vec![format!("{waiting} waiting")];
    if let Some(next) = rows.iter().find(|r| r.position == Some(1)) {
        let pr = next
            .pr_number
            .map_or_else(|| "a PR".to_string(), |n| format!("PR #{n}"));
        parts.push(format!("next {pr} ({})", next.state));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} handed back"));
    }
    Some(crate::serve_brief::QueueBrief {
        line: format!("Merge queue: {}", parts.join(", ")),
    })
}

/// Every repository's queue: active rows in order, skipped rows after them, then the latest
/// finished ones.
pub(crate) fn snapshot(all: &[Watch]) -> Vec<RepoQueue> {
    let mut repos: Vec<String> = all
        .iter()
        .filter(|w| w.mode == Mode::Merge)
        .filter_map(|w| w.queue.as_ref().map(|q| q.repo.clone()))
        .collect();
    repos.sort();
    repos.dedup();
    repos
        .into_iter()
        .map(|repo| {
            let row = |w: &Watch, position: Option<usize>, state: &str| {
                let q = w.queue.as_ref().expect("filtered on queue");
                QueueRow {
                    session_id: w.session_id.clone(),
                    pr_number: w.pr_number,
                    pr_url: w.pr_url.clone(),
                    position,
                    state: state.to_string(),
                    reason: q.reason.clone().or_else(|| w.blocked_reason.clone()),
                    pushes: q.pushes,
                    last_update: q.last_update.clone(),
                }
            };
            let live = members(all, &repo);
            let mut rows = Vec::new();
            let mut place = 0;
            for w in live.iter().filter(|w| waiting(w)) {
                place += 1;
                let state = w.queue.as_ref().map_or("queued", |q| state_name(q.state));
                rows.push(row(w, Some(place), state));
            }
            for w in live.iter().filter(|w| !waiting(w)) {
                rows.push(row(w, None, "skipped"));
            }
            let mut done: Vec<&Watch> = all
                .iter()
                .filter(|w| {
                    w.mode == Mode::Merge
                        && w.state != Phase::Watching
                        && w.queue.as_ref().is_some_and(|q| q.repo == repo)
                })
                .collect();
            done.sort_by_key(|w| std::cmp::Reverse(w.last_checked));
            done.truncate(FINISHED_ROWS);
            for w in done {
                let state = if w.state == Phase::Merged {
                    "merged"
                } else {
                    "blocked"
                };
                rows.push(row(w, None, state));
            }
            RepoQueue { repo, rows }
        })
        .collect()
}
