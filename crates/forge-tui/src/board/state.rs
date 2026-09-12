//! The board's state machine: cards, selection, focus, the composer, confirmations and toasts.
//! Pure — folds [`BoardEvent`]s in, and every user intent comes out as a [`BoardAction`] for the
//! host. Rendering reads this struct; input handling (`keys.rs`) mutates it.

use std::collections::{HashMap, HashSet, VecDeque};

use ratatui::layout::Rect;

use super::model::{live_card, past_card, Card, Column, Signal, SignalLevel};
use super::wire::{FleetRow, GitInfo, HistoryRow, LiveSnapshot, PastRow};
use super::{BoardAction, BoardEvent};

pub use super::types::{
    Button, Composer, ComposerMode, Confirm, ConfirmKind, ConnState, DetailTab, Focus, Hit, Toast,
    ToastLevel,
};

/// Past sessions kept in the Done column (the daemon serves up to 200; a board wants recent).
pub const PAST_LIMIT: usize = 30;
/// Ticks (~100 ms each) a freshly appeared card keeps its entrance highlight.
pub const FLASH_TICKS: u64 = 12;
/// Ticks a card keeps its "just finished" highlight after busy → idle.
pub const FINISHED_FLASH_TICKS: u64 = 30;
/// Ticks an informational toast stays; errors stay twice as long.
pub const TOAST_TICKS: u64 = 40;
/// Signals are time-relative ("quiet for 4m"), so cards are rebuilt this often even without news.
const REBUILD_EVERY_TICKS: u64 = 50;

pub struct BoardApp {
    pub(crate) rows: Vec<FleetRow>,
    pub(crate) past: Vec<PastRow>,
    pub(crate) snapshots: HashMap<String, LiveSnapshot>,
    pub(crate) git: HashMap<String, GitInfo>,
    pub(crate) history: HashMap<String, Vec<HistoryRow>>,
    /// Every card after filtering, in no particular order; `columns` holds the ordering.
    pub(crate) cards: Vec<Card>,
    /// Indices into `cards`, per column, in display order.
    pub(crate) columns: [Vec<usize>; 4],
    pub(crate) selected: Option<String>,
    /// The column the cursor sits in, meaningful even when it is empty.
    pub(crate) cursor_col: Column,
    pub(crate) focus: Focus,
    pub(crate) detail_open: bool,
    pub(crate) detail_tab: DetailTab,
    pub(crate) detail_scroll: usize,
    /// The live tail sticks to the newest line until the user scrolls up.
    pub(crate) tail_follow: bool,
    pub(crate) show_tools: bool,
    pub(crate) composer: Option<Composer>,
    pub(crate) confirm: Option<Confirm>,
    pub(crate) toasts: VecDeque<Toast>,
    pub(crate) project_filter: Option<String>,
    pub(crate) query: String,
    pub(crate) tick: u64,
    pub(crate) now: i64,
    pub(crate) size: (u16, u16),
    pub(crate) connection: ConnState,
    pub(crate) hits: Vec<(Rect, Hit)>,
    pub(crate) first_seen: HashMap<String, u64>,
    pub(crate) finished_at: HashMap<String, u64>,
    pub(crate) was_busy: HashMap<String, bool>,
    /// First visible card per column (kept so the selection stays on screen).
    pub(crate) column_scroll: [usize; 4],
    /// Narrow terminals show one column at a time; this is the one.
    pub(crate) column_page: usize,
    /// Where the board was started — the default cwd for a new session.
    pub(crate) board_cwd: Option<String>,
    pub(crate) closed: HashSet<String>,
    /// Sessions the host was told to fetch detail for (so it is not asked twice per open).
    pub(crate) detail_requested: HashSet<String>,
}

impl BoardApp {
    pub fn new(board_cwd: Option<String>, now: i64) -> Self {
        Self {
            rows: Vec::new(),
            past: Vec::new(),
            snapshots: HashMap::new(),
            git: HashMap::new(),
            history: HashMap::new(),
            cards: Vec::new(),
            columns: Default::default(),
            selected: None,
            cursor_col: Column::Attention,
            focus: Focus::Board,
            detail_open: false,
            detail_tab: DetailTab::Overview,
            detail_scroll: 0,
            tail_follow: true,
            show_tools: true,
            composer: None,
            confirm: None,
            toasts: VecDeque::new(),
            project_filter: None,
            query: String::new(),
            tick: 0,
            now,
            size: (0, 0),
            connection: ConnState::Connecting,
            hits: Vec::new(),
            first_seen: HashMap::new(),
            finished_at: HashMap::new(),
            was_busy: HashMap::new(),
            column_scroll: [0; 4],
            column_page: 0,
            board_cwd,
            closed: HashSet::new(),
            detail_requested: HashSet::new(),
        }
    }

