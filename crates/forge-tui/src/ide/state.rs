//! IDE mode's state machine. Pure: folds [`IdeEvent`]s in, answers every user intent with
//! [`IdeAction`]s for the host (`forge-cli`'s `ide_host`), and is drawn by `render`. It never
//! touches the network, the disk or the terminal. Navigation and review live in `nav`, the agent,
//! git and text input in `ops`.

pub use super::types::*;

mod tools;
pub use tools::{
    BlameData, ChecksState, CommentTarget, EditState, FileRow, FilesState, HlCache, JobsState,
    SaveFailure, TermState, Tools, View, WhyPopup, COACH,
};

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ratatui::layout::Rect;

use crate::board::{project_name, ConnState, FleetRow, HistoryRow, LiveSnapshot, ToastLevel};

use super::hunk::{Bucket, UndoRecord};
use super::model::{
    build_queue, flatten_queue, marks_to_json, prune_reviewed, ChangedFile, QueueAgent, QueueGroup,
    QueueRow, ReviewedMarks, Tier,
};
use super::timeline::{build_timeline, TimelineItem};
use super::wire::{GitStatus, ProjectChanges, WireSessionChanges, UNATTRIBUTED};

pub struct IdeApp {
    pub(crate) cwd: String,
    pub(crate) project: String,
    pub(crate) now: i64,
    pub(crate) tick: u64,
    pub(crate) size: (u16, u16),
    pub(crate) connection: ConnState,
    pub(super) fleet: Vec<FleetRow>,
    pub(super) changes: Option<ProjectChanges>,
    pub(crate) changes_note: Option<String>,
    pub(crate) agents: Vec<AgentRow>,
    pub(super) queue_agents: Vec<QueueAgent>,
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
    pub(super) cache: HashMap<(String, String), CachedDiff>,
    pub(super) inflight: HashSet<(String, String)>,
    pub(crate) git: HashMap<String, GitStatus>,
    pub(crate) snapshots: HashMap<String, LiveSnapshot>,
    pub(super) history: HashMap<String, Vec<HistoryRow>>,
    pub(crate) timeline: Vec<TimelineItem>,
    pub(super) timeline_for: Option<String>,
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
    pub(super) watching: Option<String>,
    pub(super) marks_loaded: bool,
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

    pub(super) fn is_worktree(&self, session_id: &str) -> bool {
        self.changes.as_ref().is_some_and(|c| {
            c.sessions
                .iter()
                .any(|s| s.session_id == session_id && s.worktree.is_some())
        })
    }

    pub(super) fn file_hash(&self, session_id: &str, path: &str) -> Option<&str> {
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

    pub(super) fn rebuild_agents(&mut self) {
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

    pub(super) fn rebuild_queue(&mut self) {
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
    pub(super) fn after_changes(&mut self) -> Vec<IdeAction> {
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

    pub(super) fn save_marks(&self) -> IdeAction {
        IdeAction::SaveReviewed(marks_to_json(&self.marks, MARKS_CAP))
    }

    /// The fetch for the open file when its cached diff is missing or out of date.
    pub(super) fn ensure_diff(&mut self) -> Vec<IdeAction> {
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

    pub(super) fn apply_diff(&mut self, loaded: DiffLoaded) {
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

    pub(super) fn apply_hunk(&mut self, outcome: HunkOutcome) -> Vec<IdeAction> {
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

    pub(super) fn rebuild_timeline(&mut self) {
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
}

fn short(id: &str) -> &str {
    let end = id.char_indices().nth(8).map_or(id.len(), |(i, _)| i);
    &id[..end]
}
