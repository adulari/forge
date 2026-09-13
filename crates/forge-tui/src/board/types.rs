//! The small value types the board's state is made of: focus, detail tabs, the composer, the
//! confirmation dialog, toasts, connectivity, and the clickable regions render records.

use super::form::FormHit;
use super::model::{project_name, Column};

/// Which surface owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Board,
    Detail,
    Composer,
    Confirm,
    Help,
    Filter,
    /// The plan-and-dispatch form.
    Form,
}

/// The detail pane's sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    /// Only on cards that coordinate or work for a dispatch, and then first.
    Dispatch,
    Overview,
    Tail,
    Tasks,
    Changes,
    Tools,
}

impl DetailTab {
    pub const ALL: [DetailTab; 6] = [
        DetailTab::Dispatch,
        DetailTab::Overview,
        DetailTab::Tail,
        DetailTab::Tasks,
        DetailTab::Changes,
        DetailTab::Tools,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dispatch => "Dispatch",
            Self::Overview => "Overview",
            Self::Tail => "Live",
            Self::Tasks => "Tasks",
            Self::Changes => "Changes",
            Self::Tools => "Tools",
        }
    }

    /// The tabs a card has: the Dispatch tab exists only for dispatch cards.
    pub fn tabs(has_dispatch: bool) -> &'static [DetailTab] {
        if has_dispatch {
            &Self::ALL
        } else {
            &Self::ALL[1..]
        }
    }

    pub fn next_in(self, has_dispatch: bool) -> Self {
        let tabs = Self::tabs(has_dispatch);
        let i = tabs.iter().position(|t| *t == self).unwrap_or(0);
        tabs[(i + 1) % tabs.len()]
    }

    pub fn prev_in(self, has_dispatch: bool) -> Self {
        let tabs = Self::tabs(has_dispatch);
        let i = tabs.iter().position(|t| *t == self).unwrap_or(0);
        tabs[(i + tabs.len() - 1) % tabs.len()]
    }
}

/// What the one-line composer is collecting.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposerMode {
    /// A prompt for the target session (queued if it is busy).
    Prompt,
    /// A steer: delivered at the target's next turn boundary, ahead of the queue.
    Steer,
    /// A `/model <id>` re-pin (empty clears the pin).
    Model,
    /// Free-text answer to the pending question (`seq` is echoed back).
    Answer { seq: u64 },
    /// The first prompt of a brand-new session in `cwd`.
    NewSession { cwd: String, worktree: bool },
    /// Feedback for a proposed split; the coordinator plans again.
    Revise { dispatch_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Composer {
    pub mode: ComposerMode,
    /// The session the text is for (empty for `NewSession`).
    pub target: String,
    pub text: String,
    /// Char index of the cursor.
    pub cursor: usize,
}

impl Composer {
    pub fn label(&self) -> String {
        match &self.mode {
            ComposerMode::Prompt => "prompt".into(),
            ComposerMode::Steer => "steer (jumps the queue)".into(),
            ComposerMode::Model => "/model  — new pin, empty clears".into(),
            ComposerMode::Answer { .. } => "answer".into(),
            ComposerMode::NewSession { cwd, worktree } => format!(
                "new session in {}{}",
                project_name(cwd),
                if *worktree { " (worktree)" } else { "" }
            ),
            ComposerMode::Revise { .. } => "what should change?".into(),
        }
    }

    pub fn insert(&mut self, s: &str) {
        let byte = self.byte_at(self.cursor);
        self.text.insert_str(byte, s);
        self.cursor += s.chars().count();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_at(self.cursor - 1);
        let end = self.byte_at(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete_word_back(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && chars[i - 1] != ' ' {
            i -= 1;
        }
        let start = self.byte_at(i);
        let end = self.byte_at(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor = i;
    }

    pub fn kill_line_back(&mut self) {
        let end = self.byte_at(self.cursor);
        self.text.replace_range(..end, "");
        self.cursor = 0;
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(b, _)| b)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmKind {
    Archive(String),
    /// Cancel a proposed dispatch, or the not-yet-started rest of a running one.
    CancelDispatch(String),
    /// Merge every succeeded worker of this dispatch, in index order.
    MergeFinished(String),
    Merge(String),
    Discard(String),
}

impl ConfirmKind {
    /// Irreversible — drawn in the danger tone instead of the warning tone.
    pub fn is_danger(&self) -> bool {
        matches!(self, Self::Discard(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub kind: ConfirmKind,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub level: ToastLevel,
    pub text: String,
    pub born: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    Connecting,
    Live,
    Reconnecting,
    Offline(String),
}

/// A clickable region recorded during render, resolved on mouse input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    Card(String),
    ColumnHeader(Column),
    DetailTab(DetailTab),
    Button(Button),
    CloseDetail,
    Help,
    ProjectFilter,
    /// The header's `◆ dispatch` chip.
    DispatchChip,
    /// The header's zoom chip (`◆ <title> ✕`).
    ClearZoom,
    /// A row of the Dispatch tab, by position in the item list.
    DispatchRow(usize),
    /// The `[✓]` box of a proposed item, by position.
    DispatchToggle(usize),
    Form(FormHit),
}

/// The action buttons the detail pane draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Attach,
    Prompt,
    Steer,
    Interrupt,
    Allow,
    Deny,
    Answer(usize),
    Model,
    Mode,
    Archive,
    Resume,
    NewSession,
    Copy,
    ToggleTools,
    Approve,
    Revise,
    CancelDispatch,
    MergeAll,
    Merge,
    Discard,
}
