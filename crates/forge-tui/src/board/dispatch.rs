//! Plan & dispatch, as the board understands it: which cards coordinate or work for a dispatch,
//! the column and signals that follow from the dispatch's state (a proposed split is the user's
//! decision even though its coordinator is idle), the stable colour that ties a dispatch's cards
//! together, and the dependency-aware selection the approval checklist uses.
//!
//! Pure and terminal-free, like `model.rs`. The status strings mirror `forge_store`'s constants;
//! `forge-tui` does not depend on the store, and the wire is the contract anyway.

use std::collections::BTreeSet;

use ratatui::style::Color;

use crate::surface::{ACCENT, OKGREEN, ORANGE, TOOLCYAN};

use super::model::{Card, Column, Health, Signal, SignalLevel};
use super::wire::{DispatchInfo, DispatchItemInfo, FleetRow};

/// Dispatch statuses (`forge_store::dispatch_status`).
pub mod dispatch_status {
    pub const PLANNING: &str = "planning";
    pub const PROPOSED: &str = "proposed";
    pub const RUNNING: &str = "running";
    pub const DONE: &str = "done";
    pub const CANCELLED: &str = "cancelled";
}

/// Item statuses (`forge_store::item_status`).
pub mod item_status {
    pub const PROPOSED: &str = "proposed";
    pub const SKIPPED: &str = "skipped";
    pub const QUEUED: &str = "queued";
    pub const RUNNING: &str = "running";
    pub const SUCCEEDED: &str = "succeeded";
    pub const FAILED: &str = "failed";
    pub const STOPPED: &str = "stopped";
    pub const CANCELLED: &str = "cancelled";
    pub const MERGED: &str = "merged";
    pub const DISCARDED: &str = "discarded";

    pub fn is_terminal(status: &str) -> bool {
        matches!(
            status,
            SUCCEEDED | FAILED | STOPPED | CANCELLED | SKIPPED | MERGED | DISCARDED
        )
    }

    /// The item's work landed (or is ready to land) — what a dependant waits for.
    pub fn is_success(status: &str) -> bool {
        matches!(status, SUCCEEDED | MERGED)
    }
}

use dispatch_status as ds;
use item_status as is;

/// A card's part in a dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Coordinator,
    /// The 1-based item number.
    Worker(usize),
}

/// What a card knows about the dispatch it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardDispatch {
    pub id: String,
    pub role: Role,
    pub status: String,
    /// Items in the proposal, selected or not — the denominator of a worker's `◆ 2/5` chip.
    pub total: usize,
}

/// Counts over the items that were (or will be) run — skipped items are not part of the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub total: usize,
    pub finished: usize,
    pub running: usize,
    pub waiting: usize,
    pub succeeded: usize,
    pub failed: usize,
}

pub fn progress(d: &DispatchInfo) -> Progress {
    let mut p = Progress::default();
    for item in d.items.iter().filter(|i| i.status != is::SKIPPED) {
        p.total += 1;
        if is::is_terminal(&item.status) {
            p.finished += 1;
        }
        match item.status.as_str() {
            is::RUNNING => p.running += 1,
            is::QUEUED | is::PROPOSED => p.waiting += 1,
            is::SUCCEEDED | is::MERGED => p.succeeded += 1,
            is::FAILED | is::STOPPED => p.failed += 1,
            _ => {}
        }
    }
    p
}

/// Two board-local hues extend the palette so six dispatches can sit side by side. Red and
/// yellow are never used: on this board they mean "needs you" and "something is wrong".
const VIOLET: Color = Color::Rgb(178, 132, 255);
const PINK: Color = Color::Rgb(240, 128, 200);
const GROUP_COLORS: [Color; 6] = [ORANGE, TOOLCYAN, ACCENT, OKGREEN, VIOLET, PINK];

/// A dispatch's colour, stable across refreshes, restarts and machines (FNV-1a over the id —
/// `DefaultHasher` makes no stability promise).
pub fn group_color(id: &str) -> Color {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    GROUP_COLORS[(hash % GROUP_COLORS.len() as u64) as usize]
}

/// Which dispatch a session belongs to, and how. The dispatch list is the authority for the
/// coordinator; for a worker the fleet row says so, with the items' `session_id` as a fallback
/// for a daemon whose rows predate the dispatch fields.
pub fn membership<'a>(
    id: &str,
    row: Option<&FleetRow>,
    dispatches: &'a [DispatchInfo],
) -> Option<(&'a DispatchInfo, Role)> {
    if let Some(d) = dispatches.iter().find(|d| d.coordinator_session_id == id) {
        return Some((d, Role::Coordinator));
    }
    if let Some(row) = row {
        if let (Some(did), Some("worker"), Some(n)) = (
            row.dispatch_id.as_deref(),
            row.dispatch_role.as_deref(),
            row.dispatch_index,
        ) {
            if let Some(d) = dispatches.iter().find(|d| d.id == did) {
                return Some((d, Role::Worker(n)));
            }
        }
    }
    dispatches.iter().find_map(|d| {
        d.items
            .iter()
            .find(|i| i.session_id.as_deref() == Some(id))
            .map(|i| (d, Role::Worker(i.index)))
    })
}

