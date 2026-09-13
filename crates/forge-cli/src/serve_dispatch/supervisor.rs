//! The dispatch supervisor: one daemon task that notices dispatched workers finishing a turn or
//! going away, records it, runs the scheduler, and tells the coordinator.
//!
//! The decisions are pure ([`observe`], [`compose_messages`], [`is_done`]) over the stored rows and
//! what each worker's snapshot shows; [`run_supervisor`] only gathers inputs and applies outputs.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_core::dispatch::{self as plan, dispatch_status, item_status};
use forge_store::DispatchRow;

use super::{advance_locked, dispatch_lock, send_to_coordinator, Advance};
use crate::remote::SnapTranscriptRow;
use crate::serve::DaemonState;

/// Upper bound on how long a finished turn waits to be noticed when the fleet channel is quiet.
const TICK: Duration = Duration::from_millis(500);
/// Streaming sessions invalidate the fleet every ~500 ms; this keeps passes from stacking up.
const MIN_PASS_GAP: Duration = Duration::from_millis(100);
/// A running item whose session has no live handle this long is `stopped`. A merge's respawn
/// window is far shorter; an archived or crashed session never comes back.
pub(super) const MISSING_GRACE: Duration = Duration::from_secs(10);
/// Finished dispatches still watched, so a worker the user prompts again updates its item.
const RECENT_DISPATCHES: usize = 50;

/// Item statuses whose session is still worth watching.
const WATCHED: [&str; 4] = [
    item_status::RUNNING,
    item_status::SUCCEEDED,
    item_status::FAILED,
    item_status::STOPPED,
];

/// What a worker's live driver shows right now.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Observation {
    /// Identity of the driver handle; a different value means a new driver whose counter restarted.
    pub(super) handle_key: usize,
    pub(super) turns_finished: u64,
    pub(super) busy: bool,
    pub(super) waiting: bool,
    pub(super) outcome: Option<String>,
    pub(super) stop_reason: Option<String>,
    pub(super) final_reply: String,
}

/// Per-item memory across passes. Not persisted: after a daemon restart every driver's counter
/// restarts at 0 too, so a fresh baseline of 0 is exact.
#[derive(Debug, Default)]
pub(super) struct Tracker {
    session_id: String,
    handle_key: usize,
    baseline: u64,
    missing_since: Option<Instant>,
    /// Whether `handle_key`/`baseline` came from a real observation. A tracker is recreated whenever
    /// the supervisor lost it (a failed store read, a dispatch leaving the recent window); seeding
    /// it from the worker's CURRENT counter is what stops a turn that was already reported from
    /// being reported to the coordinator a second time.
    seeded: bool,
}

pub(super) type Trackers = HashMap<(String, i64), Tracker>;

/// One item update decided by [`observe`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ItemChange {
    pub(super) idx: i64,
    pub(super) status: &'static str,
    pub(super) outcome: String,
    pub(super) stop_reason: Option<String>,
    pub(super) final_reply: String,
    pub(super) session_id: String,
}

/// Decide which items finished a turn or lost their session, given `live` (session id → what its
/// driver shows; absent = no live handle).
pub(super) fn observe(
    row: &DispatchRow,
    live: &HashMap<String, Observation>,
    trackers: &mut Trackers,
    now: Instant,
) -> Vec<ItemChange> {
    let mut changes = Vec::new();
    for item in &row.items {
        let Some(session_id) = item.session_id.as_deref() else {
            continue;
        };
        if !WATCHED.contains(&item.status.as_str()) {
            continue;
        }
        let tracker = trackers.entry((row.id.clone(), item.idx)).or_default();
        if tracker.session_id != session_id {
            *tracker = Tracker {
                session_id: session_id.to_string(),
                ..Tracker::default()
            };
        }
        match live.get(session_id) {
            Some(obs) => {
                tracker.missing_since = None;
                if !tracker.seeded {
                    // A running item's turns are all still unreported (its driver started at 0);
                    // a finished item's current count is already accounted for.
                    tracker.seeded = true;
                    tracker.handle_key = obs.handle_key;
                    tracker.baseline = if item.status == item_status::RUNNING {
                        0
                    } else {
                        obs.turns_finished
                    };
                } else if obs.handle_key != tracker.handle_key
                    || obs.turns_finished < tracker.baseline
                {
                    tracker.handle_key = obs.handle_key;
                    tracker.baseline = 0;
                }
                if obs.turns_finished > tracker.baseline && !obs.busy && !obs.waiting {
                    tracker.baseline = obs.turns_finished;
                    let succeeded = obs.outcome.as_deref() == Some("success");
                    changes.push(ItemChange {
                        idx: item.idx,
                        status: if succeeded {
                            item_status::SUCCEEDED
                        } else {
                            item_status::FAILED
                        },
                        outcome: obs
                            .stop_reason
                            .clone()
                            .or_else(|| obs.outcome.clone())
                            .unwrap_or_else(|| "failed".to_string()),
                        stop_reason: obs.stop_reason.clone(),
                        final_reply: obs.final_reply.clone(),
                        session_id: session_id.to_string(),
                    });
                }
            }
            None if item.status == item_status::RUNNING => {
                let since = *tracker.missing_since.get_or_insert(now);
                if now.duration_since(since) >= MISSING_GRACE {
                    changes.push(ItemChange {
                        idx: item.idx,
                        status: item_status::STOPPED,
                        outcome: "session_ended".to_string(),
                        stop_reason: Some("session_ended".to_string()),
                        final_reply: String::new(),
                        session_id: session_id.to_string(),
                    });
                }
            }
            None => {}
        }
    }
    changes
}

