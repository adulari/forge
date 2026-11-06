//! `/issues`: the project's GitHub issues, with a one-key start. Data and key handling only; the
//! binary asks the daemon for the list and for the start, and [`render`] draws the card.

use crate::app::KeyKind;
pub use crate::import_menu::{Notice, Tone};

mod render;
pub(crate) use render::render_issues_menu;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueRow {
    pub number: u64,
    pub title: String,
    pub labels: Vec<String>,
    pub author: String,
    pub comments: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready,
    Failed(String),
}

/// PR Watch mode the started session gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WatchChoice {
    Off,
    #[default]
    Fix,
    Merge,
}

impl WatchChoice {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::Fix,
            Self::Fix => Self::Merge,
            Self::Merge => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fix => "fix",
            Self::Merge => "fix + merge",
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fix => "fix",
            Self::Merge => "merge",
        }
    }
}

/// The spend envelope a drain stops at: a slice of the weekly Claude window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BudgetChoice {
    #[default]
    None,
    Pct10,
    Pct20,
    Pct30,
    Pct50,
}

impl BudgetChoice {
    fn next(self) -> Self {
        match self {
            Self::None => Self::Pct10,
            Self::Pct10 => Self::Pct20,
            Self::Pct20 => Self::Pct30,
            Self::Pct30 => Self::Pct50,
            Self::Pct50 => Self::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pct10 => "10% weekly",
            Self::Pct20 => "20% weekly",
            Self::Pct30 => "30% weekly",
            Self::Pct50 => "50% weekly",
        }
    }

    /// The `--budget` spec the daemon parses.
    pub fn wire(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Pct10 => Some("claude:10%w"),
            Self::Pct20 => Some("claude:20%w"),
            Self::Pct30 => Some("claude:30%w"),
            Self::Pct50 => Some("claude:50%w"),
        }
    }
}

pub const MAX_PARALLEL: usize = 8;

/// One backlog drain as the daemon reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainView {
    pub id: String,
    /// `active`, `paused`, `finished` or `cancelled`.
    pub status: String,
    /// `Drain: 7 started, 2 running, 4 merged, 2 waiting on you, 1 blocked`.
    pub line: String,
    pub meter: Option<String>,
    pub paused_reason: Option<String>,
    /// Issue number and its status (`queued`, `running`, `pr_open`, ...).
    pub items: Vec<(u64, String)>,
}

impl DrainView {
    pub fn is_live(&self) -> bool {
        matches!(self.status.as_str(), "active" | "paused")
    }

    fn item_label(&self, number: u64) -> Option<&'static str> {
        let status = self.items.iter().find(|(n, _)| *n == number)?.1.as_str();
        Some(match status {
            "queued" => "queued",
            "running" => "running",
            "waiting" => "needs you",
            "pr_open" => "PR open",
            "merged" => "merged",
            "done" => "idle",
            "failed" => "failed",
            "blocked" => "blocked",
            "cancelled" => "cancelled",
            _ => return None,
        })
    }
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssuesAction {
    None,
    Close,
    Start {
        number: u64,
        worktree: bool,
        watch: WatchChoice,
    },
    /// Run the marked issues as a drain.
    Drain {
        numbers: Vec<u64>,
        worktree: bool,
        watch: WatchChoice,
        parallel: usize,
        budget: Option<&'static str>,
    },
    CancelDrain {
        id: String,
    },
}

#[derive(Debug, Clone)]
pub struct IssuesMenu {
    pub open: bool,
    pub load: Load,
    pub rows: Vec<IssueRow>,
    pub cursor: usize,
    pub worktree: bool,
    pub watch: WatchChoice,
    pub starting: bool,
    pub notice: Option<Notice>,
    pub tick: u32,
    /// Issues marked for a drain, in the order they were marked.
    pub marked: Vec<u64>,
    pub parallel: usize,
    pub budget: BudgetChoice,
    /// The drain this project is running now, or its latest.
    pub drain: Option<DrainView>,
    /// `x` was pressed once on a live drain; a second press cancels it.
    pub confirm_cancel: bool,
    refresh: bool,
}

