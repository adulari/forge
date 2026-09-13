//! The plan-and-dispatch form: one prompt plus the four choices that shape the run (worktrees,
//! what sessions may do without asking, how many run at once, how many the split may have).
//!
//! State and input only; `form_render.rs` draws it. Every choice has a sensible default so the
//! common path is: `D`, type, Enter.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::dispatch::list_words;
use super::model::project_name;
use super::state::{BoardApp, Composer, ComposerMode, Focus, ToastLevel};
use super::BoardAction;

/// Mirrors `forge_core::dispatch::{MAX_RUNNING_HARD, DEFAULT_MAX_RUNNING}`.
pub const MAX_RUNNING: usize = 8;
pub const DEFAULT_RUNNING: usize = 4;
/// Mirrors `forge_core::dispatch::{MAX_ITEMS_HARD, DEFAULT_MAX_ITEMS}`.
pub const MAX_ITEMS: usize = 12;
pub const DEFAULT_ITEMS: usize = 8;
/// Ticks the prompt border flashes after an empty submit.
pub const SHAKE_TICKS: u64 = 6;

/// `(wire key, what the user reads, what it means)` for the worker permission mode.
pub const MODES: [(&str, &str, &str); 3] = [
    (
        "default",
        "ask before every change",
        "Every file edit and shell command waits for you on the board.",
    ),
    (
        "accept-edits",
        "edit files, ask before shell",
        "Sessions edit files freely; shell commands wait for you on the board.",
    ),
    (
        "bypass",
        "run without asking",
        "Sessions edit files and run commands without asking. Use with care.",
    ),
];
const DEFAULT_MODE: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Prompt,
    Worktree,
    Mode,
    Running,
    Items,
}

impl FormField {
    pub const ALL: [FormField; 5] = [
        FormField::Prompt,
        FormField::Worktree,
        FormField::Mode,
        FormField::Running,
        FormField::Items,
    ];

    fn step(self, delta: i32) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0) as i32;
        Self::ALL[(i + delta).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

/// A click inside the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormHit {
    Field(FormField),
    Worktree(bool),
    /// `‹` (-1) or `›` (+1) of a cycling field.
    Step(FormField, i32),
    Start,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DispatchForm {
    pub cwd: String,
    pub text: Composer,
    pub field: FormField,
    pub worktree: bool,
    /// Index into [`MODES`].
    pub mode: usize,
    pub max_running: usize,
    pub max_items: usize,
    /// The tick of the last rejected submit, for the border flash.
    pub shake_at: Option<u64>,
}

impl DispatchForm {
    pub fn new(cwd: String) -> Self {
        Self {
            cwd,
            text: Composer {
                mode: ComposerMode::Prompt,
                target: String::new(),
                text: String::new(),
                cursor: 0,
            },
            field: FormField::Prompt,
            worktree: true,
            mode: DEFAULT_MODE,
            max_running: DEFAULT_RUNNING,
            max_items: DEFAULT_ITEMS,
            shake_at: None,
        }
    }

    /// ←/→/Space on the focused field.
    pub fn change(&mut self, delta: i32) {
        match self.field {
            FormField::Prompt => {}
            FormField::Worktree => self.worktree = !self.worktree,
            FormField::Mode => {
                self.mode = (self.mode as i32 + delta).rem_euclid(MODES.len() as i32) as usize;
            }
            FormField::Running => {
                self.max_running = step_clamped(self.max_running, delta, MAX_RUNNING);
            }
            FormField::Items => self.max_items = step_clamped(self.max_items, delta, MAX_ITEMS),
        }
    }

    /// The one line under the focused field that says what the choice means.
    pub fn explanation(&self) -> &'static str {
        match self.field {
            FormField::Prompt => "Ctrl+J adds a line · paste works · Enter starts",
            FormField::Worktree if self.worktree => {
                "Each session gets its own branch; merge the results back from the board."
            }
            FormField::Worktree => {
                "Every session edits the same directory — only safe when the parts don't overlap."
            }
            FormField::Mode => MODES[self.mode.min(MODES.len() - 1)].2,
            FormField::Running => {
                "How many sessions run at the same time; the rest wait for a slot."
            }
            FormField::Items => "The most sessions the coordinator may split the work into.",
        }
    }

    pub fn mode_key(&self) -> &'static str {
        MODES[self.mode.min(MODES.len() - 1)].0
    }

    pub fn mode_label(&self) -> &'static str {
        MODES[self.mode.min(MODES.len() - 1)].1
    }

    /// The cursor's (line, column) in chars.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before: String = self.text.text.chars().take(self.text.cursor).collect();
        let line = before.matches('\n').count();
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count());
        (line, col)
    }

    /// Move the cursor one logical line up or down, keeping its column where the line allows.
    /// `false` at the first/last line, so the caller can move focus instead.
    fn move_line(&mut self, up: bool) -> bool {
        let lines: Vec<&str> = self.text.text.split('\n').collect();
        let (line, col) = self.cursor_line_col();
        let target = if up {
            match line.checked_sub(1) {
                Some(t) => t,
                None => return false,
            }
        } else if line + 1 < lines.len() {
            line + 1
        } else {
            return false;
        };
        let start: usize = lines[..target].iter().map(|l| l.chars().count() + 1).sum();
        self.text.cursor = start + col.min(lines[target].chars().count());
        true
    }
}