/// Whether a running dispatch just reached the end: every item terminal.
pub(super) fn is_done(after: &DispatchRow) -> bool {
    after.status == dispatch_status::RUNNING
        && plan::all_finished(after.items.iter().map(|i| i.status.as_str()))
}

/// Whether `item` waits on `on`, directly or through other items.
fn depends_on(row: &DispatchRow, item: i64, on: i64) -> bool {
    let mut stack = vec![item];
    let mut seen = BTreeSet::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current) {
            continue;
        }
        let Some(it) = row.items.iter().find(|i| i.idx == current) else {
            continue;
        };
        if it.depends_on.contains(&on) {
            return true;
        }
        stack.extend(it.depends_on.iter().copied());
    }
    false
}

/// The coordinator messages for one pass over a running dispatch: one report per item that
/// finished (or failed to start), each listing the items cancelled because of it, then the
/// all-finished message when `after` is done. `after` is the row after this pass's writes.
pub(super) fn compose_messages(
    after: &DispatchRow,
    changes: &[ItemChange],
    advanced: &Advance,
) -> Vec<String> {
    struct Report<'a> {
        idx: i64,
        session_id: &'a str,
        outcome: &'a str,
        stop_reason: Option<&'a str>,
        reply: &'a str,
    }
    let title = |idx: i64| {
        after
            .items
            .iter()
            .find(|i| i.idx == idx)
            .map_or("", |i| i.title.as_str())
    };
    let count = |status: &str| after.items.iter().filter(|i| i.status == status).count();
    let (still_running, still_waiting) = (count(item_status::RUNNING), count(item_status::QUEUED));
    let mut reports: Vec<Report<'_>> = changes
        .iter()
        .map(|c| Report {
            idx: c.idx,
            session_id: &c.session_id,
            outcome: if c.status == item_status::SUCCEEDED {
                "success"
            } else {
                "failed"
            },
            stop_reason: c.stop_reason.as_deref(),
            reply: &c.final_reply,
        })
        .collect();
    reports.extend(advanced.start_failed.iter().map(|(idx, error)| Report {
        idx: *idx,
        session_id: "",
        outcome: "failed",
        stop_reason: Some(error.as_str()),
        reply: "",
    }));
    let mut cancelled_for: Vec<Vec<(usize, &str)>> = vec![Vec::new(); reports.len()];
    for cancelled in &advanced.cancelled {
        let owner = reports
            .iter()
            .position(|r| depends_on(after, *cancelled, r.idx))
            .or(reports.len().checked_sub(1));
        if let Some(owner) = owner {
            cancelled_for[owner].push((*cancelled as usize, title(*cancelled)));
        }
    }
    let mut out: Vec<String> = reports
        .iter()
        .zip(&cancelled_for)
        .map(|(r, cancelled)| {
            plan::item_finished_message(&plan::FinishedReport {
                index: r.idx as usize,
                total: after.items.len(),
                title: title(r.idx),
                session_id: r.session_id,
                outcome: r.outcome,
                stop_reason: r.stop_reason,
                last_reply: r.reply,
                still_running,
                still_waiting,
                cancelled,
            })
        })
        .collect();
    if is_done(after) {
        let rows: Vec<(usize, &str, &str)> = after
            .items
            .iter()
            .map(|i| (i.idx as usize, i.title.as_str(), i.status.as_str()))
            .collect();
        out.push(plan::all_finished_message(&rows));
    }
    out
}

