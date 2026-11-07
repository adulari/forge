//! `/ci`: the repository's workflows as a local plan - which steps run, which pass or fail, which
//! are skipped and why - with a one-key run. Data and key handling only; the binary derives the
//! plan, runs it off the render loop and hands rows back, and [`render`] draws the card.

use crate::app::KeyKind;
pub use crate::import_menu::{Notice, Tone};

mod render;
pub(crate) use render::render_ci_menu;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiState {
    /// In the plan, no result yet.
    Pending,
    Pass,
    /// Passed earlier on this exact tree.
    Cached,
    Fail,
    /// Not run because an earlier step failed, or the tool is not installed here.
    NotRun,
    /// Left out of the plan; the detail says why.
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRow {
    pub state: CiState,
    pub name: String,
    /// The command for a step, the reason for a skip.
    pub detail: String,
    pub secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiAction {
    None,
    Close,
    Run,
}

#[derive(Debug, Clone, Default)]
pub struct CiMenu {
    pub open: bool,
    pub loading: bool,
    pub running: bool,
    pub rows: Vec<CiRow>,
    pub cursor: usize,
    /// `Last run 3 min ago: 12 passed, 1 failed`.
    pub summary: Option<String>,
    pub notice: Option<Notice>,
    pub tick: u32,
    refresh: bool,
}

impl CiMenu {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            loading: true,
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

    pub fn apply(&mut self, rows: Vec<CiRow>, summary: Option<String>) {
        self.loading = false;
        self.running = false;
        self.rows = rows;
        self.summary = summary;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
        });
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let n = |f: fn(CiState) -> bool| self.rows.iter().filter(|r| f(r.state)).count();
        (
            n(|s| matches!(s, CiState::Pass | CiState::Cached)),
            n(|s| s == CiState::Fail),
            n(|s| s == CiState::Skip),
        )
    }

    pub fn key(&mut self, key: KeyKind) -> CiAction {
        self.tick = self.tick.wrapping_add(1);
        self.notice = None;
        let last = self.rows.len().saturating_sub(1);
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                self.close();
                return CiAction::Close;
            }
            KeyKind::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyKind::Down => self.cursor = (self.cursor + 1).min(last),
            KeyKind::Home => self.cursor = 0,
            KeyKind::End => self.cursor = last,
            KeyKind::PageUp => self.cursor = self.cursor.saturating_sub(8),
            KeyKind::PageDown => self.cursor = (self.cursor + 8).min(last),
            KeyKind::Char('R') => {
                self.loading = true;
                self.refresh = true;
            }
            KeyKind::Char('r') | KeyKind::Enter => {
                if self.running || self.loading {
                    return CiAction::None;
                }
                self.running = true;
                return CiAction::Run;
            }
            _ => {}
        }
        CiAction::None
    }
}