fn step_clamped(value: usize, delta: i32, max: usize) -> usize {
    (value as i32 + delta).clamp(1, max as i32) as usize
}

impl BoardApp {
    pub(crate) fn open_form(&mut self) {
        let cwd = self.new_session_cwd();
        self.dispatch.form = Some(DispatchForm::new(cwd));
        self.focus = Focus::Form;
    }

    pub(crate) fn cancel_form(&mut self) {
        self.dispatch.form = None;
        self.focus = if self.detail_open {
            Focus::Detail
        } else {
            Focus::Board
        };
    }

    pub(crate) fn form_shaking(&self) -> bool {
        self.dispatch
            .form
            .as_ref()
            .and_then(|f| f.shake_at)
            .is_some_and(|t| self.tick.saturating_sub(t) < SHAKE_TICKS)
    }

    /// Enter: start the dispatch, or refuse an empty prompt without losing the form.
    pub(crate) fn submit_form(&mut self) -> Vec<BoardAction> {
        let tick = self.tick;
        let Some(form) = self.dispatch.form.as_mut() else {
            return Vec::new();
        };
        let prompt = form.text.text.trim().to_string();
        if prompt.is_empty() {
            form.shake_at = Some(tick);
            form.field = FormField::Prompt;
            self.toast(ToastLevel::Info, "describe the work first");
            return Vec::new();
        }
        let Some(form) = self.dispatch.form.take() else {
            return Vec::new();
        };
        self.focus = if self.detail_open {
            Focus::Detail
        } else {
            Focus::Board
        };
        self.dispatch.started_cwd = Some(form.cwd.clone());
        self.toast(
            ToastLevel::Info,
            format!("planning a split for {}…", project_name(&form.cwd)),
        );
        vec![BoardAction::StartDispatch {
            cwd: form.cwd.clone(),
            prompt,
            worktree: form.worktree,
            mode: form.mode_key().to_string(),
            max_running: form.max_running,
            max_items: form.max_items,
        }]
    }

    pub(crate) fn form_click(&mut self, hit: FormHit) -> Vec<BoardAction> {
        let Some(form) = self.dispatch.form.as_mut() else {
            return Vec::new();
        };
        match hit {
            FormHit::Field(f) => form.field = f,
            FormHit::Worktree(on) => {
                form.field = FormField::Worktree;
                form.worktree = on;
            }
            FormHit::Step(f, delta) => {
                form.field = f;
                form.change(delta);
            }
            FormHit::Start => return self.submit_form(),
            FormHit::Cancel => self.cancel_form(),
        }
        Vec::new()
    }
}

/// Every key while the form is open.
pub(crate) fn form_key(app: &mut BoardApp, key: KeyEvent) -> Vec<BoardAction> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let newline_mods = key
        .modifiers
        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
    let Some(form) = app.dispatch.form.as_mut() else {
        app.focus = Focus::Board;
        return Vec::new();
    };
    let in_prompt = form.field == FormField::Prompt;
    match key.code {
        KeyCode::Esc => app.cancel_form(),
        KeyCode::Char('j') if ctrl && in_prompt => form.text.insert("\n"),
        KeyCode::Enter if newline_mods && in_prompt => form.text.insert("\n"),
        KeyCode::Enter => return app.submit_form(),
        KeyCode::Tab => form.field = form.field.step(1),
        KeyCode::BackTab => form.field = form.field.step(-1),
        KeyCode::Up => {
            if in_prompt {
                form.move_line(true);
            } else {
                form.field = form.field.step(-1);
            }
        }
        KeyCode::Down => {
            if !(in_prompt && form.move_line(false)) {
                form.field = form.field.step(1);
            }
        }
        _ if in_prompt => edit_prompt(&mut form.text, key, ctrl),
        KeyCode::Left => form.change(-1),
        KeyCode::Right | KeyCode::Char(' ') => form.change(1),
        // Typing on a choice row means the user wants to write: send it to the prompt rather
        // than dropping the keystrokes on a field that cannot take text.
        KeyCode::Char(c) if !ctrl => {
            form.field = FormField::Prompt;
            form.text.end();
            form.text.insert(&c.to_string());
        }
        _ => {}
    }
    Vec::new()
}

fn edit_prompt(text: &mut Composer, key: KeyEvent, ctrl: bool) {
    match key.code {
        KeyCode::Backspace => text.backspace(),
        KeyCode::Left => text.move_left(),
        KeyCode::Right => text.move_right(),
        KeyCode::Home => text.home(),
        KeyCode::End => text.end(),
        KeyCode::Char('u') if ctrl => text.kill_line_back(),
        KeyCode::Char('w') if ctrl => text.delete_word_back(),
        KeyCode::Char(ch) if !ctrl => text.insert(&ch.to_string()),
        _ => {}
    }
}

/// A paste keeps its line breaks in the prompt box (a pasted spec is the common case) and is
/// flattened anywhere else.
pub(crate) fn form_paste(app: &mut BoardApp, pasted: &str) {
    let Some(form) = app.dispatch.form.as_mut() else {
        return;
    };
    form.field = FormField::Prompt;
    form.text
        .insert(&pasted.replace("\r\n", "\n").replace('\r', "\n"));
}

/// `after 1, 2` / `after 1 and 3` — the dependency chip of a checklist row.
pub fn after_words(deps: &[usize]) -> String {
    format!("after {}", list_words(deps))
}