/// A worker's final reply: the last assistant block of the transcript, without the rows the TUI
/// adds around it. A finished turn's tail looks like
/// `assistant "  ⚒ forge"`, `assistant "  Done — …"`, `system "  ※ recap …"`: the reply is followed
/// by system rows (recap, completeness and nudge notices), and each reply opens with a `⚒ forge`
/// header row. So: skip trailing system rows, collect the contiguous assistant rows before them,
/// drop the header rows, and remove the TUI's indentation.
pub(super) fn final_reply(rows: &[SnapTranscriptRow]) -> String {
    let is_header = |r: &SnapTranscriptRow| r.text.trim() == "⚒ forge";
    // Only system rows (recap, completeness and nudge notices) may follow the reply. A turn whose
    // last real activity is a tool row ended on a call: the text before it was narration.
    let Some(end) = rows
        .iter()
        .rposition(|r| r.kind != "system" && !r.text.trim().is_empty())
    else {
        return String::new();
    };
    if rows[end].kind != "assistant" {
        return String::new();
    }
    let start = rows[..=end]
        .iter()
        .rposition(|r| r.kind != "assistant")
        .map_or(0, |i| i + 1);
    rows[start..=end]
        .iter()
        .filter(|r| !is_header(r))
        .map(|r| r.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Forget trackers for dispatches no longer supervised — but only when both store reads succeeded.
/// A transient SQLITE_BUSY must not look like "every dispatch disappeared".
pub(super) fn prune_trackers(trackers: &mut Trackers, ids: &BTreeSet<String>, reads_ok: bool) {
    if reads_ok {
        trackers.retain(|(id, _), _| ids.contains(id));
    }
}

pub(crate) async fn run_supervisor(state: Arc<DaemonState>) {
    let mut fleet = state.registry.subscribe_fleet();
    let mut trackers = Trackers::new();
    loop {
        supervise(&state, &mut trackers).await;
        tokio::select! {
            () = tokio::time::sleep(TICK) => {}
            changed = fleet.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
        tokio::time::sleep(MIN_PASS_GAP).await;
    }
}

async fn supervise(state: &DaemonState, trackers: &mut Trackers) {
    let mut ids = BTreeSet::new();
    let mut reads_ok = true;
    let active = state.store.active_dispatches();
    reads_ok &= active.is_ok();
    if let Ok(active) = active {
        ids.extend(
            active
                .into_iter()
                .filter(|d| d.status == dispatch_status::RUNNING)
                .map(|d| d.id),
        );
    }
    let recent = state.store.list_dispatches(RECENT_DISPATCHES);
    reads_ok &= recent.is_ok();
    if let Ok(recent) = recent {
        ids.extend(
            recent
                .into_iter()
                .filter(|d| {
                    d.items
                        .iter()
                        .any(|i| i.session_id.is_some() && WATCHED.contains(&i.status.as_str()))
                })
                .map(|d| d.id),
        );
    }
    prune_trackers(trackers, &ids, reads_ok);
    for id in ids {
        supervise_one(state, &id, trackers).await;
    }
}

async fn supervise_one(state: &DaemonState, id: &str, trackers: &mut Trackers) {
    let lock = dispatch_lock(id);
    let _held = lock.lock().await;
    let Ok(Some(row)) = state.store.dispatch(id) else {
        return;
    };
    let live = live_observations(state, &row).await;
    let changes = observe(&row, &live, trackers, Instant::now());
    let mut changed = false;
    for c in &changes {
        changed |= state
            .store
            .set_dispatch_item(id, c.idx, c.status, None, Some(&c.outcome))
            .is_ok();
    }
    if row.status != dispatch_status::RUNNING {
        if changed {
            state.registry.notify_fleet();
        }
        return;
    }
    let advanced = match advance_locked(state, id).await {
        Ok(advanced) => advanced,
        Err(error) => {
            tracing::warn!(dispatch = %id, ?error, "dispatch: scheduling pass failed");
            Advance::default()
        }
    };
    changed |= !advanced.is_empty();
    let Ok(Some(after)) = state.store.dispatch(id) else {
        return;
    };
    for text in compose_messages(&after, &changes, &advanced) {
        send_to_coordinator(
            &state.store,
            &state.registry,
            &after.coordinator_session_id,
            &text,
        )
        .await;
    }
    if is_done(&after) {
        changed |= state
            .store
            .set_dispatch_status(id, dispatch_status::DONE)
            .is_ok();
    }
    if changed {
        state.registry.notify_fleet();
    }
}

async fn live_observations(state: &DaemonState, row: &DispatchRow) -> HashMap<String, Observation> {
    let mut live = HashMap::new();
    for item in &row.items {
        let Some(session_id) = item.session_id.as_deref() else {
            continue;
        };
        if !WATCHED.contains(&item.status.as_str()) {
            continue;
        }
        let Some(handle) = state.registry.get(session_id).await else {
            continue;
        };
        let snapshot = handle.snapshot_rx.borrow().snapshot.clone();
        // A closed frame is a driver on its way out: treat it as gone, not as a finished turn.
        if snapshot.closed {
            continue;
        }
        live.insert(
            session_id.to_string(),
            Observation {
                handle_key: Arc::as_ptr(&handle) as usize,
                turns_finished: snapshot.turns_finished,
                busy: snapshot.busy,
                waiting: snapshot.permission_prompt.is_some() || snapshot.question.is_some(),
                final_reply: final_reply(&snapshot.transcript_rows),
                outcome: snapshot.last_turn_outcome,
                stop_reason: snapshot.last_stop_reason,
            },
        );
    }
    live
}

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod tests;
