//! IDE mode's state machine. Pure: folds [`IdeEvent`]s in, answers every user intent with
//! [`IdeAction`]s for the host (`forge-cli`'s `ide_host`), and is drawn by `render`. It never
//! touches the network, the disk or the terminal.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ratatui::layout::Rect;

use crate::board::{project_name, ConnState, FleetRow, HistoryRow, LiveSnapshot, ToastLevel};
use crate::diff_overlay::{from_snapshot, layout, OverlayFile, Row};
use crate::{DiffFileSnapshot, DiffHunkSnapshot};

use super::checks::WireActions;
use super::hunk::{
    actions_for, build_patch, undo_step_for_hunk, undo_steps_for_file, Bucket, HunkAction,
    UndoRecord,
};
use super::model::{
    build_queue, flatten_queue, is_reviewed, mark_reviewed, marks_to_json, next_to_review,
    prune_reviewed, step_queue, unmark_reviewed, ChangedFile, QueueAgent, QueueGroup, QueueRow,
    ReviewedMarks, Tier,
};
use super::timeline::{build_timeline, resolve_changed_path, TimelineItem, TimelineKind};
use super::wire::{
    BlameLine, GitStatus, ProjectChanges, WireDiffFile, WireEntries, WireFile, WireJob,
    WireSessionChanges, WireTail, UNATTRIBUTED,
};

mod tools;
pub use tools::{
    BlameData, ChecksState, CommentTarget, EditState, FileRow, FilesState, HlCache, JobsState,
    SaveFailure, TermState, Tools, View, WhyPopup, COACH,
};

