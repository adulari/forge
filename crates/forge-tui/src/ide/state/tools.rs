//! The review tools around the diff: the light editor, why-trace, the review draft, the bottom
//! panel (checks, shells, terminal), the Files tab, the ship row and the coach line. All pure:
//! they read and write [`IdeApp`] and answer with [`IdeAction`]s for the host.

use std::collections::{HashMap, HashSet};

use ratatui::text::Span;

use super::*;
use crate::ide::checks::{append_output, FileRef, WireAction};
use crate::ide::edit_buf::EditBuf;
use crate::ide::review::{Drafts, ReviewLine};
use crate::ide::term_grid::TermGrid;
use crate::ide::wire::{BlameLine, WireEntry, WireJob};

/// Ticks (~100 ms) between background reads while a bottom tab is on screen.
pub(super) const CHECKS_EVERY: u64 = 30;
pub(super) const JOBS_EVERY: u64 = 20;
pub(super) const TAIL_EVERY: u64 = 10;
/// A second Esc within this many ticks leaves the terminal.
pub(super) const ESC_WINDOW: u64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Diff,
    Edit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFailure {
    /// 409: the file changed on disk since it was loaded.
    Conflict,
    ReadOnly,
    TooLarge,
    Other(String),
}

pub struct HlCache {
    pub version: u64,
    pub first: usize,
    pub rows: Vec<Vec<Span<'static>>>,
}

pub struct EditState {
    pub session_id: String,
    pub path: String,
    pub buf: Option<EditBuf>,
    pub hash: String,
    pub loading: bool,
    pub error: Option<String>,
    pub read_only: bool,
    pub scroll: usize,
    /// The queue's content hash when the buffer loaded: when it moves an agent edited the file.
    pub queue_hash: Option<String>,
    /// 1-based line to put the cursor on once the file arrives.
    pub jump: Option<usize>,
    pub hl: Option<HlCache>,
    pub saving: bool,
}

impl EditState {
    pub(super) fn new(session_id: String, path: String, queue_hash: Option<String>) -> Self {
        Self {
            session_id,
            path,
            buf: None,
            hash: String::new(),
            loading: true,
            error: None,
            read_only: false,
            scroll: 0,
            queue_hash,
            jump: None,
            hl: None,
            saving: false,
        }
    }

