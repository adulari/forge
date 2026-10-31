//! `/import`: pick Claude Code and Codex conversations from this computer and bring them into
//! Forge. Data and key handling only; the binary scans, imports and feeds the results back, and
//! [`render`] draws the card.

use std::collections::HashSet;

use crate::app::KeyKind;

mod render;
pub(crate) use render::render_import_menu;

#[cfg(test)]
mod tests;

/// One importable conversation, as the binary scanned it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportRow {
    /// `claude` or `codex`.
    pub agent: String,
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
    /// False when the folder is gone from this machine: the session opens in the fallback instead.
    pub cwd_exists: bool,
    /// Unix seconds the conversation was last written.
    pub modified: i64,
    /// Already in Forge: shown, not selectable.
    pub imported: bool,
}

impl ImportRow {
    fn key(&self) -> String {
        format!("{}:{}", self.agent, self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentFilter {
    #[default]
    All,
    Claude,
    Codex,
}

impl AgentFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Claude,
            Self::Claude => Self::Codex,
            Self::Codex => Self::All,
        }
    }

    fn admits(self, agent: &str) -> bool {
        match self {
            Self::All => true,
            Self::Claude => agent == "claude",
            Self::Codex => agent == "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All agents",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Load {
    #[default]
    Scanning,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Err,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub tone: Tone,
    pub text: String,
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportAction {
    None,
    Close,
    /// `(agent, id)` of each conversation to import.
    Import(Vec<(String, String)>),
}

#[derive(Debug, Clone, Default)]
pub struct ImportMenu {
    pub open: bool,
    pub load: Load,
    pub rows: Vec<ImportRow>,
    pub query: String,
    pub agent: AgentFilter,
    pub cursor: usize,
    selected: HashSet<String>,
    pub importing: bool,
    pub notice: Option<Notice>,
    pub tick: u32,
    /// Where a conversation whose folder is gone opens, shown in the footer note.
    pub fallback: String,
    rescan: bool,
}

/// `Imported 1 session`, `Imported 3 sessions`, with the other outcomes appended.
pub fn summary(imported: usize, skipped: usize, failed: usize) -> String {
    let plural = |n: usize| if n == 1 { "session" } else { "sessions" };
    let mut text = if imported > 0 {
        format!("Imported {imported} {}", plural(imported))
    } else {
        "Nothing new imported".to_string()
    };
    if skipped > 0 {
        text.push_str(&format!(", {skipped} already in Forge"));
    }
    if failed > 0 {
        text.push_str(&format!(", {failed} failed"));
    }
    text
}

impl ImportMenu {
    pub fn open(&mut self, fallback: String) {
        *self = Self {
            open: true,
            fallback,
            rescan: true,
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn take_rescan(&mut self) -> bool {
        std::mem::take(&mut self.rescan)
    }

    pub fn apply_scan(&mut self, result: Result<Vec<ImportRow>, String>) {
        match result {
            Ok(rows) => {
                let alive: HashSet<String> = rows
                    .iter()
                    .filter(|r| !r.imported)
                    .map(ImportRow::key)
                    .collect();
                self.selected.retain(|k| alive.contains(k));
                self.rows = rows;
                self.load = Load::Ready;
                self.clamp();
            }
            Err(error) => self.load = Load::Failed(error),
        }
    }

    pub fn finish_import(&mut self, imported: usize, skipped: usize, failed: usize) {
        self.importing = false;
        self.selected.clear();
        let tone = if failed > 0 { Tone::Warn } else { Tone::Ok };
        self.notify(tone, summary(imported, skipped, failed));
        self.rescan = true;
    }

    pub fn fail_import(&mut self, error: String) {
        self.importing = false;
        self.notify(Tone::Err, error);
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
        });
    }

    /// The rows the search text and agent filter leave, in scan order.
    pub fn visible(&self) -> Vec<&ImportRow> {
        let needle = self.query.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|r| self.agent.admits(&r.agent))
            .filter(|r| {
                needle.is_empty()
                    || r.title.to_lowercase().contains(&needle)
                    || r.cwd
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(&needle))
                    || r.id.to_lowercase().starts_with(&needle)
            })
            .collect()
    }

    pub fn is_selected(&self, row: &ImportRow) -> bool {
        self.selected.contains(&row.key())
    }

    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.visible().len().saturating_sub(1));
    }

    fn toggle_focused(&mut self) {
        let Some(row) = self.visible().get(self.cursor).map(|r| (*r).clone()) else {
            return;
        };
        if row.imported {
            return;
        }
        let key = row.key();
        if !self.selected.remove(&key) {
            self.selected.insert(key);
        }
    }

    fn select_all_visible(&mut self) {
        let keys: Vec<String> = self
            .visible()
            .into_iter()
            .filter(|r| !r.imported)
            .map(ImportRow::key)
            .collect();
        self.selected.extend(keys);
    }

    pub fn key(&mut self, key: KeyKind) -> ImportAction {
        self.tick = self.tick.wrapping_add(1);
        if self.importing {
            return ImportAction::None;
        }
        self.notice = None;
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                if self.query.is_empty() {
                    self.close();
                    return ImportAction::Close;
                }
                self.query.clear();
                self.clamp();
            }
            KeyKind::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyKind::Down => {
                self.cursor = (self.cursor + 1).min(self.visible().len().saturating_sub(1))
            }
            KeyKind::Home => self.cursor = 0,
            KeyKind::End => self.cursor = self.visible().len().saturating_sub(1),
            KeyKind::PageUp => self.cursor = self.cursor.saturating_sub(8),
            KeyKind::PageDown => {
                self.cursor = (self.cursor + 8).min(self.visible().len().saturating_sub(1))
            }
            KeyKind::Tab => {
                self.agent = self.agent.next();
                self.cursor = 0;
            }
            KeyKind::Right => self.select_all_visible(),
            KeyKind::Left => self.selected.clear(),
            // Once there is search text a space belongs to it ("flaky test"); Right then selects
            // every match and Enter imports the focused row.
            KeyKind::Char(' ') if !self.query.is_empty() => {
                self.query.push(' ');
                self.cursor = 0;
            }
            KeyKind::Char(' ') => {
                self.toggle_focused();
                self.cursor = (self.cursor + 1).min(self.visible().len().saturating_sub(1));
            }
            KeyKind::Char(c) if !c.is_control() => {
                self.query.push(c);
                self.cursor = 0;
            }
            KeyKind::Backspace => {
                self.query.pop();
                self.clamp();
            }
            KeyKind::Enter => {
                let mut picks: Vec<(String, String)> = self
                    .rows
                    .iter()
                    .filter(|r| self.selected.contains(&r.key()))
                    .map(|r| (r.agent.clone(), r.id.clone()))
                    .collect();
                if picks.is_empty() {
                    if let Some(r) = self.visible().get(self.cursor).filter(|r| !r.imported) {
                        picks.push((r.agent.clone(), r.id.clone()));
                    }
                }
                if picks.is_empty() {
                    self.notify(Tone::Warn, "Nothing to import here.");
                } else {
                    self.importing = true;
                    return ImportAction::Import(picks);
                }
            }
            _ => {}
        }
        ImportAction::None
    }
}
