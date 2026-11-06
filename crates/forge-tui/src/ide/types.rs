//! IDE mode's vocabulary: panes, hits, the events the host feeds in, the actions it must perform,
//! and the cached diff.

use super::state::{SaveFailure, View};
use std::rc::Rc;

use crate::board::{ConnState, FleetRow, HistoryRow, LiveSnapshot, ToastLevel};
use crate::diff_overlay::{from_snapshot, layout, OverlayFile, Row};
use crate::{DiffFileSnapshot, DiffHunkSnapshot};

use super::hunk::{Bucket, HunkAction, UndoRecord};
use super::model::ReviewedMarks;
use super::wire::{GitStatus, ProjectChanges, WireDiffFile};

pub const MARKS_CAP: usize = 2000;
/// Undo steps kept for reverted hunks and files.
pub(super) const UNDO_CAP: usize = 20;
/// Ticks (~100 ms) an info toast stays; errors stay twice as long.
pub(super) const TOAST_TICKS: u64 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    Rail,
    Editor,
    Agent,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailTab {
    Changes,
    Files,
    Git,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailFocus {
    Agents,
    List,
}

/// The bottom panel's tabs. Only the frame exists in this build; the panels fill in later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BottomTab {
    Checks,
    Shells,
    Terminal,
}

/// One agent as the rail lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRow {
    pub id: String,
    pub title: String,
    pub waiting: bool,
    pub busy: bool,
    pub adds: usize,
    pub dels: usize,
    pub model: String,
    pub cost_usd: f64,
    pub last_activity: i64,
    pub branch: String,
    pub worktree: bool,
    pub read_only: bool,
}

/// A clickable region recorded during render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    Agent(String),
    AllAgents,
    RailTab(RailTab),
    Queue(String),
    GitRow(usize),
    Pane(Pane),
    /// A narrow-layout tab that also picks what the rail shows.
    NarrowRail(RailFocus, RailTab),
    Hunk(usize),
    Timeline(usize),
    Needs(NeedsHit),
    BottomToggle,
    BottomTab(BottomTab),
    CommitInput,
    Commit,
    Push,
    Ship,
    Suggest,
    Composer,
    Overlay,
    File(usize),
    Check(usize),
    Job(usize),
    Draft,
    /// A line of the text editor: its index, the screen column its text starts at, and how many
    /// display cells the view is scrolled sideways.
    EditLine(usize, u16, usize),
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeedsHit {
    Allow,
    Deny,
    Option(usize),
}

