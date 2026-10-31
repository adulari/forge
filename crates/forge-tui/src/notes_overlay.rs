//! `/notes`: the project's to-do list and scratchpad, as a keyboard overlay. The same rows
//! `forge notes` prints and the desktop, web and mobile Notes surface edit.
//!
//! Data and key handling only. The binary loads a [`NotesSnapshot`] from the store and performs
//! what a [`NotesAction`] asks for, then hands back a fresh snapshot; rendering is in [`render`].

use crate::app::KeyKind;

mod render;
pub(crate) use render::render_notes_overlay;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoView {
    pub id: String,
    pub text: String,
    pub done: bool,
    /// Short id of the session started from this item.
    pub thread: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotesSnapshot {
    pub project: String,
    pub body: String,
    pub todos: Vec<TodoView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Browse,
    /// Typing a new to-do.
    Add,
    /// Rewording the to-do with this id.
    Edit(String),
    /// Editing the scratchpad.
    Pad,
    /// Waiting for y/n on deleting the to-do with this id.
    ConfirmDelete(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub tone: Tone,
    pub text: String,
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotesAction {
    None,
    Close,
    Reload,
    Add(String),
    Edit {
        id: String,
        text: String,
    },
    Toggle {
        id: String,
        done: bool,
    },
    Delete(String),
    SavePad(String),
    ClearDone,
    /// Start a daemon session in the project with this item as its first prompt.
    StartThread {
        id: String,
        text: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct NotesOverlay {
    pub open: bool,
    pub loaded: bool,
    pub data: NotesSnapshot,
    pub cursor: usize,
    pub mode: Mode,
    /// The line being typed in `Add` / `Edit`, or the scratchpad text in `Pad`.
    pub input: String,
    pub notice: Option<Notice>,
    reload: bool,
}

impl NotesOverlay {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            reload: true,
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
        self.mode = Mode::Browse;
        self.input.clear();
    }

    /// Whether the binary should load a snapshot now; clears the request.
    pub fn take_reload(&mut self) -> bool {
        std::mem::take(&mut self.reload)
    }

    pub fn apply(&mut self, snapshot: NotesSnapshot) {
        self.data = snapshot;
        self.loaded = true;
        self.clamp();
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
        });
    }

    pub fn focused(&self) -> Option<&TodoView> {
        self.data.todos.get(self.cursor)
    }

    pub fn open_count(&self) -> usize {
        self.data.todos.iter().filter(|t| !t.done).count()
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.data.todos.len().saturating_sub(1));
    }

    /// True while a text field owns the keyboard, so `q` types a letter instead of closing.
    pub fn typing(&self) -> bool {
        matches!(self.mode, Mode::Add | Mode::Edit(_) | Mode::Pad)
    }

    pub fn key(&mut self, key: KeyKind) -> NotesAction {
        self.notice = None;
        match self.mode.clone() {
            Mode::Browse => self.browse_key(key),
            Mode::Add => self.line_key(key, None),
            Mode::Edit(id) => self.line_key(key, Some(id)),
            Mode::Pad => self.pad_key(key),
            Mode::ConfirmDelete(id) => self.confirm_key(key, id),
        }
    }

    fn browse_key(&mut self, key: KeyKind) -> NotesAction {
        let last = self.data.todos.len().saturating_sub(1);
        match key {
            KeyKind::Esc | KeyKind::Char('q') | KeyKind::Interrupt => {
                self.close();
                NotesAction::Close
            }
            KeyKind::Up | KeyKind::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                NotesAction::None
            }
            KeyKind::Down | KeyKind::Char('j') => {
                self.cursor = (self.cursor + 1).min(last);
                NotesAction::None
            }
            KeyKind::Home => {
                self.cursor = 0;
                NotesAction::None
            }
            KeyKind::End => {
                self.cursor = last;
                NotesAction::None
            }
            KeyKind::PageUp => {
                self.cursor = self.cursor.saturating_sub(5);
                NotesAction::None
            }
            KeyKind::PageDown => {
                self.cursor = (self.cursor + 5).min(last);
                NotesAction::None
            }
            KeyKind::Char('a') | KeyKind::Char('n') => {
                self.mode = Mode::Add;
                self.input.clear();
                NotesAction::None
            }
            KeyKind::Char('e') => {
                if let Some(todo) = self.focused().cloned() {
                    self.input = todo.text;
                    self.mode = Mode::Edit(todo.id);
                }
                NotesAction::None
            }
            KeyKind::Char(' ') | KeyKind::Char('x') => match self.focused() {
                Some(todo) => NotesAction::Toggle {
                    id: todo.id.clone(),
                    done: !todo.done,
                },
                None => NotesAction::None,
            },
            // Enter starts a thread, or says which one already exists; `t` starts another anyway.
            KeyKind::Enter | KeyKind::Char('t') => {
                let again = key == KeyKind::Char('t');
                match self.focused().cloned() {
                    Some(todo) if todo.done => {
                        self.notify(Tone::Warn, "That one is done. Untick it to start a thread.");
                        NotesAction::None
                    }
                    Some(todo) if todo.thread.is_some() && !again => {
                        let id = todo.thread.unwrap_or_default();
                        self.notify(
                            Tone::Ok,
                            format!("Thread {id} already started. Open it in the app, or press t for another."),
                        );
                        NotesAction::None
                    }
                    Some(todo) => NotesAction::StartThread {
                        id: todo.id,
                        text: todo.text,
                    },
                    None => NotesAction::None,
                }
            }
            KeyKind::Char('d') | KeyKind::DeleteForward => {
                if let Some(todo) = self.focused() {
                    self.mode = Mode::ConfirmDelete(todo.id.clone());
                }
                NotesAction::None
            }
            KeyKind::Char('c') => NotesAction::ClearDone,
            KeyKind::Char('p') => {
                self.input = self.data.body.clone();
                self.mode = Mode::Pad;
                NotesAction::None
            }
            KeyKind::Char('r') => {
                self.reload = true;
                NotesAction::Reload
            }
            _ => NotesAction::None,
        }
    }

    fn confirm_key(&mut self, key: KeyKind, id: String) -> NotesAction {
        match key {
            KeyKind::Char('y') | KeyKind::Enter => {
                self.mode = Mode::Browse;
                NotesAction::Delete(id)
            }
            _ => {
                self.mode = Mode::Browse;
                NotesAction::None
            }
        }
    }

    /// One-line input for a new or edited to-do.
    fn line_key(&mut self, key: KeyKind, editing: Option<String>) -> NotesAction {
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                self.mode = Mode::Browse;
                self.input.clear();
                NotesAction::None
            }
            KeyKind::Enter => {
                let text = self.input.trim().to_string();
                self.mode = Mode::Browse;
                self.input.clear();
                if text.is_empty() {
                    return NotesAction::None;
                }
                match editing {
                    Some(id) => NotesAction::Edit { id, text },
                    None => NotesAction::Add(text),
                }
            }
            KeyKind::Backspace => {
                self.input.pop();
                NotesAction::None
            }
            KeyKind::KillLineBack | KeyKind::DeleteWordBack => {
                self.input.clear();
                NotesAction::None
            }
            KeyKind::Char(c) => {
                self.input.push(c);
                NotesAction::None
            }
            _ => NotesAction::None,
        }
    }

    /// The scratchpad editor: Esc keeps the text, Ctrl-C throws the edit away.
    fn pad_key(&mut self, key: KeyKind) -> NotesAction {
        match key {
            KeyKind::Esc => {
                let body = std::mem::take(&mut self.input);
                self.mode = Mode::Browse;
                if body == self.data.body {
                    NotesAction::None
                } else {
                    NotesAction::SavePad(body)
                }
            }
            KeyKind::Interrupt => {
                self.input.clear();
                self.mode = Mode::Browse;
                NotesAction::None
            }
            KeyKind::Enter | KeyKind::InsertNewline => {
                self.input.push('\n');
                NotesAction::None
            }
            KeyKind::Backspace => {
                self.input.pop();
                NotesAction::None
            }
            KeyKind::Char(c) => {
                self.input.push(c);
                NotesAction::None
            }
            _ => NotesAction::None,
        }
    }
}
