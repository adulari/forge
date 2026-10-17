//! A revised split that the coordinator never answers.
//!
//! `POST .../revise` moves a proposed dispatch back to `planning` and messages the coordinator,
//! and only the coordinator's next proposal moves it forward again. A coordinator that ends its
//! turn without proposing (it asked a question in prose, hit an error, was archived, or the daemon
//! lost it) left the dispatch in `planning` forever: it could no longer be approved or revised,
//! only cancelled. The previous proposal is still stored, so the supervisor hands the dispatch
//! back to the user as `proposed` once the coordinator has been quiet for [`QUIET_GRACE`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

use forge_core::dispatch::dispatch_status;
use forge_store::DispatchRow;

use super::{dispatch_lock, DaemonState};

/// How long the coordinator must stay idle, with nothing queued for it, before a revision counts
/// as unanswered. Far above the gap between a delivery and the driver marking itself busy.
pub(super) const QUIET_GRACE: Duration = Duration::from_secs(8);

/// What the coordinator's driver shows right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Coordinator {
    /// No live driver (archived, crashed, or not yet restored after a restart).
    Missing,
    Busy,
    Idle,
}

pub(super) type Quiet = HashMap<String, Instant>;

/// Whether `row`'s revision has gone unanswered. `queued` says a dispatch message still waits for
/// the coordinator. A dispatch with no stored items is the first plan, not a revision, and is left
/// alone: there is no earlier split to fall back to.
pub(super) fn revision_unanswered(
    row: &DispatchRow,
    coordinator: Coordinator,
    queued: bool,
    quiet: &mut Quiet,
    now: Instant,
) -> bool {
    let revising = row.status == dispatch_status::PLANNING && !row.items.is_empty();
    let quiet_now = match coordinator {
        Coordinator::Busy => false,
        Coordinator::Idle => !queued,
        Coordinator::Missing => true,
    };
    if !(revising && quiet_now) {
        quiet.remove(&row.id);
        return false;
    }
    let since = *quiet.entry(row.id.clone()).or_insert(now);
    now.duration_since(since) >= QUIET_GRACE
}

/// Hand every unanswered revision back to the user as `proposed`. Returns whether anything changed.
pub(super) async fn reclaim_stalled(
    state: &DaemonState,
    rows: &[DispatchRow],
    quiet: &mut Quiet,
) -> bool {
    let mut changed = false;
    for row in rows {
        let coordinator = match state.registry.get(&row.coordinator_session_id).await {
            None => Coordinator::Missing,
            Some(handle) => {
                let snapshot = handle.snapshot_rx.borrow().snapshot.clone();
                if snapshot.closed {
                    Coordinator::Missing
                } else if snapshot.busy
                    || snapshot.permission_prompt.is_some()
                    || snapshot.question.is_some()
                {
                    Coordinator::Busy
                } else {
                    Coordinator::Idle
                }
            }
        };
        let queued = state
            .store
            .pending_fleet_messages_for(&row.coordinator_session_id)
            .is_ok_and(|pending| !pending.is_empty());
        if !revision_unanswered(row, coordinator, queued, quiet, Instant::now()) {
            continue;
        }
        let lock = dispatch_lock(&row.id);
        let _held = lock.lock().await;
        // Re-read under the lock: a proposal may have landed since the pass began.
        let Ok(Some(current)) = state.store.dispatch(&row.id) else {
            continue;
        };
        if current.status == dispatch_status::PLANNING
            && state
                .store
                .set_dispatch_status(&row.id, dispatch_status::PROPOSED)
                .is_ok()
        {
            tracing::info!(dispatch = %row.id, "dispatch: revision unanswered, split handed back as proposed");
            quiet.remove(&row.id);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_store::DispatchItemRow;

    fn row(status: &str, items: usize) -> DispatchRow {
        DispatchRow {
            id: "d1".into(),
            coordinator_session_id: "coord".into(),
            cwd: "/repo".into(),
            prompt: "split it".into(),
            summary: "plan".into(),
            status: status.into(),
            worktree: true,
            permission_mode: None,
            max_running: 4,
            max_items: 8,
            created_at: 0,
            updated_at: 0,
            items: (1..=items as i64)
                .map(|idx| DispatchItemRow {
                    dispatch_id: "d1".into(),
                    idx,
                    title: format!("Item {idx}"),
                    prompt: "p".into(),
                    depends_on: Vec::new(),
                    status: "proposed".into(),
                    session_id: None,
                    outcome: None,
                    started_at: None,
                    finished_at: None,
                })
                .collect(),
        }
    }

    #[test]
    fn an_idle_coordinator_that_never_proposed_gives_the_split_back_after_the_grace() {
        let mut quiet = Quiet::new();
        let t0 = Instant::now();
        let d = row(dispatch_status::PLANNING, 3);
        assert!(!revision_unanswered(
            &d,
            Coordinator::Idle,
            false,
            &mut quiet,
            t0
        ));
        assert!(!revision_unanswered(
            &d,
            Coordinator::Idle,
            false,
            &mut quiet,
            t0 + QUIET_GRACE - Duration::from_millis(1)
        ));
        assert!(revision_unanswered(
            &d,
            Coordinator::Idle,
            false,
            &mut quiet,
            t0 + QUIET_GRACE
        ));
    }

    #[test]
    fn a_busy_or_message_holding_coordinator_resets_the_clock() {
        let mut quiet = Quiet::new();
        let t0 = Instant::now();
        let d = row(dispatch_status::PLANNING, 3);
        assert!(!revision_unanswered(
            &d,
            Coordinator::Idle,
            false,
            &mut quiet,
            t0
        ));
        assert!(!revision_unanswered(
            &d,
            Coordinator::Busy,
            false,
            &mut quiet,
            t0 + QUIET_GRACE
        ));
        assert!(
            !revision_unanswered(&d, Coordinator::Idle, true, &mut quiet, t0 + QUIET_GRACE),
            "a queued revise message has not reached the coordinator yet"
        );
        assert!(!revision_unanswered(
            &d,
            Coordinator::Idle,
            false,
            &mut quiet,
            t0 + QUIET_GRACE
        ));
    }

    #[test]
    fn a_missing_coordinator_also_gives_it_back() {
        let mut quiet = Quiet::new();
        let t0 = Instant::now();
        let d = row(dispatch_status::PLANNING, 2);
        assert!(!revision_unanswered(
            &d,
            Coordinator::Missing,
            true,
            &mut quiet,
            t0
        ));
        assert!(revision_unanswered(
            &d,
            Coordinator::Missing,
            true,
            &mut quiet,
            t0 + QUIET_GRACE
        ));
    }

    #[test]
    fn a_first_plan_or_a_settled_dispatch_is_never_reclaimed() {
        let mut quiet = Quiet::new();
        let t0 = Instant::now();
        let later = t0 + QUIET_GRACE * 2;
        for d in [
            row(dispatch_status::PLANNING, 0),
            row(dispatch_status::PROPOSED, 3),
            row(dispatch_status::RUNNING, 3),
        ] {
            assert!(!revision_unanswered(
                &d,
                Coordinator::Idle,
                false,
                &mut quiet,
                t0
            ));
            assert!(!revision_unanswered(
                &d,
                Coordinator::Idle,
                false,
                &mut quiet,
                later
            ));
        }
    }
}