    // ------------------------------------------------------------------ events

    pub fn apply(&mut self, event: BoardEvent) {
        match event {
            BoardEvent::Fleet(rows) => {
                let ids: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
                self.closed.retain(|id| !ids.contains(id.as_str()));
                self.snapshots.retain(|id, _| ids.contains(id.as_str()));
                self.rows = rows;
                self.rebuild();
            }
            BoardEvent::Past(past) => {
                self.past = past;
                self.rebuild();
            }
            BoardEvent::Snapshot(id, snap) => {
                if snap.closed {
                    self.closed.insert(id.clone());
                }
                if let Some(prev) = self.snapshots.get(&id) {
                    if let (Some(a), Some(b)) = (prev.revision, snap.revision) {
                        if b < a {
                            return;
                        }
                    }
                }
                self.snapshots.insert(id, snap);
                self.rebuild();
            }
            BoardEvent::SessionClosed(id) => {
                self.closed.insert(id.clone());
                self.snapshots.remove(&id);
                self.rebuild();
            }
            BoardEvent::Git(id, git) => {
                self.git.insert(id, git);
            }
            BoardEvent::History(id, rows) => {
                self.history.insert(id, rows);
            }
            BoardEvent::Toast(level, text) => self.toast(level, text),
            BoardEvent::Connection(state) => {
                if let ConnState::Offline(msg) = &state {
                    if self.connection != state {
                        self.toast(ToastLevel::Error, format!("daemon: {msg}"));
                    }
                }
                self.connection = state;
            }
            BoardEvent::Tick => {
                self.tick += 1;
                self.now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(self.now, |d| d.as_secs() as i64);
                let tick = self.tick;
                self.toasts.retain(|t| {
                    let life = if t.level == ToastLevel::Error {
                        TOAST_TICKS * 2
                    } else {
                        TOAST_TICKS
                    };
                    tick.saturating_sub(t.born) < life
                });
                if self.tick.is_multiple_of(REBUILD_EVERY_TICKS) {
                    self.rebuild();
                }
            }
            BoardEvent::Resize(w, h) => self.size = (w, h),
        }
    }

    pub fn toast(&mut self, level: ToastLevel, text: impl Into<String>) {
        self.toasts.push_back(Toast {
            level,
            text: text.into(),
            born: self.tick,
        });
        while self.toasts.len() > 4 {
            self.toasts.pop_front();
        }
    }

