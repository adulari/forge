//! `/queue`: the daemon's merge queue per repository, with hold, release, move-to-front and retry
//! on the focused PR. Data and key handling only; the binary asks the daemon for the list and
//! performs the action, and [`render`] draws the card.

use crate::app::KeyKind;
pub use crate::import_menu::{Notice, Tone};

mod render;
pub(crate) use render::render_queue_menu;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueRow {
    pub session_id: String,
    pub repo: String,
    pub pr: Option<u64>,
    /// 1-based place among the PRs still waiting.
    pub position: Option<usize>,
    /// `head` | `queued` | `checks` | `held` | `skipped` | `merged` | `blocked`
    pub state: String,
    pub reason: String,
    pub pushes: u32,
}

impl QueueRow {
    pub fn label(&self) -> String {
        self.pr
            .map_or_else(|| "PR ?".to_string(), |n| format!("PR #{n}"))
    }

    pub fn state_label(&self) -> &str {
        match self.state.as_str() {
            "head" => "next to land",
            "queued" => "queued",
            "checks" => "in checks",
            "held" => "held",
            "skipped" => "handed back",
            "merged" => "merged",
            "blocked" => "left the queue",
            other => other,
        }
    }

    fn finished(&self) -> bool {
        matches!(self.state.as_str(), "merged" | "blocked")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueOp {
    Hold,
    Release,
    Front,
    Retry,
}

impl QueueOp {
    /// The daemon's verb.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Release => "release",
            Self::Front => "front",
            Self::Retry => "retry",
        }
    }

    /// Whether `row` can take this action, or the reason it cannot.
    pub fn check(self, row: &QueueRow) -> Result<(), String> {
        let label = row.label();
        if row.finished() {
            return Err(format!("{label} has left the queue"));
        }
        match self {
            Self::Hold if row.state == "held" => Err(format!("{label} is already held")),
            Self::Release if row.state != "held" => Err(format!("{label} is not held")),
            Self::Retry if row.state != "skipped" => {
                Err(format!("{label} was not handed back; nothing to retry"))
            }
            Self::Front if row.position == Some(1) => Err(format!("{label} is already next")),
            _ => Ok(()),
        }
    }

    /// What Enter does on a row: the action that fits its state.
    fn default_for(row: &QueueRow) -> Self {
        match row.state.as_str() {
            "held" => Self::Release,
            "skipped" => Self::Retry,
            _ => Self::Hold,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready,
    Failed(String),
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueAction {
    None,
    Close,
    Act { session_id: String, op: QueueOp },
}

#[derive(Debug, Clone)]
pub struct QueueMenu {
    pub open: bool,
    pub load: Load,
    pub rows: Vec<QueueRow>,
    pub cursor: usize,
    pub notice: Option<Notice>,
    pub tick: u32,
    /// An action is on its way to the daemon.
    pub busy: bool,
    refresh: bool,
}

impl Default for QueueMenu {
    fn default() -> Self {
        Self {
            open: false,
            load: Load::Loading,
            rows: Vec::new(),
            cursor: 0,
            notice: None,
            tick: 0,
            busy: false,
            refresh: false,
        }
    }
}

impl QueueMenu {
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

    pub fn apply_list(&mut self, result: Result<Vec<QueueRow>, String>) {
        match result {
            Ok(rows) => {
                let focused = self.rows.get(self.cursor).map(|r| r.session_id.clone());
                self.rows = rows;
                self.load = Load::Ready;
                // Follow the PR the cursor was on when a front or a landing reorders the list.
                if let Some(at) =
                    focused.and_then(|id| self.rows.iter().position(|r| r.session_id == id))
                {
                    self.cursor = at;
                }
                self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
            }
            Err(error) => self.load = Load::Failed(error),
        }
    }

    /// The daemon answered an action: its message, and the queue it left behind.
    pub fn finish_action(&mut self, result: Result<(String, Vec<QueueRow>), String>) {
        self.busy = false;
        match result {
            Ok((message, rows)) => {
                self.apply_list(Ok(rows));
                self.notify(Tone::Ok, message);
            }
            Err(error) => self.notify(Tone::Err, error),
        }
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
        });
    }

    fn act(&mut self, op: QueueOp) -> QueueAction {
        let Some(row) = self.rows.get(self.cursor) else {
            return QueueAction::None;
        };
        if let Err(why) = op.check(row) {
            self.notify(Tone::Warn, why);
            return QueueAction::None;
        }
        self.busy = true;
        QueueAction::Act {
            session_id: row.session_id.clone(),
            op,
        }
    }

    pub fn key(&mut self, key: KeyKind) -> QueueAction {
        self.tick = self.tick.wrapping_add(1);
        if self.busy {
            return QueueAction::None;
        }
        self.notice = None;
        let last = self.rows.len().saturating_sub(1);
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                self.close();
                return QueueAction::Close;
            }
            KeyKind::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyKind::Down => self.cursor = (self.cursor + 1).min(last),
            KeyKind::Home => self.cursor = 0,
            KeyKind::End => self.cursor = last,
            KeyKind::PageUp => self.cursor = self.cursor.saturating_sub(8),
            KeyKind::PageDown => self.cursor = (self.cursor + 8).min(last),
            KeyKind::Char('h') => return self.act(QueueOp::Hold),
            KeyKind::Char('r') => return self.act(QueueOp::Release),
            KeyKind::Char('f') => return self.act(QueueOp::Front),
            KeyKind::Char('t') => return self.act(QueueOp::Retry),
            KeyKind::Char('R') => {
                self.load = Load::Loading;
                self.refresh = true;
            }
            KeyKind::Enter => {
                if let Some(row) = self.rows.get(self.cursor) {
                    return self.act(QueueOp::default_for(row));
                }
            }
            _ => {}
        }
        QueueAction::None
    }
}