/// Stamp every card with its dispatch, and let a coordinator's column and signals follow the
/// dispatch rather than its own idle/busy state. A coordinator that is blocked on a permission or
/// stalled keeps the column that says so — that is more urgent than any dispatch state.
pub fn annotate(cards: &mut [Card], rows: &[FleetRow], dispatches: &[DispatchInfo]) {
    for card in cards.iter_mut() {
        let row = rows.iter().find(|r| r.id == card.id);
        let Some((d, role)) = membership(&card.id, row, dispatches) else {
            continue;
        };
        card.dispatch = Some(CardDispatch {
            id: d.id.clone(),
            role,
            status: d.status.clone(),
            total: d.items.len(),
        });
        if role != Role::Coordinator || card.past {
            continue;
        }
        let p = progress(d);
        let urgent = matches!(card.health, Health::Waiting | Health::Stalled);
        let column = match d.status.as_str() {
            ds::PROPOSED => {
                let n = d.items.len();
                card.signals.push(Signal {
                    level: SignalLevel::Danger,
                    text: format!("split ready · {n} sessions to review"),
                });
                Column::Attention
            }
            ds::PLANNING | ds::RUNNING => Column::Working,
            ds::DONE => {
                if p.failed > 0 {
                    card.signals.push(Signal {
                        level: SignalLevel::Warn,
                        text: format!("{} of {} did not finish", p.failed, p.total),
                    });
                }
                Column::Ready
            }
            _ => Column::Ready,
        };
        if !urgent {
            card.column = column;
        }
        card.signals.sort_by_key(|s| std::cmp::Reverse(s.level));
    }
}

/// The items `item` still waits on: dependencies that have not succeeded.
pub fn waits_for(d: &DispatchInfo, item: &DispatchItemInfo) -> Vec<usize> {
    item.depends_on
        .iter()
        .copied()
        .filter(|dep| {
            d.items
                .iter()
                .find(|i| i.index == *dep)
                .is_none_or(|i| !is::is_success(&i.status))
        })
        .collect()
}

/// Toggle item `index` in `selected`, keeping the selection runnable: deselecting an item also
/// deselects everything that (transitively) depends on it, and selecting one also selects what it
/// depends on. Returns whether `index` is now selected and the OTHER items that changed with it.
pub fn toggle(
    items: &[DispatchItemInfo],
    selected: &mut BTreeSet<usize>,
    index: usize,
) -> (bool, Vec<usize>) {
    let mut changed = BTreeSet::new();
    if selected.remove(&index) {
        let mut removed = vec![index];
        while let Some(gone) = removed.pop() {
            for item in items {
                if item.depends_on.contains(&gone) && selected.remove(&item.index) {
                    changed.insert(item.index);
                    removed.push(item.index);
                }
            }
        }
        (false, changed.into_iter().collect())
    } else {
        selected.insert(index);
        let mut stack = vec![index];
        while let Some(n) = stack.pop() {
            let deps = items
                .iter()
                .find(|i| i.index == n)
                .map(|i| i.depends_on.clone())
                .unwrap_or_default();
            for dep in deps {
                if items.iter().any(|i| i.index == dep) && selected.insert(dep) {
                    changed.insert(dep);
                    stack.push(dep);
                }
            }
        }
        (true, changed.into_iter().collect())
    }
}

/// `1, 2 and 4` — how a toast names a handful of items.
pub fn list_words(ns: &[usize]) -> String {
    let parts: Vec<String> = ns.iter().map(usize::to_string).collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The first non-empty line of a prompt — how a checklist row previews it.
pub fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// What a dispatch is called: its coordinator's title without the `Dispatch: ` prefix the daemon
/// adds, else the request's first line.
pub fn title(d: &DispatchInfo) -> String {
    let t = d.coordinator_title.trim();
    let t = t.strip_prefix("Dispatch:").map_or(t, str::trim);
    if t.is_empty() {
        first_line(&d.prompt)
    } else {
        t.to_string()
    }
}

/// A signature of the proposal's content, so a revised split resets the checklist while a mere
/// refresh of the same proposal keeps the user's ticks.
pub fn proposal_signature(d: &DispatchInfo) -> String {
    d.items
        .iter()
        .map(|i| format!("{}:{}:{:?}", i.index, i.title, i.depends_on))
        .collect::<Vec<_>>()
        .join("|")
}

pub fn is_proposed(d: &DispatchInfo) -> bool {
    d.status == ds::PROPOSED
}

pub fn is_running(d: &DispatchInfo) -> bool {
    d.status == ds::RUNNING
}

pub fn is_active(d: &DispatchInfo) -> bool {
    matches!(d.status.as_str(), ds::PLANNING | ds::PROPOSED | ds::RUNNING)
}
