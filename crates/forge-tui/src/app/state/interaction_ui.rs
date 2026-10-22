//! UI state and entry points owned by the composer, popups and decision blocks.

use std::cell::Cell;

use crate::app::{App, KeyKind};
use crate::{ConfirmOutcome, ConfirmReply};

/// How many recently used commands the palette lists first on an empty query.
const RECENT_COMMANDS: usize = 5;

/// The outcomes of a tool permission, in the order the block lists them. "This session" comes
/// last so the first three keep their digits: `3` still denies.
pub(crate) const PERMISSION_CHOICES: [ConfirmOutcome; 4] = [
    ConfirmOutcome::Allow,
    ConfirmOutcome::AlwaysAllow,
    ConfirmOutcome::Deny,
    ConfirmOutcome::AllowSession,
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
    /// The note being typed (Tab opens it). It goes to the model with whichever choice Enter
    /// confirms.
    pub note: Option<String>,
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
    /// Map a key pressed while a tool permission prompt is open to its answer. `None` means the
    /// key moved the focus, edited the note or is not a choice, and the prompt stays open. `y`,
    /// `a`, `n`, Enter and Esc keep their old meanings and `s` allows for this session; digits
    /// pick directly; arrows move the focus and Enter confirms the focused choice (the first,
    /// "Allow once", until it is moved). Tab opens a note that goes to the model with the answer.
    pub fn permission_key(&mut self, key: KeyKind) -> Option<ConfirmReply> {
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
        if let Some(note) = self.permission_note().map(str::to_string) {
            return self.permission_note_key(key, focus, note);
        }
        let outcome = match key {
            KeyKind::Char('a') | KeyKind::Char('A') | KeyKind::Char('2') => {
                ConfirmOutcome::AlwaysAllow
            }
            KeyKind::Char('y') | KeyKind::Char('Y') | KeyKind::Char('1') => ConfirmOutcome::Allow,
            KeyKind::Char('s') | KeyKind::Char('S') | KeyKind::Char('4') => {
                ConfirmOutcome::AllowSession
            }
            KeyKind::Char('n')
            | KeyKind::Char('N')
            | KeyKind::Char('3')
            | KeyKind::Esc
            | KeyKind::Interrupt => ConfirmOutcome::Deny,
            KeyKind::Enter => PERMISSION_CHOICES[focus.min(PERMISSION_CHOICES.len() - 1)],
            KeyKind::Tab => {
                if let Some(p) = self.interaction_ui.permission.as_mut() {
                    p.note = Some(String::new());
                }
                return None;
            }
            KeyKind::Up | KeyKind::Char('k') | KeyKind::CycleTemper => {
                move_focus(self, focus.saturating_sub(1));
                return None;
            }
            KeyKind::Down | KeyKind::Char('j') => {
                move_focus(self, focus + 1);
                return None;
            }
            KeyKind::Home => {
                move_focus(self, 0);
                return None;
            }
            KeyKind::End => {
                move_focus(self, PERMISSION_CHOICES.len() - 1);
                return None;
            }
            _ => return None,
        };
        Some(outcome.into())
    }

    /// The note being typed at the open permission prompt, `None` when no note field is open.
    pub fn permission_note(&self) -> Option<&str> {
        self.interaction_ui
            .permission
            .as_ref()
            .and_then(|p| p.note.as_deref())
    }

    /// A key while the note field is open: printable keys type, arrows still move the focus,
    /// Enter answers with the focused choice and the note, Esc or Tab closes the field (the
    /// prompt stays open), Ctrl-C denies.
    fn permission_note_key(
        &mut self,
        key: KeyKind,
        focus: usize,
        mut note: String,
    ) -> Option<ConfirmReply> {
        let last = PERMISSION_CHOICES.len() - 1;
        let (focus, note) = match key {
            KeyKind::Enter => {
                let choice = PERMISSION_CHOICES[focus.min(last)];
                return Some(ConfirmReply::with_note(choice, &note));
            }
            KeyKind::Interrupt => return Some(ConfirmOutcome::Deny.into()),
            KeyKind::Esc | KeyKind::Tab => (focus, None),
            KeyKind::Char(c) => {
                note.push(c);
                (focus, Some(note))
            }
            KeyKind::Backspace => {
                note.pop();
                (focus, Some(note))
            }
            KeyKind::Up => (focus.saturating_sub(1), Some(note)),
            KeyKind::Down => ((focus + 1).min(last), Some(note)),
            _ => (focus, Some(note)),
        };
        if let Some(p) = self.interaction_ui.permission.as_mut() {
            p.focus = focus;
            p.note = note;
        }
        None
    }

    /// Say what an answer did beyond this one call: a standing allow and where it lives, and that
    /// a note goes to the model. `mcp`: the tool's standing allows are in-memory only.
    pub fn note_permission_answer(&mut self, tool: &str, answer: &ConfirmReply, mcp: bool) {
        match answer.outcome {
            ConfirmOutcome::AlwaysAllow => {
                let scope = if mcp {
                    "for this session"
                } else {
                    "saved to .forge/config.toml"
                };
                self.note(&format!("✓ {tool} always allowed — {scope}"));
            }
            ConfirmOutcome::AllowSession => {
                self.note(&format!("✓ {tool} allowed for this session — not saved"));
            }
            ConfirmOutcome::Allow | ConfirmOutcome::Deny => {}
        }
        if answer.note.is_some() {
            self.note("✎ your note goes to the model with the tool result");
        }
    }

    /// Open the permission prompt for `tool`; `side_effect` is its `SideEffect` debug name.
    pub fn open_permission(&mut self, tool: &str, side_effect: &str) {
        self.prompt = Some(format!("allow {tool} ({side_effect}) [y/n/a=always]"));
        self.interaction_ui.permission = Some(PendingPermission {
            tool: tool.to_string(),
            side_effect: side_effect.to_string(),
            focus: 0,
            note: None,
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

    fn outcome(app: &mut App, key: KeyKind) -> Option<ConfirmOutcome> {
        app.permission_key(key).map(|reply| reply.outcome)
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
            (KeyKind::Char('s'), ConfirmOutcome::AllowSession),
            (KeyKind::Char('4'), ConfirmOutcome::AllowSession),
        ] {
            assert_eq!(outcome(&mut app, key), Some(want), "{key:?}");
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
            KeyKind::Char('5'),
        ] {
            assert_eq!(app.permission_key(key), None, "{key:?}");
        }
    }

    #[test]
    fn arrows_move_the_focus_and_enter_confirms_the_focused_choice() {
        let mut app = app_with_prompt();
        assert_eq!(app.permission_key(KeyKind::Down), None);
        assert_eq!(
            outcome(&mut app, KeyKind::Enter),
            Some(ConfirmOutcome::AlwaysAllow)
        );
        assert_eq!(app.permission_key(KeyKind::Down), None);
        assert_eq!(
            outcome(&mut app, KeyKind::Enter),
            Some(ConfirmOutcome::Deny)
        );
        assert_eq!(app.permission_key(KeyKind::Down), None);
        assert_eq!(
            app.permission_key(KeyKind::Down),
            None,
            "clamps at the last choice"
        );
        assert_eq!(
            outcome(&mut app, KeyKind::Enter),
            Some(ConfirmOutcome::AllowSession)
        );
        assert_eq!(app.permission_key(KeyKind::Up), None);
        assert_eq!(app.permission_key(KeyKind::Home), None);
        assert_eq!(
            outcome(&mut app, KeyKind::Enter),
            Some(ConfirmOutcome::Allow)
        );
    }

    #[test]
    fn a_new_prompt_starts_on_allow_once() {
        let mut app = app_with_prompt();
        app.permission_key(KeyKind::End);
        app.open_permission("shell", "Shell");
        assert_eq!(
            outcome(&mut app, KeyKind::Enter),
            Some(ConfirmOutcome::Allow)
        );
    }

    #[test]
    fn tab_opens_a_note_that_rides_with_the_focused_choice() {
        let mut app = app_with_prompt();
        assert_eq!(app.permission_key(KeyKind::Tab), None);
        assert_eq!(app.permission_note(), Some(""));
        // In the note, choice letters and digits are text and arrows still move the focus.
        for c in "use rg 3".chars() {
            assert_eq!(app.permission_key(KeyKind::Char(c)), None);
        }
        app.permission_key(KeyKind::Backspace);
        app.permission_key(KeyKind::Backspace);
        app.permission_key(KeyKind::Down);
        app.permission_key(KeyKind::Down);
        assert_eq!(
            app.permission_key(KeyKind::Enter),
            Some(ConfirmReply::with_note(ConfirmOutcome::Deny, "use rg"))
        );
    }

    #[test]
    fn esc_closes_the_note_without_answering_and_ctrl_c_still_denies() {
        let mut app = app_with_prompt();
        app.permission_key(KeyKind::Tab);
        app.permission_key(KeyKind::Char('x'));
        assert_eq!(app.permission_key(KeyKind::Esc), None);
        assert_eq!(app.permission_note(), None, "the note field closed");
        assert!(app.prompt.is_some(), "the prompt stays open");
        app.permission_key(KeyKind::Tab);
        assert_eq!(
            app.permission_key(KeyKind::Interrupt),
            Some(ConfirmOutcome::Deny.into())
        );
    }

    #[test]
    fn a_blank_note_is_no_note() {
        let mut app = app_with_prompt();
        app.permission_key(KeyKind::Tab);
        app.permission_key(KeyKind::Char(' '));
        assert_eq!(
            app.permission_key(KeyKind::Enter),
            Some(ConfirmOutcome::Allow.into())
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
