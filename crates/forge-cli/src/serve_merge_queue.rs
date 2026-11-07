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
    /// Paused by the owner; out of the way until released.
    Held,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueueInfo {
    pub(crate) repo: String,
    pub(crate) state: QueueState,
    pub(crate) reason: Option<String>,
    pub(crate) pushes: u32,
    pub(crate) last_update: Option<String>,
    /// The owner's hold: the PR keeps its watch but neither lands nor blocks the PRs behind it.
    #[serde(default)]
    pub(crate) held: bool,
    /// Owner-chosen place in line (lower goes first); `None` keeps the order the watches were armed in.
    #[serde(default)]
    rank: Option<i64>,
    /// Bumped by every retry; the poll that sees it ahead of `retried_seq` re-enters a skipped PR.
    #[serde(default)]
    retry_seq: u32,
    #[serde(default)]
    retried_seq: u32,
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
            held: false,
            rank: None,
            retry_seq: 0,
            retried_seq: 0,
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
        order_key(a)
            .cmp(&order_key(b))
            .then(a.session_id.cmp(&b.session_id))
    });
    rows
}

/// Place in line: the owner's move-to-front rank, else the moment the watch was armed.
fn order_key(w: &Watch) -> i64 {
    w.queue
        .as_ref()
        .and_then(|q| q.rank)
        .unwrap_or(w.created_at)
}

fn waiting(w: &Watch) -> bool {
    w.queue
        .as_ref()
        .is_some_and(|q| !matches!(q.state, QueueState::Skipped | QueueState::Held))
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
    if info.retry_seq != info.retried_seq {
        info.retried_seq = info.retry_seq;
        if info.state == QueueState::Skipped {
            info.state = QueueState::Queued;
            info.reason = None;
            info.skipped_head = None;
        }
    }
    if info.state == QueueState::Skipped && head_sha != info.skipped_head {
        info.state = QueueState::Queued;
        info.reason = None;
        info.skipped_head = None;
    }
    if info.held {
        info.state = QueueState::Held;
        info.reason = Some("held by you".into());
    } else if info.state == QueueState::Held {
        info.state = QueueState::Queued;
        info.reason = None;
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

    if info.state == QueueState::Held {
        w.hold = Some("held in the merge queue".into());
        return pre;
    }
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
    /// `head` | `queued` | `checks` | `held` | `skipped` | `merged` | `blocked`
    pub(crate) state: String,
    pub(crate) held: bool,
    pub(crate) reason: Option<String>,
    pub(crate) pushes: u32,
    pub(crate) last_update: Option<String>,
    /// `review clean` / `review: 2 confirmed, fixing` (docs/features/pr-review-gate.md).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) review: Option<String>,
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
        QueueState::Held => "held",
    }
}