impl Default for IssuesMenu {
    fn default() -> Self {
        Self {
            open: false,
            load: Load::Loading,
            rows: Vec::new(),
            cursor: 0,
            worktree: true,
            watch: WatchChoice::default(),
            starting: false,
            notice: None,
            tick: 0,
            marked: Vec::new(),
            parallel: 2,
            budget: BudgetChoice::default(),
            drain: None,
            confirm_cancel: false,
            refresh: false,
        }
    }
}

impl IssuesMenu {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            refresh: true,
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    pub fn apply_list(&mut self, result: Result<Vec<IssueRow>, String>) {
        match result {
            Ok(rows) => {
                self.rows = rows;
                self.load = Load::Ready;
                self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
            }
            Err(error) => self.load = Load::Failed(error),
        }
    }

    pub fn finish_start(&mut self, result: Result<String, String>) {
        self.starting = false;
        match result {
            Ok(text) => self.notify(Tone::Ok, text),
            Err(error) => self.notify(Tone::Err, error),
        }
    }

    /// A drain was queued: the marks have done their job.
    pub fn finish_drain(&mut self, result: Result<String, String>) {
        if result.is_ok() {
            self.marked.clear();
        }
        self.finish_start(result);
    }

    /// The daemon's drains (newest first): show the live one, else the latest.
    pub fn apply_drains(&mut self, drains: Vec<DrainView>) {
        self.drain = drains
            .iter()
            .find(|d| d.is_live())
            .or_else(|| drains.first())
            .cloned();
        self.confirm_cancel &= self.drain.as_ref().is_some_and(DrainView::is_live);
    }

    fn toggle_mark(&mut self) {
        let Some(row) = self.rows.get(self.cursor) else {
            return;
        };
        match self.marked.iter().position(|n| *n == row.number) {
            Some(at) => {
                self.marked.remove(at);
            }
            None => self.marked.push(row.number),
        }
        self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1));
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
        });
    }

    pub fn key(&mut self, key: KeyKind) -> IssuesAction {
        self.tick = self.tick.wrapping_add(1);
        if self.starting {
            return IssuesAction::None;
        }
        self.notice = None;
        if !matches!(key, KeyKind::Char('x')) {
            self.confirm_cancel = false;
        }
        let last = self.rows.len().saturating_sub(1);
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                self.close();
                return IssuesAction::Close;
            }
            KeyKind::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyKind::Down => self.cursor = (self.cursor + 1).min(last),
            KeyKind::Home => self.cursor = 0,
            KeyKind::End => self.cursor = last,
            KeyKind::PageUp => self.cursor = self.cursor.saturating_sub(8),
            KeyKind::PageDown => self.cursor = (self.cursor + 8).min(last),
            KeyKind::Tab => self.watch = self.watch.next(),
            KeyKind::Char('w') => self.worktree = !self.worktree,
            KeyKind::Char(' ' | 'm') => self.toggle_mark(),
            KeyKind::Char('u') => self.marked.clear(),
            KeyKind::Char('b') => self.budget = self.budget.next(),
            KeyKind::Char('+') => self.parallel = (self.parallel + 1).min(MAX_PARALLEL),
            KeyKind::Char('-') => self.parallel = self.parallel.saturating_sub(1).max(1),
            KeyKind::Char('D') => {
                if self.marked.is_empty() {
                    self.notify(Tone::Warn, "Mark issues with Space first, then press D");
                } else {
                    self.starting = true;
                    return IssuesAction::Drain {
                        numbers: self.marked.clone(),
                        worktree: self.worktree,
                        watch: self.watch,
                        parallel: self.parallel,
                        budget: self.budget.wire(),
                    };
                }
            }
            KeyKind::Char('x') => {
                if let Some(drain) = self.drain.as_ref().filter(|d| d.is_live()) {
                    if self.confirm_cancel {
                        self.confirm_cancel = false;
                        return IssuesAction::CancelDrain {
                            id: drain.id.clone(),
                        };
                    }
                    self.confirm_cancel = true;
                    self.notify(
                        Tone::Warn,
                        "Press x again to stop the drain (running sessions keep going)",
                    );
                }
            }
            KeyKind::Char('r') => {
                self.load = Load::Loading;
                self.refresh = true;
            }
            KeyKind::Enter => {
                if let Some(row) = self.rows.get(self.cursor) {
                    self.starting = true;
                    return IssuesAction::Start {
                        number: row.number,
                        worktree: self.worktree,
                        watch: self.watch,
                    };
                }
            }
            _ => {}
        }
        IssuesAction::None
    }
}
