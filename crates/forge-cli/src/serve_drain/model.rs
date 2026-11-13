//! The drain's data and its scheduling decision. Pure: everything the decision reads is in
//! [`Inputs`], so the tests drive it with fabricated sessions, watches and quota readings.

use std::collections::HashMap;

use forge_types::goal_envelope::{EnvelopeCheck, EnvelopeState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::*;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ItemStatus {
    Queued,
    /// A session is working on it and holds one of the drain's slots.
    Running,
    /// The session stopped on a permission prompt or a question for the owner. It still holds
    /// its slot: answering resumes it, so the queue must not run ahead of unanswered prompts.
    Waiting,
    /// A pull request exists and PR Watch is carrying it.
    PrOpen,
    Merged,
    /// The session went idle without a pull request.
    Done,
    Failed,
    Blocked,
    Cancelled,
}

impl ItemStatus {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Merged | Self::Done | Self::Failed | Self::Blocked | Self::Cancelled
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DrainStatus {
    Active,
    /// The envelope is spent; items start again when the window resets.
    Paused,
    /// Nothing is queued or running.
    Finished,
    Cancelled,
}

impl DrainStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Finished => "finished",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub(crate) number: u64,
    pub(crate) status: ItemStatus,
    pub(crate) session_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) pr_url: Option<String>,
    pub(crate) outcome: Option<String>,
    pub(crate) started_at: Option<i64>,
    pub(crate) finished_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct Drain {
    pub(crate) id: String,
    /// The project checkout the issues belong to.
    pub(crate) cwd: String,
    pub(crate) parallel: usize,
    pub(crate) worktree: bool,
    pub(crate) mode: Mode,
    pub(crate) method: String,
    /// The budget as the caller wrote it (`claude:30%w`).
    pub(crate) budget: Option<String>,
    pub(crate) envelope: Option<EnvelopeState>,
    pub(crate) status: DrainStatus,
    pub(crate) paused_reason: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) items: Vec<Item>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Counts {
    pub(crate) total: usize,
    pub(crate) started: usize,
    pub(crate) queued: usize,
    pub(crate) running: usize,
    /// Sessions stopped on a prompt or question; they keep their slot like running ones.
    pub(crate) prompted: usize,
    pub(crate) merged: usize,
    /// Needs the owner: a prompt to answer, a PR to merge, or work with no PR to look at.
    pub(crate) waiting: usize,
    pub(crate) blocked: usize,
}

/// What the registry says about a drain item's session right now.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LiveSession {
    pub(crate) busy: bool,
    pub(crate) closed: bool,
    pub(crate) waiting: bool,
    pub(crate) turns_finished: u64,
    pub(crate) failed: bool,
}

/// The part of a PR Watch the drain reads.
#[derive(Debug, Clone, Default)]
pub(crate) struct WatchView {
    pub(crate) merged: bool,
    pub(crate) blocked: Option<String>,
    pub(crate) pr_number: Option<u64>,
    pub(crate) pr_url: Option<String>,
}

impl WatchView {
    pub(crate) fn of(watch: &Watch) -> Self {
        Self {
            merged: watch.state == Phase::Merged,
            blocked: (watch.state == Phase::Blocked).then(|| {
                watch
                    .blocked_reason
                    .clone()
                    .unwrap_or_else(|| "PR Watch is blocked".into())
            }),
            pr_number: watch.pr_number,
            pr_url: watch.pr_url.clone(),
        }
    }
}

pub(crate) struct Inputs<'a> {
    pub(crate) live: &'a HashMap<String, LiveSession>,
    pub(crate) watches: &'a HashMap<String, WatchView>,
    pub(crate) sources: &'a EnvelopeSources,
    /// What the drain's sessions have cost so far, for a `$` envelope.
    pub(crate) spent_usd: f64,
    pub(crate) now: i64,
}

/// How long after going idle an item with no pull request is still watched for one: PR Watch
/// polls every 30 seconds, so a PR opened in the last turn shows up a little later.
const PR_LOOKUP_SECS: i64 = 10 * 60;

impl Counts {
    /// Items that hold one of the drain's slots.
    pub(crate) fn occupied(&self) -> usize {
        self.running + self.prompted
    }
}

