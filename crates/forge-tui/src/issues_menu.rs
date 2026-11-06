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