    pub fn dirty(&self) -> bool {
        self.buf.as_ref().is_some_and(EditBuf::is_dirty)
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlameData {
    pub hash: String,
    pub lines: HashMap<usize, BlameLine>,
}

#[derive(Debug, Clone)]
pub struct WhyPopup {
    pub entries: Vec<BlameLine>,
    pub index: usize,
}

#[derive(Debug, Clone)]
pub struct CommentTarget {
    pub session_id: String,
    pub path: String,
    pub header: String,
    pub staged: bool,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: Vec<ReviewLine>,
}

#[derive(Default)]
pub struct ChecksState {
    pub rows: Vec<WireAction>,
    pub loaded: bool,
    pub untrusted: bool,
    pub note: Option<String>,
    pub cursor: usize,
    pub outputs: HashMap<String, String>,
    pub refs: Vec<FileRef>,
    pub show_output: bool,
    pub ref_cursor: usize,
    pub failing: HashSet<String>,
    pub streamed: HashSet<String>,
    pub fetched_at: u64,
}

#[derive(Default)]
pub struct JobsState {
    pub rows: Vec<WireJob>,
    pub loaded: bool,
    pub note: Option<String>,
    pub cursor: usize,
    pub viewing: Option<u32>,
    pub tail: Vec<String>,
    pub tail_status: String,
    pub fetched_at: u64,
    pub tail_at: u64,
}

#[derive(Default)]
pub struct TermState {
    pub grid: Option<TermGrid>,
    pub opened: bool,
    pub ended: Option<Option<i64>>,
    pub focused: bool,
    pub view: (u16, u16),
    pub sent: (u16, u16),
    pub esc_tick: Option<u64>,
    pub session: Option<String>,
}

#[derive(Default)]
pub struct FilesState {
    pub dirs: HashMap<String, Vec<WireEntry>>,
    pub expanded: HashSet<String>,
    pub loading: HashSet<String>,
    pub cursor: usize,
    pub error: Option<String>,
    pub truncated: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub dir: bool,
    pub open: bool,
    pub changed: bool,
}

#[derive(Default)]
pub struct Tools {
    pub edit: Option<EditState>,
    pub why: bool,
    pub blame: HashMap<(String, String), BlameData>,
    pub blame_pending: HashSet<(String, String)>,
    pub why_popup: Option<WhyPopup>,
    pub pending_turn: Option<(String, i64)>,
    pub drafts: Drafts,
    pub comment: Input,
    pub comment_target: Option<CommentTarget>,
    pub checks: ChecksState,
    pub jobs: JobsState,
    pub term: TermState,
    pub files: FilesState,
    pub suggested_body: Option<(String, String)>,
    pub coach: Option<usize>,
    pub coach_loaded: bool,
}

/// What the coach line says at each step, and what the user does to move past it.
pub const COACH: [(&str, &str); 3] = [
    (
        "Changes queue: files that need you first. j opens the first one",
        "open a file",
    ),
    (
        "Review the diff: n/p hunks, a accept, x revert, c comment, e edit",
        "act on a hunk",
    ),
    (
        "Send it back: S sends your comments as one review, m messages the agent",
        "send",
    ),
];

impl IdeApp {
    // ------------------------------------------------------------------ events and ticks

    pub(crate) fn apply_tool_event(&mut self, event: IdeEvent) -> Vec<IdeAction> {
        let mut actions = Vec::new();
        match event {
            IdeEvent::File {
                session_id,
                path,
                result,
            } => self.apply_file(&session_id, &path, result),
            IdeEvent::Saved {
                session_id,
                path,
                result,
            } => actions.extend(self.apply_saved(&session_id, &path, result)),
            IdeEvent::Blame {
                session_id,
                path,
                result,
            } => self.apply_blame(session_id, path, result),
            IdeEvent::Checks(_, result) => actions.extend(self.apply_checks(result)),
            IdeEvent::CheckOutput {
                action,
                text,
                reset,
                ..
            } => {
                if reset {
                    self.tools.checks.outputs.remove(&action);
                }
                append_output(self.tools.checks.outputs.entry(action).or_default(), &text);
                self.refresh_refs();
                self.recompute_failing();
            }
            IdeEvent::Jobs(_, Ok(rows)) => {
                self.tools.jobs.rows = rows;
                self.tools.jobs.loaded = true;
                self.tools.jobs.note = None;
                self.tools.jobs.cursor = self
                    .tools
                    .jobs
                    .cursor
                    .min(self.tools.jobs.rows.len().saturating_sub(1));
            }
            IdeEvent::Jobs(_, Err(message)) => {
                self.tools.jobs.loaded = true;
                self.tools.jobs.note = Some(message);
            }
            IdeEvent::JobTail { job, result, .. } => {
                if self.tools.jobs.viewing == Some(job) {
                    match result {
                        Ok(tail) => {
                            self.tools.jobs.tail = tail.text.lines().map(str::to_string).collect();
                            self.tools.jobs.tail_status = tail.status;
                        }
                        Err(message) => self.tools.jobs.note = Some(message),
                    }
                }
            }
            IdeEvent::Entries { dir, result, .. } => self.apply_entries(dir, result),
            IdeEvent::Suggested(_, result) => self.apply_suggested(result),
            IdeEvent::TermBytes(bytes) => {
                if let Some(grid) = self.tools.term.grid.as_mut() {
                    grid.feed(&bytes);
                }
            }
            IdeEvent::TermEnded(code) => {
                self.tools.term.ended = Some(code);
            }
            IdeEvent::Handoff(Ok((cwd, command))) => {
                let id = self.panel_session().unwrap_or_default();
                actions.extend(self.open_terminal_with(&id));
                let quoted = format!("'{}'", cwd.replace('\'', "'\\''"));
                actions.push(IdeAction::TermInput(
                    format!("cd {quoted} && {command}\n").into_bytes(),
                ));
            }
            IdeEvent::Handoff(Err(message)) => self.say(ToastLevel::Error, message),
            IdeEvent::Prefs { coach_seen } => {
                self.tools.coach_loaded = true;
                self.tools.coach = (!coach_seen).then_some(0);
            }
            _ => {}
        }
        actions
    }

    pub(crate) fn tools_after_changes(&mut self) -> Vec<IdeAction> {
        let mut actions = self.edit_after_changes();
        actions.extend(self.ensure_blame());
        actions
    }

    pub(crate) fn tools_tick(&mut self) -> Vec<IdeAction> {
        let mut actions = self.sync_term();
        if let Some(open) = self.open.as_ref() {
            if open.view == View::Diff {
                actions.extend(self.ensure_blame());
            }
        }
        if !self.bottom_open {
            return actions;
        }
        let Some(id) = self.panel_session() else {
            return actions;
        };
        let session = self.route_session(&id);
        match self.bottom_tab {
            BottomTab::Checks => {
                let busy = self.tools.checks.rows.iter().any(|r| r.running);
                let every = if busy { CHECKS_EVERY / 3 } else { CHECKS_EVERY };
                if self.tick.saturating_sub(self.tools.checks.fetched_at) >= every {
                    self.tools.checks.fetched_at = self.tick;
                    actions.push(IdeAction::FetchChecks(session));
                }
            }
            BottomTab::Shells => {
                if self.tick.saturating_sub(self.tools.jobs.fetched_at) >= JOBS_EVERY {
                    self.tools.jobs.fetched_at = self.tick;
                    actions.push(IdeAction::FetchJobs(session.clone()));
                }
                if let Some(job) = self.tools.jobs.viewing {
                    if self.tick.saturating_sub(self.tools.jobs.tail_at) >= TAIL_EVERY {
                        self.tools.jobs.tail_at = self.tick;
                        actions.push(IdeAction::FetchJobTail {
                            session_id: session,
                            job,
                        });
                    }
                }
            }
            BottomTab::Terminal => {}
        }
        actions
    }

    /// Something moves on screen without input (a running check, a followed log, a terminal).
    pub(crate) fn tools_animating(&self) -> bool {
        self.bottom_open
            && match self.bottom_tab {
                BottomTab::Checks => self.tools.checks.rows.iter().any(|r| r.running),
                BottomTab::Shells => {
                    self.tools.jobs.rows.iter().any(|j| j.running)
                        || self.tools.jobs.viewing.is_some()
                }
                BottomTab::Terminal => false,
            }
    }
}
