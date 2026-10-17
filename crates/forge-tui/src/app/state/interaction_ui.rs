//! UI state and entry points owned by the composer, popups and decision blocks.

use std::cell::Cell;

use crate::app::{App, KeyKind};
use crate::ConfirmOutcome;

/// How many recently used commands the palette lists first on an empty query.
const RECENT_COMMANDS: usize = 5;

/// The three outcomes of a tool permission, in the order the block lists them.
pub(crate) const PERMISSION_CHOICES: [ConfirmOutcome; 3] = [
    ConfirmOutcome::Allow,
    ConfirmOutcome::AlwaysAllow,
    ConfirmOutcome::Deny,
];

/// A tool permission waiting for the user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PendingPermission {
    pub tool: String,
    /// `SideEffect` debug name (`Write`, `Shell`, `Network`, ...).
    pub side_effect: String,
    /// Index into [`PERMISSION_CHOICES`] that Enter confirms. Starts on "Allow once", so Enter
    /// keeps the meaning it always had.
    pub focus: usize,
}

/// State only the interaction surfaces read or write.
#[derive(Debug, Clone, Default)]
pub(crate) struct InteractionUi {
    pub permission: Option<PendingPermission>,
    /// Most recently run commands, newest first.
    pub recent_commands: Vec<String>,
    /// The newest call of each tool, so a permission prompt can show what it is about even where
    /// there are no tool cards (inline mode).
    pub recent_calls: Vec<(String, String)>,
    /// Size of the last full frame, so a decision block can budget its height against the screen.
    pub screen: Cell<(u16, u16)>,
}

impl InteractionUi {
    pub fn note_tool_call(&mut self, name: &str, args: &str) {
        self.recent_calls.retain(|(n, _)| n != name);
        self.recent_calls.push((name.to_string(), args.to_string()));
        if self.recent_calls.len() > 8 {
            self.recent_calls.remove(0);
        }
    }

    pub fn call_args(&self, name: &str) -> Option<&str> {
        self.recent_calls
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, a)| a.as_str())
    }

    pub fn note_screen(&self, width: u16, height: u16) {
        self.screen.set((width, height));
    }

    pub fn screen_size(&self) -> (u16, u16) {
        match self.screen.get() {
            (0, _) | (_, 0) => (80, 24),
            size => size,
        }
    }
}

impl App {
    /// Map a key pressed while a tool permission prompt is open to its outcome. `None` means the
    /// key moved the focus or is not a choice, and the prompt stays open. `y`, `a`, `n`, Enter and
    /// Esc keep their old meanings; digits pick directly; arrows move the focus and Enter confirms
    /// the focused choice (the first, "Allow once", until it is moved).
    pub fn permission_key(&mut self, key: KeyKind) -> Option<ConfirmOutcome> {
        let focus = self
            .interaction_ui
            .permission
            .as_ref()
            .map_or(0, |p| p.focus);
        let move_focus = |app: &mut App, to: usize| {
            if let Some(p) = app.interaction_ui.permission.as_mut() {
                p.focus = to.min(PERMISSION_CHOICES.len() - 1);
            }
        };
        match key {
            KeyKind::Char('a') | KeyKind::Char('A') | KeyKind::Char('2') => {
                Some(ConfirmOutcome::AlwaysAllow)
            }
            KeyKind::Char('y') | KeyKind::Char('Y') | KeyKind::Char('1') => {
                Some(ConfirmOutcome::Allow)
            }
            KeyKind::Char('n')
            | KeyKind::Char('N')
            | KeyKind::Char('3')
            | KeyKind::Esc
            | KeyKind::Interrupt => Some(ConfirmOutcome::Deny),
            KeyKind::Enter => Some(PERMISSION_CHOICES[focus.min(PERMISSION_CHOICES.len() - 1)]),
            KeyKind::Up | KeyKind::Char('k') | KeyKind::CycleTemper => {
                move_focus(self, focus.saturating_sub(1));
                None
            }
            KeyKind::Down | KeyKind::Char('j') | KeyKind::Tab => {
                move_focus(self, focus + 1);
                None
            }
            KeyKind::Home => {
                move_focus(self, 0);
                None
            }
            KeyKind::End => {
                move_focus(self, PERMISSION_CHOICES.len() - 1);
                None
            }
            _ => None,
        }
    }

    /// Open the permission prompt for `tool`; `side_effect` is its `SideEffect` debug name.
    pub fn open_permission(&mut self, tool: &str, side_effect: &str) {
        self.prompt = Some(format!("allow {tool} ({side_effect}) [y/n/a=always]"));
        self.interaction_ui.permission = Some(PendingPermission {
            tool: tool.to_string(),
            side_effect: side_effect.to_string(),
            focus: 0,
        });
    }

