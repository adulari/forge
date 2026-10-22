//! UI state owned by the transcript surface: the open turn, its rail, repeated notices.

use crate::theme::styles::NoticeKind;
use std::time::Instant;

/// How long a settled turn's rail takes to cool through each step (brand → faint → line).
pub(crate) const COOL_STEP_MS: u128 = 650;

/// The open turn's rail, drawn hot while it works and cooled once it settles (full-screen only:
/// inline rows are baked into the terminal's scrollback).
#[derive(Debug, Clone)]
pub(crate) struct Cooling {
    /// First `main_log` line of the turn; rows from here on take the live rail colour.
    pub(crate) from: usize,
    /// When the turn settled; `None` while it is still working.
    pub(crate) since: Option<Instant>,
    /// The step last drawn, so a tick knows when a redraw is due.
    pub(crate) stage: u8,
}

/// The newest notice and how many times in a row it repeated (digits aside).
#[derive(Debug, Clone)]
pub(crate) struct NoticeRun {
    pub(crate) key: String,
    pub(crate) kind: NoticeKind,
    pub(crate) text: String,
    pub(crate) count: usize,
    pub(crate) in_turn: bool,
    /// The log position of its row, and the position just past it.
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Inline only: repeats were held back and the final `×N` row is still to print.
    pub(crate) held: bool,
}

/// State only the transcript renderer reads or writes.
#[derive(Debug, Clone, Default)]
pub(crate) struct TranscriptUi {
    /// The assistant label for this turn has been written.
    pub(crate) turn_open: bool,
    pub(crate) turn_started: Option<Instant>,
    /// Session cost when the turn began, so the footer shows this turn's cost alone.
    pub(crate) cost_base: f64,
    /// Text of each answer segment finished this turn (identical re-answers are dropped).
    pub(crate) segments: Vec<String>,
    pub(crate) tools_used: usize,
    pub(crate) reasoning_since: Option<Instant>,
    pub(crate) cooling: Option<Cooling>,
    pub(crate) notice_run: Option<NoticeRun>,
    /// Notices that arrived while an answer was streaming. They print once the segment ends, so
    /// a notice never lands in the middle of a line of the answer.
    pub(crate) deferred_notices: Vec<(NoticeKind, String)>,
    /// The CLI-bridge notice has been shown this session (it prints once).
    pub(crate) bridge_notice_seen: bool,
}