    /// Recompute every card and column from the raw rows/snapshots, keeping the selection.
    pub(crate) fn rebuild(&mut self) {
        let mut cards: Vec<Card> = Vec::new();
        // A closed socket never hides a session the daemon still lists: the card falls back to
        // the fleet row's own state, and the next fleet refresh clears the mark so the host
        // reconnects. Only the daemon dropping the row removes the card.
        for row in &self.rows {
            cards.push(live_card(row, self.snapshots.get(&row.id), self.now));
        }
        let live_ids: HashSet<String> = cards.iter().map(|c| c.id.clone()).collect();
        for row in self.past.iter().take(PAST_LIMIT) {
            if !live_ids.contains(&row.id) {
                cards.push(past_card(row));
            }
        }
        cards.retain(|c| self.passes_filter(c));

        for c in &cards {
            self.first_seen.entry(c.id.clone()).or_insert(self.tick);
            let prev = self.was_busy.insert(c.id.clone(), c.busy);
            if prev == Some(true) && !c.busy {
                self.finished_at.insert(c.id.clone(), self.tick);
            }
        }

        let mut columns: [Vec<usize>; 4] = Default::default();
        for (i, c) in cards.iter().enumerate() {
            columns[c.column.index()].push(i);
        }
        for col in &mut columns {
            col.sort_by(|a, b| {
                let (a, b) = (&cards[*a], &cards[*b]);
                severity(b)
                    .cmp(&severity(a))
                    .then_with(|| b.last_activity.cmp(&a.last_activity))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        self.cards = cards;
        self.columns = columns;

        let still_there = self
            .selected
            .as_ref()
            .is_some_and(|id| self.cards.iter().any(|c| &c.id == id));
        if !still_there {
            self.selected = None;
            if self.detail_open && self.focus == Focus::Detail {
                self.focus = Focus::Board;
            }
        }
        if let Some(card) = self.selected_card() {
            self.cursor_col = card.column;
        }
        if self.selected.is_none() {
            self.select_first_in(self.cursor_col)
                .then_some(())
                .or_else(|| {
                    Column::ALL
                        .iter()
                        .find(|c| self.select_first_in(**c))
                        .map(|_| ())
                });
        }
    }

    fn passes_filter(&self, c: &Card) -> bool {
        if let Some(p) = &self.project_filter {
            if &c.project() != p {
                return false;
            }
        }
        let q = self.query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return true;
        }
        let hay = format!(
            "{} {} {} {} {} {}",
            c.title,
            c.id,
            c.model,
            c.cwd,
            c.current_task.as_deref().unwrap_or(""),
            c.worktree.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        q.split_whitespace().all(|w| hay.contains(w))
    }

    // --------------------------------------------------------------- selection

    pub fn selected_card(&self) -> Option<&Card> {
        let id = self.selected.as_ref()?;
        self.cards.iter().find(|c| &c.id == id)
    }

    pub fn selected_snapshot(&self) -> Option<&LiveSnapshot> {
        self.snapshots.get(self.selected.as_ref()?)
    }

    pub fn card(&self, id: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    pub fn column_cards(&self, col: Column) -> Vec<&Card> {
        self.columns[col.index()]
            .iter()
            .map(|i| &self.cards[*i])
            .collect()
    }

    /// Position of the selection inside its column.
    pub fn selected_pos(&self) -> Option<(Column, usize)> {
        let id = self.selected.as_ref()?;
        for col in Column::ALL {
            if let Some(pos) = self.columns[col.index()]
                .iter()
                .position(|i| &self.cards[*i].id == id)
            {
                return Some((col, pos));
            }
        }
        None
    }

    fn select_first_in(&mut self, col: Column) -> bool {
        if let Some(i) = self.columns[col.index()].first() {
            self.selected = Some(self.cards[*i].id.clone());
            self.cursor_col = col;
            true
        } else {
            false
        }
    }

    pub(crate) fn select(&mut self, id: &str) {
        if let Some(c) = self.card(id) {
            self.cursor_col = c.column;
            self.selected = Some(id.to_string());
            self.on_selection_changed();
        }
    }

    fn on_selection_changed(&mut self) {
        self.detail_scroll = 0;
        self.tail_follow = true;
    }

    pub(crate) fn move_in_column(&mut self, delta: i32) {
        let col = self.cursor_col;
        let len = self.columns[col.index()].len();
        if len == 0 {
            return;
        }
        let pos = self
            .selected_pos()
            .filter(|(c, _)| *c == col)
            .map_or(0, |(_, p)| p as i32);
        let next = (pos + delta).clamp(0, len as i32 - 1) as usize;
        let id = self.cards[self.columns[col.index()][next]].id.clone();
        if self.selected.as_deref() != Some(id.as_str()) {
            self.selected = Some(id);
            self.on_selection_changed();
        }
    }

    pub(crate) fn jump_in_column(&mut self, to_end: bool) {
        let len = self.columns[self.cursor_col.index()].len() as i32;
        self.move_in_column(if to_end { len } else { -len });
    }

    /// Move the cursor to the neighbouring column, landing on the card at the same height.
    pub(crate) fn move_column(&mut self, delta: i32) {
        let cur = self.cursor_col.index() as i32;
        let mut next = cur;
        for _ in 0..Column::ALL.len() {
            next = (next + delta).rem_euclid(Column::ALL.len() as i32);
            if !self.columns[next as usize].is_empty() {
                break;
            }
        }
        let col = Column::from_index(next as usize);
        let pos = self.selected_pos().map_or(0, |(_, p)| p);
        let list = &self.columns[col.index()];
        if let Some(i) = list.get(pos.min(list.len().saturating_sub(1))) {
            let id = self.cards[*i].id.clone();
            self.selected = Some(id);
            self.on_selection_changed();
        }
        self.cursor_col = col;
        self.column_page = col.index();
    }

    pub(crate) fn set_cursor_column(&mut self, col: Column) {
        self.cursor_col = col;
        self.column_page = col.index();
        if self.selected_pos().is_none_or(|(c, _)| c != col) {
            self.select_first_in(col);
            self.on_selection_changed();
        }
    }

    /// The window of `capacity` cards to draw for `col`, scrolled so the selection is visible.
    pub(crate) fn visible_window(&mut self, col: Column, capacity: usize) -> (usize, usize) {
        let len = self.columns[col.index()].len();
        let cap = capacity.max(1);
        let selected = self.selected_pos().filter(|(c, _)| *c == col);
        let scroll = &mut self.column_scroll[col.index()];
        if let Some((_, pos)) = selected {
            if pos < *scroll {
                *scroll = pos;
            } else if pos >= *scroll + cap {
                *scroll = pos + 1 - cap;
            }
        }
        *scroll = (*scroll).min(len.saturating_sub(cap));
        (*scroll, (*scroll + cap).min(len))
    }

    // ------------------------------------------------------------------ detail

    pub(crate) fn open_detail(&mut self) -> Vec<BoardAction> {
        let Some(id) = self.selected.clone() else {
            return Vec::new();
        };
        self.detail_open = true;
        self.focus = Focus::Detail;
        self.detail_scroll = 0;
        self.tail_follow = true;
        self.detail_actions_for(&id)
    }

    /// Ask the host for git/history the first time a card is opened (and again on refresh).
    pub(crate) fn detail_actions_for(&mut self, id: &str) -> Vec<BoardAction> {
        let past = self.card(id).is_some_and(|c| c.past);
        if past || !self.detail_requested.insert(id.to_string()) {
            return Vec::new();
        }
        vec![BoardAction::WantDetail(id.to_string())]
    }

    pub(crate) fn close_detail(&mut self) {
        self.detail_open = false;
        self.focus = Focus::Board;
    }

    pub(crate) fn scroll_detail(&mut self, delta: i32) {
        if delta < 0 {
            self.tail_follow = false;
            self.detail_scroll = self.detail_scroll.saturating_sub((-delta) as usize);
        } else {
            self.detail_scroll = self.detail_scroll.saturating_add(delta as usize);
        }
    }

    // -------------------------------------------------------------- composer

    pub(crate) fn open_composer(&mut self, mode: ComposerMode, target: &str, prefill: &str) {
        self.composer = Some(Composer {
            mode,
            target: target.to_string(),
            text: prefill.to_string(),
            cursor: prefill.chars().count(),
        });
        self.focus = Focus::Composer;
    }

    pub(crate) fn cancel_composer(&mut self) {
        self.composer = None;
        self.focus = if self.detail_open {
            Focus::Detail
        } else {
            Focus::Board
        };
    }

    /// Enter in the composer: turn the text into the action the mode promises.
    pub(crate) fn submit_composer(&mut self) -> Vec<BoardAction> {
        let Some(c) = self.composer.take() else {
            return Vec::new();
        };
        self.focus = if self.detail_open {
            Focus::Detail
        } else {
            Focus::Board
        };
        let text = c.text.trim().to_string();
        let mut out = Vec::new();
        match c.mode {
            ComposerMode::Prompt => {
                if !text.is_empty() {
                    out.push(BoardAction::Input(
                        c.target.clone(),
                        serde_json::json!({ "kind": "prompt", "text": text }),
                    ));
                    self.toast(ToastLevel::Ok, "prompt sent");
                }
            }
            ComposerMode::Steer => {
                if !text.is_empty() {
                    out.push(BoardAction::Input(
                        c.target.clone(),
                        serde_json::json!({ "kind": "steer", "text": text }),
                    ));
                    self.toast(
                        ToastLevel::Ok,
                        "steer sent — delivered at the next turn boundary",
                    );
                }
            }
            ComposerMode::Model => {
                let cmd = if text.is_empty() {
                    "/model".to_string()
                } else {
                    format!("/model {text}")
                };
                out.push(BoardAction::Input(
                    c.target.clone(),
                    serde_json::json!({ "kind": "prompt", "text": cmd }),
                ));
                self.toast(
                    ToastLevel::Ok,
                    if text.is_empty() {
                        "pin cleared — routing decides again".to_string()
                    } else {
                        format!("pinned to {text}")
                    },
                );
            }
            ComposerMode::Answer { seq } => {
                if !text.is_empty() {
                    out.push(BoardAction::Input(
                        c.target.clone(),
                        serde_json::json!({ "kind": "answer", "text": text, "seq": seq }),
                    ));
                    self.toast(ToastLevel::Ok, "answer sent");
                }
            }
            ComposerMode::NewSession { cwd, worktree } => {
                if !text.is_empty() {
                    out.push(BoardAction::NewSession {
                        cwd,
                        worktree,
                        prompt: text,
                    });
                    self.toast(ToastLevel::Ok, "starting a new session…");
                }
            }
        }
        out
    }

    // ------------------------------------------------------------ host API

    /// Draw the whole board (columns, pane, overlays) into `frame`, recording click targets.
    pub fn draw(&mut self, frame: &mut ratatui::Frame) {
        super::render::draw(self, frame);
    }

    /// Preselect a project (the cwd's last path component); `None` shows every project.
    pub fn set_project_filter(&mut self, project: Option<String>) {
        self.project_filter = project;
        self.rebuild();
    }

    pub fn project_filter(&self) -> Option<&str> {
        self.project_filter.as_deref()
    }

    /// The session whose detail pane is open, so the host keeps its git status fresh.
    pub fn detail_session(&self) -> Option<String> {
        if self.detail_open {
            self.selected.clone()
        } else {
            None
        }
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    // ------------------------------------------------------------ queries

    /// Sessions the host should hold a WebSocket open for: every live row with an input path
    /// whose socket has not just been declared closed (the mark lifts on the next fleet refresh).
    pub fn watch_ids(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| !r.read_only && !self.closed.contains(&r.id))
            .map(|r| r.id.clone())
            .collect()
    }

    /// Whether anything on screen is animating (spinners, pulses, flashes, toasts).
    pub fn needs_animation(&self) -> bool {
        !self.toasts.is_empty()
            || self.cards.iter().any(|c| c.busy || c.waiting)
            || self
                .first_seen
                .values()
                .any(|t| self.tick.saturating_sub(*t) < FLASH_TICKS)
            || self
                .finished_at
                .values()
                .any(|t| self.tick.saturating_sub(*t) < FINISHED_FLASH_TICKS)
    }

    pub fn is_new(&self, id: &str) -> bool {
        self.first_seen
            .get(id)
            .is_some_and(|t| self.tick.saturating_sub(*t) < FLASH_TICKS && *t > 0)
    }

    pub fn just_finished(&self, id: &str) -> bool {
        self.finished_at
            .get(id)
            .is_some_and(|t| self.tick.saturating_sub(*t) < FINISHED_FLASH_TICKS)
    }

    /// Board-wide totals for the header.
    pub fn totals(&self) -> Totals {
        let live: Vec<&Card> = self.cards.iter().filter(|c| !c.past).collect();
        Totals {
            sessions: live.len(),
            attention: self.columns[Column::Attention.index()].len(),
            working: self.columns[Column::Working.index()].len(),
            ready: self.columns[Column::Ready.index()].len(),
            done: self.columns[Column::Done.index()].len(),
            cost_usd: live.iter().map(|c| c.cost_usd).sum(),
            subagents: live
                .iter()
                .map(|c| c.subagents.iter().filter(|s| !s.done).count())
                .sum(),
        }
    }

    pub fn hit_at(&self, col: u16, row: u16) -> Option<&Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
            .map(|(_, h)| h)
    }

    pub fn strongest_signal(card: &Card) -> Option<&Signal> {
        card.signals.first()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Totals {
    pub sessions: usize,
    pub attention: usize,
    pub working: usize,
    pub ready: usize,
    pub done: usize,
    pub cost_usd: f64,
    pub subagents: usize,
}

/// Ordering weight inside a column: the loudest signal first, waiting ahead of everything.
fn severity(c: &Card) -> u8 {
    let base = match c.health {
        super::model::Health::Waiting => 6,
        super::model::Health::Stalled => 5,
        super::model::Health::Failed => 4,
        super::model::Health::Busy => 3,
        super::model::Health::Idle => 2,
        super::model::Health::Finished => 1,
    };
    let sig = match c.signals.first().map(|s| s.level) {
        Some(SignalLevel::Danger) => 2,
        Some(SignalLevel::Warn) => 1,
        _ => 0,
    };
    base * 3 + sig
}