    /// Enter or Tab on the open `@path` picker. A file replaces the `@token` with `@path ` and
    /// closes the picker; a directory replaces it with `@dir/` and stays open, browsing inside.
    pub fn accept_at_path(&mut self) {
        let Some(path) = self.at_picker.selected_path() else {
            self.at_picker.close();
            return;
        };
        let is_dir = path.ends_with('/');
        let text = if is_dir {
            format!("@{path}")
        } else {
            format!("@{path} ")
        };
        let cursor = self.input_cursor.min(self.input.len());
        match crate::at_token_at(&self.input, cursor) {
            Some(tok) => self.input.replace_range(tok.start..tok.end, &text),
            None => self.input = text,
        }
        self.input_cursor = self.input.len();
        if is_dir {
            self.at_picker.query = path;
            self.at_picker.selected = 0;
        } else {
            self.at_picker.close();
        }
    }

    /// Open the Help takeover on `tab`; a command chosen there lands in the composer.
    pub fn open_help(&mut self, tui: &mut crate::Tui, tab: crate::HelpTab) {
        let bindings = self.keybinds.clone();
        if let Ok(Some(text)) = tui.run_fullscreen(|| crate::help::run_help_pick(&bindings, tab)) {
            let at = self.input_cursor.min(self.input.len());
            self.input.insert_str(at, &text);
            self.input_cursor = at + text.len();
        }
    }

    /// Remember that `line` (a `/command ...`) was run, for the palette's recents.
    pub fn note_command_used(&mut self, line: &str) {
        let Some(rest) = line.trim_start().strip_prefix('/') else {
            return;
        };
        let name = rest.split_whitespace().next().unwrap_or("");
        if name.is_empty() || name.starts_with('/') {
            return;
        }
        let recent = &mut self.interaction_ui.recent_commands;
        recent.retain(|n| n != name);
        recent.insert(0, name.to_string());
        recent.truncate(RECENT_COMMANDS);
        self.palette.recents = recent.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_prompt() -> App {
        let mut app = App::default();
        app.open_permission("write_file", "Write");
        app
    }

    #[test]
    fn legacy_keys_keep_their_outcomes() {
        let mut app = app_with_prompt();
        for (key, want) in [
            (KeyKind::Char('y'), ConfirmOutcome::Allow),
            (KeyKind::Char('Y'), ConfirmOutcome::Allow),
            (KeyKind::Enter, ConfirmOutcome::Allow),
            (KeyKind::Char('a'), ConfirmOutcome::AlwaysAllow),
            (KeyKind::Char('A'), ConfirmOutcome::AlwaysAllow),
            (KeyKind::Char('n'), ConfirmOutcome::Deny),
            (KeyKind::Esc, ConfirmOutcome::Deny),
            (KeyKind::Interrupt, ConfirmOutcome::Deny),
            (KeyKind::Char('1'), ConfirmOutcome::Allow),
            (KeyKind::Char('2'), ConfirmOutcome::AlwaysAllow),
            (KeyKind::Char('3'), ConfirmOutcome::Deny),
        ] {
            assert_eq!(app.permission_key(key), Some(want), "{key:?}");
        }
    }

    #[test]
    fn keys_that_are_not_a_choice_are_ignored_instead_of_denying() {
        let mut app = app_with_prompt();
        for key in [
            KeyKind::Char('x'),
            KeyKind::Left,
            KeyKind::Right,
            KeyKind::PageUp,
            KeyKind::Backspace,
            KeyKind::Char('4'),
        ] {
            assert_eq!(app.permission_key(key), None, "{key:?}");
        }
    }

    #[test]
    fn arrows_move_the_focus_and_enter_confirms_the_focused_choice() {
        let mut app = app_with_prompt();
        assert_eq!(app.permission_key(KeyKind::Down), None);
        assert_eq!(
            app.permission_key(KeyKind::Enter),
            Some(ConfirmOutcome::AlwaysAllow)
        );
        assert_eq!(app.permission_key(KeyKind::Down), None);
        assert_eq!(
            app.permission_key(KeyKind::Down),
            None,
            "clamps at the last choice"
        );
        assert_eq!(
            app.permission_key(KeyKind::Enter),
            Some(ConfirmOutcome::Deny)
        );
        assert_eq!(app.permission_key(KeyKind::Up), None);
        assert_eq!(app.permission_key(KeyKind::Home), None);
        assert_eq!(
            app.permission_key(KeyKind::Enter),
            Some(ConfirmOutcome::Allow)
        );
    }

    #[test]
    fn a_new_prompt_starts_on_allow_once() {
        let mut app = app_with_prompt();
        app.permission_key(KeyKind::End);
        app.open_permission("shell", "Shell");
        assert_eq!(
            app.permission_key(KeyKind::Enter),
            Some(ConfirmOutcome::Allow)
        );
    }

    #[test]
    fn the_prompt_string_is_unchanged() {
        let app = app_with_prompt();
        assert_eq!(
            app.prompt.as_deref(),
            Some("allow write_file (Write) [y/n/a=always]")
        );
    }

    #[test]
    fn recents_are_newest_first_deduped_and_capped() {
        let mut app = App::default();
        for line in [
            "/a",
            "/b x y",
            "/a",
            "//literal",
            "hello",
            "/c",
            "/d",
            "/e",
            "/f",
        ] {
            app.note_command_used(line);
        }
        assert_eq!(
            app.interaction_ui.recent_commands,
            ["f", "e", "d", "c", "a"]
        );
    }
}