/// Reviewed marks kept in the persisted file.
pub const MARKS_CAP: usize = 2000;
/// Undo steps kept for reverted hunks and files.
const UNDO_CAP: usize = 20;
/// Ticks (~100 ms) an info toast stays; errors stay twice as long.
const TOAST_TICKS: u64 = 40;

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
    /// A line of the text editor: its index and the screen column its text starts at.
    EditLine(usize, u16),
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
        result: Result<WireFile, String>,
    },
    Saved {
        session_id: String,
        path: String,
        result: Result<String, SaveFailure>,
    },
    Blame {
        session_id: String,
        path: String,
        result: Result<Vec<BlameLine>, String>,
    },
    Checks(String, Result<WireActions, String>),
    /// Output of a check's terminal; `reset` starts the buffer over (a new run or a re-attach).
    CheckOutput {
        session_id: String,
        action: String,
        text: String,
        reset: bool,
    },
    Jobs(String, Result<Vec<WireJob>, String>),
    JobTail {
        session_id: String,
        job: u32,
        result: Result<WireTail, String>,
    },
    Entries {
        session_id: String,
        dir: String,
        result: Result<WireEntries, String>,
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

struct CachedDiff {
    hash: String,
    bucket: Bucket,
    parsed: Rc<ParsedDiff>,
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
    fn push(&mut self, ch: char) {
        if !ch.is_control() {
            self.text.push(ch);
        }
    }
}

pub struct IdeApp {
    pub(crate) cwd: String,
    pub(crate) project: String,
    pub(crate) now: i64,
    pub(crate) tick: u64,
    pub(crate) size: (u16, u16),
    pub(crate) connection: ConnState,
    fleet: Vec<FleetRow>,
    changes: Option<ProjectChanges>,
    pub(crate) changes_note: Option<String>,
    pub(crate) agents: Vec<AgentRow>,
    queue_agents: Vec<QueueAgent>,
    pub(crate) groups: Vec<QueueGroup>,
    pub(crate) rows: Vec<QueueRow>,
    pub(crate) marks: ReviewedMarks,
    /// Limit the queue to one agent (`None` is "All agents").
    pub(crate) scope: Option<String>,
    pub(crate) agent_cursor: usize,
    pub(crate) rail_tab: RailTab,
    pub(crate) rail_focus: RailFocus,
    pub(crate) git_cursor: usize,
    pub(crate) focus: Pane,
    pub(crate) bottom_open: bool,
    pub(crate) bottom_tab: BottomTab,
    pub(crate) split: bool,
    pub(crate) open: Option<OpenFile>,
    cache: HashMap<(String, String), CachedDiff>,
    inflight: HashSet<(String, String)>,
    pub(crate) git: HashMap<String, GitStatus>,
    pub(crate) snapshots: HashMap<String, LiveSnapshot>,
    history: HashMap<String, Vec<HistoryRow>>,
    pub(crate) timeline: Vec<TimelineItem>,
    timeline_for: Option<String>,
    pub(crate) timeline_cursor: usize,
    pub(crate) composer: Input,
    pub(crate) commit: Input,
    pub(crate) undo: Vec<UndoRecord>,
    pub(crate) confirm: Option<Confirm>,
    pub(crate) toast: Option<Toast>,
    pub(crate) help_open: bool,
    pub(crate) hits: Vec<(Rect, Hit)>,
    pub(crate) view_h: usize,
    pub(crate) tools: Tools,
    watching: Option<String>,
    marks_loaded: bool,
}

impl IdeApp {
    pub fn new(cwd: &str, now: i64) -> Self {
        Self {
            cwd: cwd.to_string(),
            project: project_name(cwd),
            now,
            tick: 0,
            size: (0, 0),
            connection: ConnState::Connecting,
            fleet: Vec::new(),
            changes: None,
            changes_note: None,
            agents: Vec::new(),
            queue_agents: Vec::new(),
            groups: Vec::new(),
            rows: Vec::new(),
            marks: ReviewedMarks::new(),
            scope: None,
            agent_cursor: 0,
            rail_tab: RailTab::Changes,
            rail_focus: RailFocus::List,
            git_cursor: 0,
            focus: Pane::Rail,
            bottom_open: false,
            bottom_tab: BottomTab::Checks,
            split: false,
            open: None,
            cache: HashMap::new(),
            inflight: HashSet::new(),
            git: HashMap::new(),
            snapshots: HashMap::new(),
            history: HashMap::new(),
            timeline: Vec::new(),
            timeline_for: None,
            timeline_cursor: 0,
            composer: Input::default(),
            commit: Input::default(),
            undo: Vec::new(),
            confirm: None,
            toast: None,
            help_open: false,
            hits: Vec::new(),
            view_h: 20,
            tools: Tools::default(),
            watching: None,
            marks_loaded: false,
        }
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn set_now(&mut self, now: i64) {
        self.now = now;
    }

    // ------------------------------------------------------------------ derived

    /// The session the agent panel shows: the scoped agent, else the open file's, else the one at
    /// the agents cursor (needs-you agents sort first, so that is who most needs a look).
    pub fn panel_session(&self) -> Option<String> {
        self.scope
            .clone()
            .or_else(|| {
                self.open
                    .as_ref()
                    .map(|o| o.session_id.clone())
                    .filter(|id| id != UNATTRIBUTED)
            })
            .or_else(|| self.agents.get(self.agent_cursor).map(|a| a.id.clone()))
    }

    pub fn panel_agent(&self) -> Option<&AgentRow> {
        let id = self.panel_session()?;
        self.agents.iter().find(|a| a.id == id)
    }

    pub fn snapshot(&self) -> Option<&LiveSnapshot> {
        self.snapshots.get(&self.panel_session()?)
    }

    /// A real session to ask git about: the unattributed group has none of its own.
    pub fn route_session(&self, session_id: &str) -> String {
        if session_id != UNATTRIBUTED {
            return session_id.to_string();
        }
        self.changes
            .as_ref()
            .and_then(|c| {
                c.sessions
                    .iter()
                    .find(|s| s.session_id != UNATTRIBUTED && s.worktree.is_none())
            })
            .map(|s| s.session_id.clone())
            .unwrap_or_else(|| session_id.to_string())
    }

    fn is_worktree(&self, session_id: &str) -> bool {
        self.changes.as_ref().is_some_and(|c| {
            c.sessions
                .iter()
                .any(|s| s.session_id == session_id && s.worktree.is_some())
        })
    }

    fn file_hash(&self, session_id: &str, path: &str) -> Option<&str> {
        self.queue_agents
            .iter()
            .find(|a| a.session_id == session_id)?
            .files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.hash.as_str())
    }

    pub fn to_review(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.tier != Tier::Reviewed)
            .count()
    }

    pub fn waiting_count(&self) -> usize {
        self.agents.iter().filter(|a| a.waiting).count()
    }

    pub fn parsed(&self) -> Option<(Rc<ParsedDiff>, Bucket)> {
        let open = self.open.as_ref()?;
        let cached = self
            .cache
            .get(&(open.session_id.clone(), open.path.clone()))?;
        Some((cached.parsed.clone(), cached.bucket))
    }

    pub fn open_row(&self) -> Option<&QueueRow> {
        let open = self.open.as_ref()?;
        self.rows
            .iter()
            .find(|r| r.session_id == open.session_id && r.path == open.path)
    }

    pub fn open_is_reviewed(&self) -> bool {
        self.open_row().is_some_and(|r| r.tier == Tier::Reviewed)
    }

    /// The git rows the Git tab lists, in display order: conflicted, staged, unstaged, untracked.
    pub fn git_list(&self) -> Vec<(Bucket, bool, super::wire::GitRow)> {
        let Some(status) = self.panel_session().and_then(|id| self.git.get(&id)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for row in &status.conflicted {
            out.push((Bucket::Unstaged, true, row.clone()));
        }
        for row in &status.staged {
            out.push((Bucket::Staged, false, row.clone()));
        }
        for row in &status.unstaged {
            out.push((Bucket::Unstaged, false, row.clone()));
        }
        for row in &status.untracked {
            out.push((Bucket::Untracked, false, row.clone()));
        }
        out
    }

    pub fn hit_at(&self, col: u16, row: u16) -> Option<&Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
            .map(|(_, h)| h)
    }

    /// Something on screen moves with time (a spinner, a toast about to expire).
    pub fn animating(&self) -> bool {
        self.toast.is_some() || self.agents.iter().any(|a| a.busy) || self.tools_animating()
    }

    pub fn is_typing(&self) -> bool {
        self.composer.active || self.commit.active || self.tools.comment.active
    }

    // ------------------------------------------------------------------ events

    pub fn apply(&mut self, event: IdeEvent) -> Vec<IdeAction> {
        let mut actions = Vec::new();
        match event {
            IdeEvent::Fleet(rows) => {
                self.fleet = rows;
                self.rebuild_agents();
            }
            IdeEvent::Changes(Ok(changes)) => {
                self.changes_note = None;
                self.changes = Some(changes);
                self.rebuild_agents();
                self.rebuild_queue();
                actions.extend(self.after_changes());
                actions.extend(self.tools_after_changes());
            }
            IdeEvent::Changes(Err((missing, message))) => {
                self.changes_note = Some(if missing {
                    "this daemon predates IDE mode: update and restart `forge serve`".to_string()
                } else {
                    message
                });
            }
            IdeEvent::GitStatus(id, Ok(status)) => {
                self.git.insert(id, status);
                let len = self.git_list().len();
                self.git_cursor = self.git_cursor.min(len.saturating_sub(1));
            }
            IdeEvent::GitStatus(_, Err(_)) => {}
            IdeEvent::Diff(loaded) => self.apply_diff(loaded),
            IdeEvent::Snapshot(id, snap) => {
                if snap.closed {
                    self.snapshots.remove(&id);
                } else {
                    self.snapshots.insert(id, snap);
                }
            }
            IdeEvent::SessionClosed(id) => {
                self.snapshots.remove(&id);
            }
            IdeEvent::History(id, rows) => {
                self.history.insert(id, rows);
                self.rebuild_timeline();
                self.apply_pending_turn();
            }
            IdeEvent::Hunk(outcome) => actions.extend(self.apply_hunk(outcome)),
            IdeEvent::GitOp(result) => {
                match result {
                    Ok(note) => self.say(ToastLevel::Ok, note),
                    Err(e) => self.say(ToastLevel::Error, e),
                }
                actions.push(IdeAction::RefreshChanges);
                if let Some(id) = self.panel_session() {
                    actions.push(IdeAction::FetchGitStatus(self.route_session(&id)));
                }
            }
            IdeEvent::Toast(level, text) => self.say(level, text),
            IdeEvent::Connection(state) => self.connection = state,
            IdeEvent::Reviewed(marks) => {
                // Marks made before the file was read win over the stored ones.
                let mut merged = marks;
                merged.extend(std::mem::take(&mut self.marks));
                self.marks = merged;
                self.marks_loaded = true;
                self.rebuild_queue();
            }
            IdeEvent::Tick => {
                self.tick += 1;
                if self.toast.as_ref().is_some_and(|t| self.tick >= t.until) {
                    self.toast = None;
                }
                actions.extend(self.tools_tick());
            }
            IdeEvent::Resize(w, h) => self.size = (w, h),
            other => actions.extend(self.apply_tool_event(other)),
        }
        actions.extend(self.sync_watch());
        actions
    }

    pub fn say(&mut self, level: ToastLevel, text: String) {
        let ticks = if level == ToastLevel::Error {
            TOAST_TICKS * 2
        } else {
            TOAST_TICKS
        };
        self.toast = Some(Toast {
            level,
            text,
            until: self.tick + ticks,
        });
    }

    fn rebuild_agents(&mut self) {
        let Some(changes) = &self.changes else {
            self.agents.clear();
            return;
        };
        let fleet: HashMap<&str, &FleetRow> =
            self.fleet.iter().map(|r| (r.id.as_str(), r)).collect();
        let mut agents: Vec<AgentRow> = changes
            .sessions
            .iter()
            .filter(|s| s.session_id != UNATTRIBUTED)
            .map(|s: &WireSessionChanges| {
                let row = fleet.get(s.session_id.as_str());
                AgentRow {
                    id: s.session_id.clone(),
                    title: if !s.title.trim().is_empty() {
                        s.title.trim().to_string()
                    } else if let Some(r) = row.filter(|r| !r.title.trim().is_empty()) {
                        r.title.trim().to_string()
                    } else {
                        format!("session {}", short(&s.session_id))
                    },
                    waiting: s.waiting || row.is_some_and(|r| r.waiting),
                    busy: s.busy || row.is_some_and(|r| r.busy),
                    adds: s.files.iter().map(|f| f.adds).sum(),
                    dels: s.files.iter().map(|f| f.dels).sum(),
                    model: row.map(|r| r.model.clone()).unwrap_or_default(),
                    cost_usd: row.map_or(0.0, |r| r.cost_usd),
                    last_activity: row.map_or(0, |r| r.last_activity),
                    branch: s.branch.clone(),
                    worktree: s.worktree.is_some(),
                    read_only: row.is_some_and(|r| r.read_only),
                }
            })
            .collect();
        agents.sort_by(|a, b| {
            b.waiting
                .cmp(&a.waiting)
                .then(b.busy.cmp(&a.busy))
                .then(b.last_activity.cmp(&a.last_activity))
                .then(a.title.cmp(&b.title))
        });
        self.agents = agents;
        self.agent_cursor = self.agent_cursor.min(self.agents.len().saturating_sub(1));
        if self
            .scope
            .as_ref()
            .is_some_and(|s| !self.agents.iter().any(|a| &a.id == s))
        {
            self.scope = None;
        }
    }

    fn rebuild_queue(&mut self) {
        let Some(changes) = &self.changes else {
            return;
        };
        let fleet: HashMap<&str, &FleetRow> =
            self.fleet.iter().map(|r| (r.id.as_str(), r)).collect();
        self.queue_agents = changes
            .sessions
            .iter()
            .map(|s| QueueAgent {
                session_id: s.session_id.clone(),
                name: if s.session_id == UNATTRIBUTED {
                    "Unattributed changes".to_string()
                } else {
                    self.agents
                        .iter()
                        .find(|a| a.id == s.session_id)
                        .map_or_else(|| short(&s.session_id).to_string(), |a| a.title.clone())
                },
                waiting: s.waiting,
                busy: s.busy,
                last_activity: fleet
                    .get(s.session_id.as_str())
                    .map_or(0, |r| r.last_activity),
                files: s
                    .files
                    .iter()
                    .map(|f| {
                        let status = match f.kind.as_str() {
                            "created" => "A",
                            "deleted" => "D",
                            "renamed" => "R",
                            _ => "M",
                        };
                        let mut file =
                            ChangedFile::new(&f.path, status, f.adds, f.dels, &f.content_hash);
                        file.failing = self.tools.checks.failing.contains(&f.path);
                        file
                    })
                    .collect(),
            })
            .collect();
        self.groups = build_queue(&self.queue_agents, &self.marks, self.scope.as_deref());
        self.rows = flatten_queue(&self.groups);
    }

    /// After a new changes read: drop stale marks, and refetch the open file if an agent edited
    /// it again (its content hash moved).
    fn after_changes(&mut self) -> Vec<IdeAction> {
        let mut actions = Vec::new();
        if self.marks_loaded {
            let live: std::collections::BTreeMap<
                String,
                std::collections::BTreeMap<String, String>,
            > = self
                .queue_agents
                .iter()
                .map(|a| {
                    (
                        a.session_id.clone(),
                        a.files
                            .iter()
                            .map(|f| (f.path.clone(), f.hash.clone()))
                            .collect(),
                    )
                })
                .collect();
            if prune_reviewed(&mut self.marks, &live) {
                actions.push(self.save_marks());
                self.rebuild_queue();
            }
        }
        let live_files: HashSet<(String, String)> = self
            .queue_agents
            .iter()
            .flat_map(|a| {
                a.files
                    .iter()
                    .map(|f| (a.session_id.clone(), f.path.clone()))
            })
            .collect();
        self.cache.retain(|key, _| live_files.contains(key));
        if let Some(open) = &self.open {
            let key = (open.session_id.clone(), open.path.clone());
            if !live_files.contains(&key) {
                self.open = None;
            } else {
                actions.extend(self.ensure_diff());
            }
        }
        actions
    }

    fn save_marks(&self) -> IdeAction {
        IdeAction::SaveReviewed(marks_to_json(&self.marks, MARKS_CAP))
    }

    /// The fetch for the open file when its cached diff is missing or out of date.
    fn ensure_diff(&mut self) -> Vec<IdeAction> {
        let Some(open) = &self.open else {
            return Vec::new();
        };
        let key = (open.session_id.clone(), open.path.clone());
        let fresh = self
            .cache
            .get(&key)
            .is_some_and(|c| Some(c.hash.as_str()) == self.file_hash(&key.0, &key.1));
        if fresh {
            if let Some(open) = self.open.as_mut() {
                open.loading = false;
            }
            return Vec::new();
        }
        if let Some(open) = self.open.as_mut() {
            open.loading = true;
        }
        if !self.inflight.insert(key.clone()) {
            return Vec::new();
        }
        vec![IdeAction::FetchDiff {
            session_id: self.route_session(&key.0),
            worktree: self.is_worktree(&key.0),
            path: key.1,
        }]
    }

    fn apply_diff(&mut self, loaded: DiffLoaded) {
        // The request carried the routed session; the cache is keyed by the queue's session.
        let key = self
            .open
            .as_ref()
            .map(|o| (o.session_id.clone(), o.path.clone()))
            .filter(|(sid, path)| {
                path == &loaded.path && self.route_session(sid) == loaded.session_id
            });
        let Some(key) = key else {
            let gone: Vec<(String, String)> = self
                .inflight
                .iter()
                .filter(|(sid, path)| {
                    path == &loaded.path && self.route_session(sid) == loaded.session_id
                })
                .cloned()
                .collect();
            for key in gone {
                self.inflight.remove(&key);
            }
            return;
        };
        self.inflight.remove(&key);
        match loaded.file {
            Ok(Some(file)) => {
                let hash = self.file_hash(&key.0, &key.1).unwrap_or("").to_string();
                let hunks = file.hunks.len();
                self.cache.insert(
                    key,
                    CachedDiff {
                        hash,
                        bucket: loaded.bucket,
                        parsed: Rc::new(ParsedDiff::new(file)),
                    },
                );
                if let Some(open) = self.open.as_mut() {
                    open.loading = false;
                    open.error = None;
                    open.hunk = open.hunk.min(hunks.saturating_sub(1));
                }
                self.seek_hunk();
            }
            Ok(None) => {
                if let Some(open) = self.open.as_mut() {
                    open.loading = false;
                    open.error = Some("the daemon has no diff for this file".to_string());
                }
            }
            Err(message) => {
                if let Some(open) = self.open.as_mut() {
                    open.loading = false;
                    open.error = Some(message);
                }
            }
        }
    }

    fn apply_hunk(&mut self, outcome: HunkOutcome) -> Vec<IdeAction> {
        let mut actions = vec![IdeAction::RefreshChanges];
        match outcome.result {
            Ok(()) => {
                if outcome.is_undo {
                    self.say(ToastLevel::Ok, "revert undone".to_string());
                } else {
                    self.say(ToastLevel::Ok, outcome.action.done().to_string());
                    if let Some(record) = outcome.undo {
                        self.undo.push(record);
                        if self.undo.len() > UNDO_CAP {
                            self.undo.remove(0);
                        }
                    }
                }
            }
            Err(HunkFailure::Stale(message)) | Err(HunkFailure::Busy(message)) => {
                self.say(ToastLevel::Info, message);
            }
            Err(HunkFailure::Error(message)) => self.say(ToastLevel::Error, message),
        }
        // Whatever happened the diff on screen may be out of date: drop it and read it again.
        let key = self
            .open
            .as_ref()
            .map(|o| (o.session_id.clone(), o.path.clone()))
            .filter(|(sid, path)| {
                path == &outcome.path && self.route_session(sid) == outcome.session_id
            });
        if let Some(key) = key {
            self.cache.remove(&key);
            actions.push(IdeAction::FetchGitStatus(outcome.session_id));
            // The changes read that follows renews the hash; fetch now so the view is not blank.
            self.inflight.remove(&key);
            if let Some(open) = self.open.as_mut() {
                open.loading = true;
            }
            self.inflight.insert(key.clone());
            actions.push(IdeAction::FetchDiff {
                session_id: self.route_session(&key.0),
                worktree: self.is_worktree(&key.0),
                path: key.1,
            });
        }
        actions
    }

    fn rebuild_timeline(&mut self) {
        let Some(id) = self.panel_session() else {
            self.timeline.clear();
            self.timeline_for = None;
            return;
        };
        let rows = self.history.get(&id).map(Vec::as_slice).unwrap_or(&[]);
        self.timeline = build_timeline(rows, 60);
        if self.timeline_for.as_deref() != Some(id.as_str()) {
            self.timeline_cursor = 0;
            self.timeline_for = Some(id);
        }
        self.timeline_cursor = self
            .timeline_cursor
            .min(self.timeline.len().saturating_sub(1));
    }

    /// Keep exactly the panel's agent watched, and read its history and git status when it
    /// changes.
    pub fn sync_watch(&mut self) -> Vec<IdeAction> {
        let want = self.panel_session();
        if want == self.watching {
            return Vec::new();
        }
        self.watching = want.clone();
        self.rebuild_timeline();
        let mut actions = vec![IdeAction::Watch(want.clone())];
        if let Some(id) = want {
            actions.push(IdeAction::FetchHistory(id.clone()));
            actions.push(IdeAction::FetchGitStatus(self.route_session(&id)));
        }
        actions
    }

    // ------------------------------------------------------------------ navigation

    fn set_scope(&mut self, scope: Option<String>) -> Vec<IdeAction> {
        self.scope = scope;
        self.rebuild_queue();
        let mut actions = Vec::new();
        if let Some(open) = &self.open {
            if self.scope.as_ref().is_some_and(|s| s != &open.session_id) && !self.edit_dirty() {
                self.open = None;
                self.tools.edit = None;
            }
        }
        actions.extend(self.sync_watch());
        actions
    }

    /// Enter on an agent: scope the queue to it, or clear the scope if it already is.
    pub fn toggle_scope(&mut self, id: &str) -> Vec<IdeAction> {
        if let Some(index) = self.agents.iter().position(|a| a.id == id) {
            self.agent_cursor = index;
        }
        let next = if self.scope.as_deref() == Some(id) {
            None
        } else {
            Some(id.to_string())
        };
        self.set_scope(next)
    }

    pub fn clear_scope(&mut self) -> Vec<IdeAction> {
        self.set_scope(None)
    }

    pub fn move_agent(&mut self, step: i32) -> Vec<IdeAction> {
        if self.agents.is_empty() {
            return Vec::new();
        }
        let max = self.agents.len() as i32 - 1;
        self.agent_cursor = (self.agent_cursor as i32 + step).clamp(0, max) as usize;
        self.sync_watch()
    }

    /// Open `key`'s file in the editor.
    pub fn open_key(&mut self, key: &str) -> Vec<IdeAction> {
        let Some(row) = self.rows.iter().find(|r| r.key == key) else {
            return Vec::new();
        };
        let (session_id, path) = (row.session_id.clone(), row.path.clone());
        self.open_file(session_id, path)
    }

    pub fn open_file(&mut self, session_id: String, path: String) -> Vec<IdeAction> {
        let same = self
            .open
            .as_ref()
            .is_some_and(|o| o.session_id == session_id && o.path == path);
        if self.edit_dirty() && !same {
            self.say(
                ToastLevel::Info,
                "unsaved edits: Ctrl+S saves, Esc discards".to_string(),
            );
            return Vec::new();
        }
        let tracked = self.tracked(&session_id, &path);
        if !same {
            self.tools.edit = None;
            self.tools.why_popup = None;
        }
        self.open = Some(OpenFile {
            session_id,
            path,
            loading: false,
            error: None,
            hunk: 0,
            scroll: 0,
            view: if tracked { View::Diff } else { View::Edit },
            seek: None,
        });
        self.confirm = None;
        self.advance_coach(0);
        let mut actions = if tracked {
            let mut actions = self.ensure_diff();
            actions.extend(self.ensure_blame());
            actions
        } else {
            self.start_edit(None)
        };
        actions.extend(self.sync_watch());
        actions
    }

    pub fn step_file(&mut self, step: i32) -> Vec<IdeAction> {
        let current = self.open_row().map(|r| r.key.clone());
        let Some(next) = step_queue(&self.rows, current.as_deref(), step).map(|r| r.key.clone())
        else {
            return Vec::new();
        };
        self.open_key(&next)
    }

    pub fn step_hunk(&mut self, step: i32) {
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let (_, starts) = parsed.layout(self.split);
        if starts.is_empty() {
            return;
        }
        let split = self.split;
        let view_h = self.view_h;
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let max = starts.len() as i32 - 1;
        open.hunk = (open.hunk as i32 + step).clamp(0, max) as usize;
        let (rows, starts) = parsed.layout(split);
        open.scroll = reveal(
            open.scroll,
            starts[open.hunk],
            hunk_end(rows.len(), starts, open.hunk),
            view_h,
        );
    }

    pub fn scroll_by(&mut self, delta: i32) {
        if self.editing() {
            self.edit_scroll_by(delta);
            return;
        }
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let (rows, starts) = parsed.layout(self.split);
        let max = rows.len().saturating_sub(self.view_h.max(1));
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.scroll = (open.scroll as i64 + i64::from(delta)).clamp(0, max as i64) as usize;
        // The cursor follows the viewport: the hunk whose rows are at the top.
        if let Some(index) = starts.iter().rposition(|&s| s <= open.scroll) {
            open.hunk = index;
        }
    }

    pub fn set_hunk(&mut self, hunk: usize) {
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let (rows, starts) = parsed.layout(self.split);
        let view_h = self.view_h;
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if hunk < starts.len() {
            open.hunk = hunk;
            open.scroll = reveal(
                open.scroll,
                starts[hunk],
                hunk_end(rows.len(), starts, hunk),
                view_h,
            );
        }
    }

    pub fn cycle_pane(&mut self, back: bool) {
        let mut order = vec![Pane::Rail, Pane::Editor, Pane::Agent];
        if self.bottom_open {
            order.push(Pane::Bottom);
        }
        let at = order.iter().position(|p| *p == self.focus).unwrap_or(0) as i32;
        let step = if back { -1 } else { 1 };
        let next = (at + step).rem_euclid(order.len() as i32) as usize;
        self.focus = order[next];
    }

    pub fn toggle_bottom(&mut self) -> Vec<IdeAction> {
        self.bottom_open = !self.bottom_open;
        if !self.bottom_open && self.focus == Pane::Bottom {
            self.focus = Pane::Editor;
        } else if self.bottom_open {
            self.focus = Pane::Bottom;
            return self.activate_bottom();
        }
        Vec::new()
    }

    pub fn set_rail_tab(&mut self, tab: RailTab) -> Vec<IdeAction> {
        self.rail_tab = tab;
        self.rail_focus = RailFocus::List;
        self.focus = Pane::Rail;
        if tab == RailTab::Files {
            return self.ensure_files();
        }
        if tab == RailTab::Git {
            return self
                .panel_session()
                .map(|id| vec![IdeAction::FetchGitStatus(self.route_session(&id))])
                .unwrap_or_default();
        }
        Vec::new()
    }

    pub fn focus_agents(&mut self) {
        self.focus = Pane::Rail;
        self.rail_focus = RailFocus::Agents;
    }

    // ------------------------------------------------------------------ reviewing

    /// Mark the open file reviewed (or clear the mark) and move to the next file that still needs
    /// a look.
    pub fn toggle_reviewed(&mut self) -> Vec<IdeAction> {
        let Some(row) = self.open_row().cloned() else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        let was = row.tier == Tier::Reviewed;
        let changed = if was {
            unmark_reviewed(&mut self.marks, &row.session_id, &row.path)
        } else {
            mark_reviewed(&mut self.marks, &row.session_id, &row.path, &row.file.hash)
        };
        if changed {
            actions.push(self.save_marks());
        }
        self.rebuild_queue();
        if !was {
            if let Some(next) = next_to_review(&self.rows, &row.key).map(|r| r.key.clone()) {
                actions.extend(self.open_key(&next));
            }
        }
        actions
    }

    fn current_hunk(&self) -> Option<(Rc<ParsedDiff>, Bucket, usize)> {
        let (parsed, bucket) = self.parsed()?;
        let hunk = self.open.as_ref()?.hunk;
        (hunk < parsed.wire.hunks.len()).then_some((parsed, bucket, hunk))
    }

    fn read_only_session(&self, session_id: &str) -> bool {
        self.agents
            .iter()
            .any(|a| a.id == session_id && a.read_only)
    }

    /// `a`: stage the hunk under the cursor (unstage when the diff is the staged side).
    pub fn accept_hunk(&mut self) -> Vec<IdeAction> {
        let Some((parsed, bucket, index)) = self.current_hunk() else {
            return Vec::new();
        };
        let Some(open) = &self.open else {
            return Vec::new();
        };
        let read_only = self.read_only_session(&self.route_session(&open.session_id));
        let allowed = actions_for(bucket, &parsed.wire, read_only);
        let action = if allowed.contains(&HunkAction::Stage) {
            HunkAction::Stage
        } else if allowed.contains(&HunkAction::Unstage) {
            HunkAction::Unstage
        } else {
            self.say(
                ToastLevel::Info,
                "this hunk cannot be staged on its own".to_string(),
            );
            return Vec::new();
        };
        self.hunk_action(&parsed, index, action)
    }

    fn hunk_action(
        &mut self,
        parsed: &ParsedDiff,
        index: usize,
        action: HunkAction,
    ) -> Vec<IdeAction> {
        let Some(open) = &self.open else {
            return Vec::new();
        };
        let hunk = &parsed.wire.hunks[index];
        let patch = build_patch(&parsed.wire.path, &parsed.wire.kind, hunk);
        let undo = (action == HunkAction::Revert).then(|| UndoRecord {
            session_id: self.route_session(&open.session_id),
            path: parsed.wire.path.clone(),
            label: format!("hunk {}", hunk.header),
            steps: vec![undo_step_for_hunk(&parsed.wire.path, hunk)],
        });
        vec![IdeAction::Hunk {
            session_id: self.route_session(&open.session_id),
            path: parsed.wire.path.clone(),
            header: hunk.header.clone(),
            patch,
            action,
            undo,
            is_undo: false,
        }]
    }

    /// `x`: ask before reverting the hunk under the cursor.
    pub fn request_revert_hunk(&mut self) {
        let Some((parsed, bucket, index)) = self.current_hunk() else {
            return;
        };
        let read_only = self
            .open
            .as_ref()
            .is_some_and(|o| self.read_only_session(&self.route_session(&o.session_id)));
        if !actions_for(bucket, &parsed.wire, read_only).contains(&HunkAction::Revert) {
            self.say(
                ToastLevel::Info,
                "only working-tree hunks can be reverted: unstage this one first".to_string(),
            );
            return;
        }
        let header = parsed.wire.hunks[index].header.clone();
        self.confirm = Some(Confirm {
            text: format!("revert hunk {header}?  y/n"),
            kind: ConfirmKind::RevertHunk { header },
        });
    }

    /// `X`: ask before reverting the whole file.
    pub fn request_revert_file(&mut self) {
        let Some(open) = &self.open else {
            return;
        };
        self.confirm = Some(Confirm {
            text: format!("revert all changes to {}?  y/n", open.path),
            kind: ConfirmKind::RevertFile,
        });
    }

    pub fn answer_confirm(&mut self, yes: bool) -> Vec<IdeAction> {
        let Some(confirm) = self.confirm.take() else {
            return Vec::new();
        };
        if !yes {
            self.say(ToastLevel::Info, "cancelled".to_string());
            return Vec::new();
        }
        match confirm.kind {
            ConfirmKind::DiscardEdits => {
                self.leave_edit();
                Vec::new()
            }
            ConfirmKind::QuitDiscard => {
                self.tools.edit = None;
                vec![IdeAction::Close]
            }
            ConfirmKind::SaveConflict => Vec::new(),
            ConfirmKind::RevertHunk { header } => {
                let Some((parsed, _, _)) = self.current_hunk() else {
                    return Vec::new();
                };
                match parsed.wire.hunks.iter().position(|h| h.header == header) {
                    Some(index) => self.hunk_action(&parsed, index, HunkAction::Revert),
                    None => Vec::new(),
                }
            }
            ConfirmKind::RevertFile => {
                let Some(open) = &self.open else {
                    return Vec::new();
                };
                let undo = self.parsed().and_then(|(parsed, _)| {
                    let steps = undo_steps_for_file(&parsed.wire);
                    (!steps.is_empty()).then(|| UndoRecord {
                        session_id: self.route_session(&open.session_id),
                        path: parsed.wire.path.clone(),
                        label: format!("file {}", parsed.wire.path),
                        steps,
                    })
                });
                vec![IdeAction::DiscardFile {
                    session_id: self.route_session(&open.session_id),
                    path: open.path.clone(),
                    undo,
                }]
            }
        }
    }

    /// `u`: put back the last revert by re-applying its reversed patch(es).
    pub fn undo_last(&mut self) -> Vec<IdeAction> {
        let Some(record) = self.undo.pop() else {
            self.say(ToastLevel::Info, "nothing to undo".to_string());
            return Vec::new();
        };
        let session_id = record.session_id.clone();
        record
            .steps
            .iter()
            .map(|step| IdeAction::Hunk {
                session_id: session_id.clone(),
                path: record.path.clone(),
                header: step.hunk_header.clone(),
                patch: step.patch.clone(),
                action: HunkAction::Revert,
                undo: None,
                is_undo: true,
            })
            .collect()
    }

    // ------------------------------------------------------------------ the agent

    /// Y / N: answer the panel agent's pending permission prompt, seq-checked like the board.
    pub fn answer_permission(&mut self, yes: bool) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        let Some(snap) = self.snapshots.get(&id) else {
            return Vec::new();
        };
        if snap.permission_prompt.is_none() {
            self.say(
                ToastLevel::Info,
                "nothing is waiting on a permission".to_string(),
            );
            return Vec::new();
        }
        let seq = snap.prompt_seq;
        self.say(
            ToastLevel::Ok,
            if yes { "allowed" } else { "denied" }.to_string(),
        );
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "allow", "yes": yes, "seq": seq }),
        )]
    }

    /// 1-9 on a question: pick option `n` (1-based).
    pub fn answer_option(&mut self, n: usize) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        let Some(snap) = self.snapshots.get(&id) else {
            return Vec::new();
        };
        if snap.question.is_none() || n == 0 || n > snap.question_options.len() {
            return Vec::new();
        }
        let seq = snap.prompt_seq;
        let label = snap.question_options[n - 1].label.clone();
        self.say(ToastLevel::Ok, format!("answered: {label}"));
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "answer", "text": n.to_string(), "seq": seq }),
        )]
    }

    pub fn start_composer(&mut self) {
        if self.panel_session().is_none() {
            return;
        }
        self.focus = Pane::Agent;
        self.composer.active = true;
    }

    pub fn send_composer(&mut self) -> Vec<IdeAction> {
        let text = self.composer.text.trim().to_string();
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        if text.is_empty() {
            self.composer.active = false;
            return Vec::new();
        }
        if self.read_only_session(&id) {
            self.say(
                ToastLevel::Info,
                "view only: this session runs in a terminal".to_string(),
            );
            return Vec::new();
        }
        self.composer = Input::default();
        self.say(ToastLevel::Ok, "sent".to_string());
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "prompt", "text": text }),
        )]
    }

    /// Enter on a timeline edit: open the file it touched.
    pub fn open_timeline(&mut self, index: usize) -> Vec<IdeAction> {
        let Some(item) = self.timeline.get(index).cloned() else {
            return Vec::new();
        };
        self.timeline_cursor = index;
        if item.kind != TimelineKind::Edit {
            return Vec::new();
        }
        let Some(written) = item.path else {
            return Vec::new();
        };
        let Some(agent) = self.panel_session() else {
            return Vec::new();
        };
        let changed: Vec<String> = self
            .queue_agents
            .iter()
            .filter(|a| a.session_id == agent || a.session_id == UNATTRIBUTED)
            .flat_map(|a| a.files.iter().map(|f| f.path.clone()))
            .collect();
        let Some(path) = resolve_changed_path(&written, &changed) else {
            self.say(
                ToastLevel::Info,
                "that file has no changes left".to_string(),
            );
            return Vec::new();
        };
        let session = self
            .queue_agents
            .iter()
            .find(|a| {
                (a.session_id == agent || a.session_id == UNATTRIBUTED)
                    && a.files.iter().any(|f| f.path == path)
            })
            .map_or(agent, |a| a.session_id.clone());
        self.focus = Pane::Editor;
        let mut actions = self.open_file(session, path);
        if let Some(open) = self.open.as_mut() {
            open.seek = item.needle.clone();
        }
        self.seek_hunk();
        actions.extend(self.sync_watch());
        actions
    }

    /// Put the hunk cursor on the hunk that holds the line an edit wrote, once that diff is here.
    fn seek_hunk(&mut self) {
        let Some(needle) = self.open.as_ref().and_then(|o| o.seek.clone()) else {
            return;
        };
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let wanted = needle.trim();
        let found = parsed.overlay.hunks.iter().position(|h| {
            h.lines
                .iter()
                .any(|l| l.kind != crate::diff_overlay::LineKind::Del && l.text.trim() == wanted)
        });
        if let Some(open) = self.open.as_mut() {
            open.seek = None;
        }
        if let Some(index) = found {
            self.set_hunk(index);
        }
    }

    pub fn move_timeline(&mut self, step: i32) {
        if self.timeline.is_empty() {
            return;
        }
        let max = self.timeline.len() as i32 - 1;
        self.timeline_cursor = (self.timeline_cursor as i32 + step).clamp(0, max) as usize;
    }

    // ------------------------------------------------------------------ git

    pub fn move_git(&mut self, step: i32) {
        let len = self.git_list().len();
        if len == 0 {
            return;
        }
        self.git_cursor = (self.git_cursor as i32 + step).clamp(0, len as i32 - 1) as usize;
    }

    /// Enter on a Git row: show that file's diff (staged side for a staged row).
    pub fn open_git_row(&mut self, index: usize) -> Vec<IdeAction> {
        let list = self.git_list();
        let Some((_, _, row)) = list.get(index) else {
            return Vec::new();
        };
        self.git_cursor = index;
        let path = row.path.clone();
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        // Prefer the queue's own row for the file so reviewed marks and hunks line up.
        let owner = self
            .queue_agents
            .iter()
            .find(|a| a.session_id == session && a.files.iter().any(|f| f.path == path))
            .or_else(|| {
                self.queue_agents
                    .iter()
                    .find(|a| a.files.iter().any(|f| f.path == path))
            })
            .map(|a| a.session_id.clone());
        match owner {
            Some(owner) => {
                self.focus = Pane::Editor;
                self.open_file(owner, path)
            }
            None => {
                self.say(
                    ToastLevel::Info,
                    "that file is not in the change queue yet".to_string(),
                );
                Vec::new()
            }
        }
    }

    /// `s` / Space on a Git row: stage it, or unstage it when it is staged.
    pub fn toggle_stage_git_row(&mut self) -> Vec<IdeAction> {
        let list = self.git_list();
        let Some((bucket, conflicted, row)) = list.get(self.git_cursor) else {
            return Vec::new();
        };
        if *conflicted {
            self.say(
                ToastLevel::Info,
                "resolve the conflict in your editor first".to_string(),
            );
            return Vec::new();
        }
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        vec![IdeAction::StagePaths {
            session_id: self.route_session(&session),
            paths: vec![row.path.clone()],
            stage: *bucket != Bucket::Staged,
        }]
    }

    pub fn staged_count(&self) -> usize {
        self.panel_session()
            .and_then(|id| self.git.get(&id))
            .map_or(0, |s| s.staged.len())
    }

    pub fn commit_now(&mut self) -> Vec<IdeAction> {
        let message = self.full_commit_message();
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        if message.is_empty() {
            self.say(ToastLevel::Info, "write a commit message first".to_string());
            return Vec::new();
        }
        if self.staged_count() == 0 {
            self.say(
                ToastLevel::Info,
                "nothing is staged: stage a file with s first".to_string(),
            );
            return Vec::new();
        }
        self.commit = Input::default();
        self.tools.suggested_body = None;
        vec![IdeAction::Commit {
            session_id: self.route_session(&session),
            message,
        }]
    }

    pub fn push_now(&mut self) -> Vec<IdeAction> {
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        let ahead = self.git.get(&session).map_or(0, |s| s.ahead);
        if ahead == 0 {
            self.say(ToastLevel::Info, "no commits to push".to_string());
            return Vec::new();
        }
        vec![IdeAction::Push {
            session_id: self.route_session(&session),
        }]
    }

    // ------------------------------------------------------------------ text input

    pub fn type_char(&mut self, ch: char) {
        if self.composer.active {
            self.composer.push(ch);
        } else if self.tools.comment.active {
            self.tools.comment.push(ch);
        } else if self.commit.active {
            self.commit.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        let input = if self.composer.active {
            &mut self.composer
        } else if self.tools.comment.active {
            &mut self.tools.comment
        } else {
            &mut self.commit
        };
        input.text.pop();
    }

    pub fn clear_input(&mut self) {
        let input = if self.composer.active {
            &mut self.composer
        } else if self.tools.comment.active {
            &mut self.tools.comment
        } else {
            &mut self.commit
        };
        input.text.clear();
    }

    pub fn paste(&mut self, text: &str) {
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                self.type_char(' ');
            } else {
                self.type_char(ch);
            }
        }
    }

    pub fn is_reviewed_file(&self, session_id: &str, file: &ChangedFile) -> bool {
        is_reviewed(&self.marks, session_id, &file.path, &file.hash)
    }
}

fn short(id: &str) -> &str {
    let end = id.char_indices().nth(8).map_or(id.len(), |(i, _)| i);
    &id[..end]
}

fn hunk_end(total: usize, starts: &[usize], hunk: usize) -> usize {
    starts.get(hunk + 1).copied().unwrap_or(total)
}

/// Scroll so rows `start..end` are visible, preferring the start of the hunk at the top.
fn reveal(scroll: usize, start: usize, end: usize, view_h: usize) -> usize {
    let view_h = view_h.max(1);
    if start < scroll || end > scroll + view_h {
        start
    } else {
        scroll
    }
}