/// Why a hunk request failed, so the view knows what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkFailure {
    /// 409 `stale_patch`: the file changed since the diff loaded; refetch rather than retry.
    Stale(String),
    /// 409 `session_busy`: the agent is mid-turn and owns the tree.
    Busy(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffLoaded {
    pub session_id: String,
    pub path: String,
    pub bucket: Bucket,
    pub file: Result<Option<WireDiffFile>, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HunkOutcome {
    pub session_id: String,
    pub path: String,
    pub action: HunkAction,
    pub result: Result<(), HunkFailure>,
    /// The record that puts a reverted hunk or file back, kept only on success.
    pub undo: Option<UndoRecord>,
    pub is_undo: bool,
}

/// A file whose routine hunks "accept routine" should stage; `session_id` is the routed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineTarget {
    pub session_id: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoutineOutcome {
    /// Hunks that went into the index.
    pub staged: usize,
    /// Hunks (or files) the daemon refused, usually because the file changed meanwhile.
    pub failed: usize,
    /// Takes the whole accept back with `u`.
    pub undo: Option<UndoRecord>,
}

/// Something the host learned and the IDE should reflect.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum IdeEvent {
    Fleet(Vec<FleetRow>),
    /// `Err(true)` is "this daemon has no such route" (404/405); `Err(false)` a real failure.
    Changes(Result<ProjectChanges, (bool, String)>),
    GitStatus(String, Result<GitStatus, String>),
    Diff(DiffLoaded),
    Snapshot(String, LiveSnapshot),
    SessionClosed(String),
    History(String, Vec<HistoryRow>),
    Hunk(HunkOutcome),
    Routine(RoutineOutcome),
    /// A stage, commit or push finished.
    GitOp(Result<String, String>),
    Toast(ToastLevel, String),
    Connection(ConnState),
    /// The persisted reviewed marks, read at start.
    Reviewed(ReviewedMarks),
    Tick,
    Resize(u16, u16),
    File {
        session_id: String,
        path: String,
        result: Result<super::wire::WireFile, String>,
    },
    Saved {
        session_id: String,
        path: String,
        result: Result<String, SaveFailure>,
    },
    Blame {
        session_id: String,
        path: String,
        result: Result<Vec<super::wire::BlameLine>, String>,
    },
    Checks(String, Result<super::checks::WireActions, String>),
    /// Output of a check's terminal; `reset` starts the buffer over (a new run or a re-attach).
    CheckOutput {
        session_id: String,
        action: String,
        text: String,
        reset: bool,
    },
    Jobs(String, Result<Vec<super::wire::WireJob>, String>),
    JobTail {
        session_id: String,
        job: u32,
        result: Result<super::wire::WireTail, String>,
    },
    Entries {
        session_id: String,
        dir: String,
        result: Result<super::wire::WireEntries, String>,
    },
    Suggested(String, Result<String, String>),
    TermBytes(Vec<u8>),
    TermEnded(Option<i64>),
    /// The hand-off command to run in the terminal tab: `(cwd, command)`.
    Handoff(Result<(String, String), String>),
    /// The persisted TUI state, read at start.
    Prefs {
        coach_seen: bool,
    },
}

/// What the host must do because of user input.
#[derive(Debug, Clone, PartialEq)]
pub enum IdeAction {
    /// Leave IDE mode, back to the caller (chat, board or the shell).
    Close,
    RefreshChanges,
    FetchDiff {
        session_id: String,
        path: String,
        worktree: bool,
    },
    FetchGitStatus(String),
    FetchHistory(String),
    Hunk {
        session_id: String,
        path: String,
        header: String,
        patch: String,
        action: HunkAction,
        undo: Option<UndoRecord>,
        is_undo: bool,
    },
    DiscardFile {
        session_id: String,
        path: String,
        undo: Option<UndoRecord>,
    },
    /// Stage every routine hunk of these files, one hunk request each.
    AcceptRoutine(Vec<RoutineTarget>),
    /// Take an accept back: unstage these hunks one request at a time (two `git apply --cached`
    /// at once would fight over the index lock).
    UnstageHunks(Vec<UndoRecord>),
    StagePaths {
        session_id: String,
        paths: Vec<String>,
        stage: bool,
    },
    Commit {
        session_id: String,
        message: String,
    },
    Push {
        session_id: String,
    },
    /// `RemoteInput` JSON for this session's socket (allow / answer / prompt).
    Input(String, serde_json::Value),
    /// Keep exactly this session's `/ws` open (and no other).
    Watch(Option<String>),
    /// Write the reviewed marks (this JSON) to `tui-ide-reviewed.json`.
    SaveReviewed(String),
    /// Write the TUI state (this JSON) to `tui-ide-state.json`.
    SavePrefs(String),
    FetchFile {
        session_id: String,
        path: String,
    },
    /// `expected_hash: None` overwrites: the host reads the file's current hash first.
    SaveFile {
        session_id: String,
        path: String,
        content: String,
        expected_hash: Option<String>,
    },
    /// Hand the file to `$EDITOR` and save the result back.
    ExternalEdit {
        session_id: String,
        path: String,
    },
    FetchBlame {
        session_id: String,
        path: String,
    },
    FetchChecks(String),
    RunCheck {
        session_id: String,
        action: String,
    },
    StopCheck {
        session_id: String,
        action: String,
    },
    StreamCheck {
        session_id: String,
        action: String,
        terminal_id: String,
    },
    FetchJobs(String),
    FetchJobTail {
        session_id: String,
        job: u32,
    },
    FetchEntries {
        session_id: String,
        dir: String,
    },
    SuggestMessage(String),
    Ship {
        session_id: String,
        stage: Vec<String>,
        message: Option<String>,
        push: bool,
    },
    TermOpen {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    TermInput(Vec<u8>),
    TermResize(u16, u16),
    TermClose,
    /// `POST /api/sessions/{id}/claude`, then run the returned command in the terminal tab.
    ContinueClaude(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmKind {
    RevertHunk {
        header: String,
    },
    RevertFile,
    /// Esc in the editor with unsaved edits.
    DiscardEdits,
    /// F2 with unsaved edits.
    QuitDiscard,
    /// A save found the file changed on disk: `r` reloads, `o` overwrites.
    SaveConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub kind: ConfirmKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub level: ToastLevel,
    pub text: String,
    pub until: u64,
}

/// A diff parsed once and drawn many times: the daemon's hunks (for patches), the overlay form
/// (for rendering) and the row layouts. Cached per file and content hash.
pub struct ParsedDiff {
    pub wire: WireDiffFile,
    pub overlay: OverlayFile,
    pub gutter: usize,
    pub(crate) rows: Vec<Row>,
    pub starts: Vec<usize>,
    split: std::cell::OnceCell<(Vec<Row>, Vec<usize>)>,
}

impl ParsedDiff {
    pub fn new(wire: WireDiffFile) -> Self {
        let snap = DiffFileSnapshot {
            path: wire.path.clone(),
            kind: wire.kind.clone(),
            binary: wire.binary,
            adds: wire.adds,
            dels: wire.dels,
            hunks: wire
                .hunks
                .iter()
                .map(|h| DiffHunkSnapshot {
                    header: h.header.clone(),
                    lines: h.lines.clone(),
                })
                .collect(),
            skipped_lines: wire.skipped_lines,
        };
        let mut overlay = from_snapshot(&snap);
        if overlay.note.is_none() && wire.orig_path.is_some() {
            overlay.note = wire
                .orig_path
                .as_ref()
                .map(|from| format!("renamed from {from}"));
        }
        let max_no = overlay
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter_map(|l| l.old_no.max(l.new_no))
            .max()
            .unwrap_or(1);
        let (rows, starts) = layout(&overlay, false);
        Self {
            wire,
            overlay,
            gutter: max_no.to_string().len().max(3),
            rows,
            starts,
            split: std::cell::OnceCell::new(),
        }
    }

    /// Rows and hunk starts for the unified or the side-by-side layout.
    pub(crate) fn layout(&self, split: bool) -> (&[Row], &[usize]) {
        if !split {
            return (&self.rows, &self.starts);
        }
        let (rows, starts) = self.split.get_or_init(|| layout(&self.overlay, true));
        (rows, starts)
    }
}

pub(super) struct CachedDiff {
    pub(super) hash: String,
    pub(super) bucket: Bucket,
    pub(super) parsed: Rc<ParsedDiff>,
}

/// The file in the editor.
pub struct OpenFile {
    pub session_id: String,
    pub path: String,
    pub loading: bool,
    pub error: Option<String>,
    pub hunk: usize,
    pub scroll: usize,
    pub view: View,
    /// A line the edit wrote: once the diff loads, the hunk holding it becomes the cursor.
    pub seek: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    pub active: bool,
    pub text: String,
}

impl Input {
    pub(super) fn push(&mut self, ch: char) {
        if !ch.is_control() {
            self.text.push(ch);
        }
    }
}