impl Drain {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: String,
        cwd: String,
        numbers: &[u64],
        parallel: usize,
        budget: Option<String>,
        envelope: Option<EnvelopeState>,
        (worktree, mode, method): (bool, Mode, String),
        now: i64,
    ) -> Self {
        Self {
            id,
            cwd,
            parallel,
            worktree,
            mode,
            method,
            budget,
            envelope,
            status: DrainStatus::Active,
            paused_reason: None,
            created_at: now,
            updated_at: now,
            items: numbers
                .iter()
                .map(|&number| Item {
                    number,
                    status: ItemStatus::Queued,
                    session_id: None,
                    title: None,
                    pr_url: None,
                    outcome: None,
                    started_at: None,
                    finished_at: None,
                })
                .collect(),
        }
    }

    fn observing(item: &Item, now: i64) -> bool {
        match item.status {
            ItemStatus::Running | ItemStatus::Waiting | ItemStatus::PrOpen => true,
            ItemStatus::Done => item.finished_at.is_none_or(|at| now - at < PR_LOOKUP_SECS),
            _ => false,
        }
    }

    /// Whether the loop has anything to do for this drain.
    pub(crate) fn wants_scheduling(&self) -> bool {
        let now = now_secs();
        matches!(self.status, DrainStatus::Active | DrainStatus::Paused)
            || self.items.iter().any(|i| Self::observing(i, now))
    }

    pub(crate) fn is_settled(&self) -> bool {
        !self.wants_scheduling()
    }

    pub(crate) fn counts(&self) -> Counts {
        let mut c = Counts {
            total: self.items.len(),
            ..Counts::default()
        };
        for item in &self.items {
            c.started += usize::from(item.session_id.is_some());
            match item.status {
                ItemStatus::Queued => c.queued += 1,
                ItemStatus::Running => c.running += 1,
                ItemStatus::Merged => c.merged += 1,
                ItemStatus::Waiting => {
                    c.prompted += 1;
                    c.waiting += 1;
                }
                ItemStatus::Done => c.waiting += 1,
                ItemStatus::PrOpen => c.waiting += usize::from(self.mode != Mode::Merge),
                ItemStatus::Failed | ItemStatus::Blocked => c.blocked += 1,
                ItemStatus::Cancelled => {}
            }
        }
        c
    }

    /// `Drain: 7 started, 2 running, 4 merged, 2 waiting on you, 1 blocked`.
    pub(crate) fn line(&self) -> String {
        let c = self.counts();
        let mut parts = vec![format!("{} started", c.started)];
        for (n, label) in [
            (c.running, "running"),
            (c.merged, "merged"),
            (c.waiting, "waiting on you"),
            (c.blocked, "blocked"),
            (c.queued, "queued"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {label}"));
            }
        }
        let tail = match self.status {
            DrainStatus::Paused => " (paused)",
            DrainStatus::Cancelled => " (cancelled)",
            _ => "",
        };
        format!("Drain: {}{tail}", parts.join(", "))
    }

    pub(crate) fn record_start(
        &mut self,
        number: u64,
        outcome: Result<(String, String), String>,
        now: i64,
    ) {
        let Some(item) = self.items.iter_mut().find(|i| i.number == number) else {
            return;
        };
        item.started_at = Some(now);
        match outcome {
            Ok((session_id, title)) => {
                item.status = ItemStatus::Running;
                item.session_id = Some(session_id);
                item.title = Some(title).filter(|t| !t.is_empty());
            }
            Err(message) => {
                item.status = ItemStatus::Failed;
                item.outcome = Some(message);
                item.finished_at = Some(now);
            }
        }
        self.updated_at = now;
    }

    /// Stop starting items. Sessions already running are not touched.
    pub(crate) fn cancel(&mut self, now: i64) {
        if matches!(self.status, DrainStatus::Cancelled) {
            return;
        }
        for item in self
            .items
            .iter_mut()
            .filter(|i| i.status == ItemStatus::Queued)
        {
            item.status = ItemStatus::Cancelled;
        }
        self.status = DrainStatus::Cancelled;
        self.paused_reason = None;
        self.updated_at = now;
    }

    pub(crate) fn to_json(&self) -> Value {
        let envelope = self.envelope.as_ref().map(|e| {
            json!({
                "meter": e.meter(),
                "label": e.spec.label(),
                "used": e.used,
                "slice": e.spec.slice,
                "fill": e.fill(),
                "spent": e.spent,
                "unverified": e.unverified,
                "resets_at": e.resets_at,
                "resume_label": e.resume_label,
            })
        });
        json!({
            "id": self.id,
            "cwd": self.cwd,
            "status": self.status.as_str(),
            "parallel": self.parallel,
            "budget": self.budget,
            "mode": self.mode,
            "method": self.method,
            "worktree": self.worktree,
            "paused_reason": self.paused_reason,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "line": self.line(),
            "counts": self.counts(),
            "envelope": envelope,
            "items": self.items.iter().map(|i| json!({
                "number": i.number,
                "status": i.status,
                "session_id": i.session_id,
                "title": i.title,
                "pr_url": i.pr_url,
                "outcome": i.outcome,
                "started_at": i.started_at,
                "finished_at": i.finished_at,
            })).collect::<Vec<_>>(),
        })
    }
}