/// The merge queue as one brief line, or `None` when nothing is queued or recently handed back.
pub(crate) fn brief_line(all: &[Watch]) -> Option<crate::serve_brief::QueueBrief> {
    let rows: Vec<QueueRow> = snapshot(all).into_iter().flat_map(|q| q.rows).collect();
    let waiting = rows.iter().filter(|r| r.position.is_some()).count();
    let skipped = rows.iter().filter(|r| r.state == "skipped").count();
    let held = rows.iter().filter(|r| r.state == "held").count();
    if waiting + skipped + held == 0 {
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
    if held > 0 {
        parts.push(format!("{held} held"));
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
                    held: q.held,
                    reason: q.reason.clone().or_else(|| w.blocked_reason.clone()),
                    pushes: q.pushes,
                    last_update: q.last_update.clone(),
                    review: w.review.as_ref().map(|r| r.summary()),
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
                let state = w.queue.as_ref().map_or("skipped", |q| state_name(q.state));
                rows.push(row(w, None, state));
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

/// What the owner can do to a queued PR.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Action {
    /// Take the PR out of the line without dropping its watch.
    Hold,
    /// Put a held PR back at its place.
    Release,
    /// Make the PR the next to land.
    Front,
    /// Re-enter a handed-back PR without waiting for a new push.
    Retry,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ActionError {
    NotFound(String),
    Conflict(String),
}

/// The session a request names: its id, or a PR number when exactly one queued watch has it.
pub(crate) fn resolve(
    all: &std::collections::HashMap<String, Watch>,
    session: Option<&str>,
    pr: Option<u64>,
) -> Result<String, ActionError> {
    if let Some(id) = session {
        return all
            .contains_key(id)
            .then(|| id.to_string())
            .ok_or_else(|| ActionError::NotFound(format!("no PR watch for session {id}")));
    }
    let Some(number) = pr else {
        return Err(ActionError::Conflict(
            "name the PR with `pr` or the session with `session`".into(),
        ));
    };
    let mut ids: Vec<&str> = all
        .values()
        .filter(|w| w.mode == Mode::Merge && w.queue.is_some() && w.pr_number == Some(number))
        .map(|w| w.session_id.as_str())
        .collect();
    ids.sort_unstable();
    match ids.as_slice() {
        [] => Err(ActionError::NotFound(format!(
            "PR #{number} is not in a merge queue"
        ))),
        [one] => Ok((*one).to_string()),
        many => Err(ActionError::Conflict(format!(
            "PR #{number} is in more than one queue; name the session: {}",
            many.join(", ")
        ))),
    }
}

/// Apply an owner action to one watch. The poll that follows picks it up; the fields set here
/// are the ones [`carry_owner_edits`] keeps when a poll that started earlier is stored.
pub(crate) fn apply(
    all: &mut std::collections::HashMap<String, Watch>,
    session: &str,
    action: Action,
) -> Result<String, ActionError> {
    let w = all
        .get(session)
        .ok_or_else(|| ActionError::NotFound(format!("no PR watch for session {session}")))?;
    let label = pr_label(w);
    if w.mode != Mode::Merge || w.state != Phase::Watching {
        return Err(ActionError::Conflict(format!(
            "{label} is not waiting in a merge queue"
        )));
    }
    let Some(info) = w.queue.as_ref() else {
        return Err(ActionError::Conflict(format!(
            "{label} has not joined the queue yet; it does on its next PR Watch poll"
        )));
    };
    let (repo, state, held) = (info.repo.clone(), info.state, info.held);
    match action {
        Action::Hold if held => {
            return Err(ActionError::Conflict(format!("{label} is already held")))
        }
        Action::Release if !held => {
            return Err(ActionError::Conflict(format!("{label} is not held")))
        }
        Action::Retry if state != QueueState::Skipped => {
            return Err(ActionError::Conflict(format!(
                "{label} was not handed back; only a skipped PR can be retried"
            )))
        }
        _ => {}
    }
    let front_rank = (action == Action::Front).then(|| {
        let own = order_key(w);
        let others: Vec<i64> = members(&all.values().cloned().collect::<Vec<_>>(), &repo)
            .into_iter()
            .filter(|m| m.session_id != session && waiting(m))
            .map(order_key)
            .collect();
        others
            .iter()
            .any(|k| *k <= own)
            .then(|| others.iter().min().map_or(own, |k| k - 1))
    });
    if front_rank == Some(None) {
        return Ok(format!("{label} is already at the front"));
    }
    let w = all.get_mut(session).expect("checked above");
    let info = w.queue.as_mut().expect("checked above");
    let message = match action {
        Action::Hold => {
            info.held = true;
            info.state = QueueState::Held;
            info.reason = Some("held by you".into());
            w.hold = Some("held in the merge queue".into());
            format!("{label} held; the queue goes on without it")
        }
        Action::Release => {
            info.held = false;
            info.state = QueueState::Queued;
            info.reason = None;
            w.last_checked = None;
            format!("{label} released; it takes its place again")
        }
        Action::Front => {
            info.rank = front_rank.flatten();
            w.last_checked = None;
            format!("{label} moved to the front")
        }
        Action::Retry => {
            info.retry_seq += 1;
            info.retried_seq = info.retry_seq;
            info.state = QueueState::Queued;
            info.reason = None;
            info.skipped_head = None;
            w.last_checked = None;
            format!("{label} re-entered the queue")
        }
    };
    Ok(message)
}

/// A poll computes from a copy taken before it ran; what the owner chose meanwhile wins.
pub(crate) fn carry_owner_edits(live: &Watch, next: &mut Watch) {
    if let (Some(live), Some(next)) = (live.queue.as_ref(), next.queue.as_mut()) {
        next.held = live.held;
        next.rank = live.rank;
        next.retry_seq = live.retry_seq;
    }
}