fn watch_verdict(item: &mut Item, watch: &WatchView) -> Option<ItemStatus> {
    if watch.pr_url.is_some() {
        item.pr_url.clone_from(&watch.pr_url);
    }
    if watch.merged {
        return Some(ItemStatus::Merged);
    }
    if let Some(reason) = &watch.blocked {
        item.outcome = Some(reason.clone());
        return Some(ItemStatus::Blocked);
    }
    watch.pr_number.map(|_| ItemStatus::PrOpen)
}

fn observe(item: &mut Item, inp: &Inputs) {
    if !Drain::observing(item, inp.now) {
        return;
    }
    let Some(session) = item.session_id.clone() else {
        return;
    };
    let watch = inp.watches.get(&session).cloned().unwrap_or_default();
    let live = inp.live.get(&session).filter(|l| !l.closed);
    let was = item.status;
    let next = match was {
        // A session that is idle again (a PR Watch fix turn) never takes its slot back.
        ItemStatus::PrOpen | ItemStatus::Done => watch_verdict(item, &watch).unwrap_or(was),
        _ => match live {
            None => watch_verdict(item, &watch).unwrap_or_else(|| {
                item.outcome = Some("the session ended before it opened a pull request".into());
                ItemStatus::Blocked
            }),
            Some(l) if l.waiting => ItemStatus::Waiting,
            Some(l)
                if l.busy
                    || (l.turns_finished == 0
                        && item
                            .started_at
                            .is_some_and(|at| inp.now - at < START_GRACE_SECS)) =>
            {
                ItemStatus::Running
            }
            Some(l) if l.failed => {
                item.outcome = Some("the last turn failed".into());
                ItemStatus::Failed
            }
            Some(_) => watch_verdict(item, &watch).unwrap_or(ItemStatus::Done),
        },
    };
    item.status = next;
    if next != ItemStatus::Running && next != ItemStatus::Waiting && item.finished_at.is_none() {
        item.finished_at = Some(inp.now);
    }
}

/// One scheduling decision: bring every item up to date, apply the envelope, and say which queued
/// issues to start now (never more than the free slots).
pub(crate) fn advance(drain: &mut Drain, inp: &Inputs) -> Vec<u64> {
    let before = drain.clone();
    let numbers = decide(drain, inp);
    if *drain != before {
        drain.updated_at = inp.now;
    }
    numbers
}

fn decide(drain: &mut Drain, inp: &Inputs) -> Vec<u64> {
    for item in &mut drain.items {
        observe(item, inp);
    }
    if matches!(drain.status, DrainStatus::Finished | DrainStatus::Cancelled) {
        return Vec::new();
    }
    let counts = drain.counts();
    if counts.queued == 0 && counts.occupied() == 0 {
        drain.status = DrainStatus::Finished;
        drain.paused_reason = None;
        return Vec::new();
    }
    if let Some(env) = drain.envelope.as_mut() {
        if drain.status == DrainStatus::Paused && envelope::resume_gate(env, inp.now).is_err() {
            return Vec::new();
        }
        let check = if env.is_usd() {
            env.observe_usd(inp.spent_usd)
        } else {
            env.observe_quota(inp.sources.quota(&env.spec), inp.now)
        };
        match check {
            EnvelopeCheck::Spent => {
                envelope::mark_waiting(env, inp.now);
                drain.paused_reason = Some(spent_message(env, inp.now));
                drain.status = DrainStatus::Paused;
                return Vec::new();
            }
            // No fresh reading: hold the queue this tick without declaring the drain paused.
            EnvelopeCheck::Unverified => return Vec::new(),
            EnvelopeCheck::Within => {
                drain.status = DrainStatus::Active;
                drain.paused_reason = None;
            }
        }
    }
    let free = drain.parallel.saturating_sub(counts.occupied());
    drain
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Queued)
        .take(free)
        .map(|i| i.number)
        .collect()
}

fn spent_message(env: &EnvelopeState, now: i64) -> String {
    let phrase = env.slice_phrase();
    match env.auto_resume_at() {
        Some(at) => format!(
            "Envelope spent ({phrase}). The drain carries on at {} when the window resets.",
            envelope::resume_clock(at, now)
        ),
        None => format!(
            "Envelope spent ({phrase}). Cancel the drain, or start a new one with a larger budget."
        ),
    }
}
