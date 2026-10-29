//! Readline editing on top of the pure [`handle_key`]: a kill ring fed by Ctrl+K / Ctrl+U /
//! Ctrl+W (Ctrl+Y yanks the latest kill, Alt+Y right after cycles to older ones) and an undo stack
//! for Ctrl+_ with Alt+_ to redo. Consecutive kills join one ring entry and a run of typing undoes as one step, the
//! way readline and Claude Code behave.

use super::*;

const RING_MAX: usize = 16;
const UNDO_MAX: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastEdit {
    Typed,
    Erased,
    KilledForward,
    KilledBack,
    Yanked,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct YankSpan {
    start: usize,
    end: usize,
    index: usize,
}

/// The kill ring and undo history of the input line.
#[derive(Debug, Clone, Default)]
pub struct InputEditState {
    ring: Vec<String>,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    yank: Option<YankSpan>,
    last: Option<LastEdit>,
    /// `(len, cursor)` as the last edit left the line. Anything else changed it since (history
    /// recall, a paste), so a following kill or Alt+Y does not chain onto the stale edit.
    left: (usize, usize),
}

impl InputEditState {
    fn chained(&self, input: &str, cursor: usize) -> Option<LastEdit> {
        self.last.filter(|_| self.left == (input.len(), cursor))
    }

    fn push_undo(&mut self, text: String, cursor: usize) {
        self.redo.clear();
        if self.undo.len() == UNDO_MAX {
            self.undo.remove(0);
        }
        self.undo.push((text, cursor));
    }

    fn push_kill(&mut self, killed: String, forward: bool, chained: bool) {
        match self.ring.last_mut() {
            Some(top) if chained && forward => top.push_str(&killed),
            Some(top) if chained => top.insert_str(0, &killed),
            _ => {
                if self.ring.len() == RING_MAX {
                    self.ring.remove(0);
                }
                self.ring.push(killed);
            }
        }
    }

    /// The killed entries, oldest first (for tests and the remote snapshot of nothing — it stays
    /// local to the composer).
    pub fn kill_ring(&self) -> &[String] {
        &self.ring
    }
}

/// What a kill removed: forward kills cut from the old cursor onward, backward kills cut from
/// where the cursor landed up to the old cursor.
fn killed_text(
    before: &str,
    before_cursor: usize,
    after: &str,
    after_cursor: usize,
    forward: bool,
) -> String {
    let (start, end) = if forward {
        let cut = before.len().saturating_sub(after.len());
        (before_cursor, before_cursor + cut)
    } else {
        (after_cursor, before_cursor)
    };
    before.get(start..end).unwrap_or_default().to_string()
}

impl App {
    /// Feed one keystroke to the input line with kill-ring and undo bookkeeping. Every caller that
    /// used `handle_key(&mut app.input, &mut app.input_cursor, key)` for the composer goes through
    /// here, so Ctrl+Y, Alt+Y and Ctrl+_ work wherever typing does.
    pub fn edit_key(&mut self, key: KeyKind) -> InputOutcome {
        match key {
            KeyKind::UndoEdit => {
                self.undo_edit();
                return InputOutcome::Editing;
            }
            KeyKind::RedoEdit => {
                self.redo_edit();
                return InputOutcome::Editing;
            }
            KeyKind::Yank => {
                self.yank();
                return InputOutcome::Editing;
            }
            KeyKind::YankPop => {
                self.yank_pop();
                return InputOutcome::Editing;
            }
            _ => {}
        }
        let chained = self.input_edit.chained(&self.input, self.input_cursor);
        let before = self.input.clone();
        let before_cursor = self.input_cursor;
        let outcome = handle_key(&mut self.input, &mut self.input_cursor, key);
        if let InputOutcome::Submit(_) = outcome {
            self.input_edit.undo.clear();
            self.input_edit.redo.clear();
            self.input_edit.yank = None;
            self.input_edit.last = None;
            return outcome;
        }
        let kind = if self.input == before {
            LastEdit::Other
        } else {
            match key {
                KeyKind::Char(c) if !c.is_whitespace() => LastEdit::Typed,
                KeyKind::Backspace => LastEdit::Erased,
                KeyKind::KillLineForward => LastEdit::KilledForward,
                KeyKind::KillLineBack | KeyKind::DeleteWordBack => LastEdit::KilledBack,
                _ => LastEdit::Other,
            }
        };
        if self.input != before {
            let coalesce = matches!(
                (kind, chained),
                (LastEdit::Typed, Some(LastEdit::Typed))
                    | (LastEdit::Erased, Some(LastEdit::Erased))
            );
            if !coalesce {
                self.input_edit.push_undo(before.clone(), before_cursor);
            }
            if matches!(kind, LastEdit::KilledForward | LastEdit::KilledBack) {
                let forward = kind == LastEdit::KilledForward;
                let joins = chained.is_some_and(|last| {
                    matches!(last, LastEdit::KilledForward | LastEdit::KilledBack)
                });
                let killed = killed_text(
                    &before,
                    before_cursor,
                    &self.input,
                    self.input_cursor,
                    forward,
                );
                self.input_edit.push_kill(killed, forward, joins);
            }
        }
        // A space typed after a word starts a new undo group: the word before it stays one step.
        let kind = match key {
            KeyKind::Char(c) if c.is_whitespace() && self.input != before => LastEdit::Typed,
            _ => kind,
        };
        self.finish_edit(kind);
        outcome
    }

    fn finish_edit(&mut self, kind: LastEdit) {
        self.input_edit.last = Some(kind);
        self.input_edit.left = (self.input.len(), self.input_cursor);
        if kind != LastEdit::Yanked {
            self.input_edit.yank = None;
        }
    }

    /// Ctrl+_: restore the input and cursor as they were before the last edit.
    pub fn undo_edit(&mut self) -> bool {
        let Some((text, cursor)) = self.input_edit.undo.pop() else {
            return false;
        };
        let current = (std::mem::replace(&mut self.input, text), self.input_cursor);
        self.input_edit.redo.push(current);
        self.input_cursor = cursor.min(self.input.len());
        self.finish_edit(LastEdit::Other);
        true
    }

    /// Alt+_: put back the edit the last Ctrl+_ took off. Any new edit forgets what was undone.
    pub fn redo_edit(&mut self) -> bool {
        let Some((text, cursor)) = self.input_edit.redo.pop() else {
            return false;
        };
        let current = (std::mem::replace(&mut self.input, text), self.input_cursor);
        self.input_edit.undo.push(current);
        self.input_cursor = cursor.min(self.input.len());
        self.finish_edit(LastEdit::Other);
        true
    }

    /// Replace the whole draft (e.g. with what came back from `$EDITOR`) as one undoable edit.
    pub fn replace_input(&mut self, text: String) {
        if text == self.input {
            return;
        }
        let before = std::mem::replace(&mut self.input, text);
        self.input_edit.push_undo(before, self.input_cursor);
        self.input_cursor = self.input.len();
        self.finish_edit(LastEdit::Other);
    }

    /// Ctrl+Y: insert the most recent kill at the cursor.
    fn yank(&mut self) {
        let Some(text) = self.input_edit.ring.last().cloned() else {
            return;
        };
        let start = self.input_cursor.min(self.input.len());
        self.input_edit.push_undo(self.input.clone(), start);
        self.input.insert_str(start, &text);
        self.input_cursor = start + text.len();
        self.input_edit.yank = Some(YankSpan {
            start,
            end: self.input_cursor,
            index: self.input_edit.ring.len() - 1,
        });
        self.finish_edit(LastEdit::Yanked);
    }

    /// Alt+Y right after Ctrl+Y (or another Alt+Y): swap the yanked text for the next older kill.
    fn yank_pop(&mut self) {
        let chained = self.input_edit.chained(&self.input, self.input_cursor);
        let (Some(LastEdit::Yanked), Some(span)) = (chained, self.input_edit.yank) else {
            return;
        };
        let ring = &self.input_edit.ring;
        if ring.len() < 2 || span.end > self.input.len() {
            return;
        }
        let index = (span.index + ring.len() - 1) % ring.len();
        let text = ring[index].clone();
        self.input.replace_range(span.start..span.end, &text);
        self.input_cursor = span.start + text.len();
        self.input_edit.yank = Some(YankSpan {
            start: span.start,
            end: self.input_cursor,
            index,
        });
        self.finish_edit(LastEdit::Yanked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.edit_key(KeyKind::Char(c));
        }
    }

    #[test]
    fn ctrl_k_then_ctrl_y_puts_the_text_back() {
        let mut app = App::default();
        typed(&mut app, "hello world");
        app.input_cursor = 5;
        app.edit_key(KeyKind::KillLineForward);
        assert_eq!(app.input, "hello");
        app.edit_key(KeyKind::Home);
        app.edit_key(KeyKind::Yank);
        assert_eq!(app.input, " worldhello");
        assert_eq!(app.input_cursor, 6);
    }

    #[test]
    fn consecutive_kills_join_one_entry_in_reading_order() {
        let mut app = App::default();
        typed(&mut app, "one two three");
        app.edit_key(KeyKind::DeleteWordBack);
        app.edit_key(KeyKind::DeleteWordBack);
        assert_eq!(app.input, "one ");
        assert_eq!(app.input_edit.kill_ring(), ["two three"]);
        app.edit_key(KeyKind::Yank);
        assert_eq!(app.input, "one two three");
    }

    #[test]
    fn a_cursor_move_between_kills_starts_a_new_entry_and_alt_y_cycles() {
        let mut app = App::default();
        typed(&mut app, "alpha beta");
        app.edit_key(KeyKind::DeleteWordBack);
        app.edit_key(KeyKind::Left);
        app.edit_key(KeyKind::KillLineBack);
        assert_eq!(app.input, " ");
        assert_eq!(app.input_edit.kill_ring(), ["beta", "alpha"]);
        app.edit_key(KeyKind::End);
        app.edit_key(KeyKind::Yank);
        assert_eq!(app.input, " alpha");
        app.edit_key(KeyKind::YankPop);
        assert_eq!(app.input, " beta");
        app.edit_key(KeyKind::YankPop);
        assert_eq!(app.input, " alpha", "the ring wraps around");
    }

    #[test]
    fn alt_y_without_a_preceding_yank_does_nothing() {
        let mut app = App::default();
        typed(&mut app, "a b");
        app.edit_key(KeyKind::DeleteWordBack);
        app.edit_key(KeyKind::YankPop);
        assert_eq!(app.input, "a ");
        app.edit_key(KeyKind::Yank);
        app.edit_key(KeyKind::Char('x'));
        app.edit_key(KeyKind::YankPop);
        assert_eq!(app.input, "a bx");
    }

    #[test]
    fn redo_puts_back_what_undo_took_off_until_a_new_edit() {
        let mut app = App::default();
        typed(&mut app, "fix the bug");
        app.edit_key(KeyKind::KillLineBack);
        assert!(!app.redo_edit(), "nothing was undone yet");
        app.edit_key(KeyKind::UndoEdit);
        assert_eq!(app.input, "fix the bug");
        app.edit_key(KeyKind::RedoEdit);
        assert_eq!(app.input, "");
        app.edit_key(KeyKind::UndoEdit);
        app.edit_key(KeyKind::UndoEdit);
        assert_eq!(app.input, "fix the");
        app.edit_key(KeyKind::RedoEdit);
        assert_eq!(app.input, "fix the bug");
        assert_eq!(app.input_cursor, 11);
        app.edit_key(KeyKind::UndoEdit);
        app.edit_key(KeyKind::Char('!'));
        assert!(!app.redo_edit(), "a new edit forgets what was undone");
        assert_eq!(app.input, "fix the!");
    }

    #[test]
    fn undo_restores_word_by_word_and_after_a_kill() {
        let mut app = App::default();
        typed(&mut app, "fix the bug");
        app.edit_key(KeyKind::KillLineBack);
        assert_eq!(app.input, "");
        assert!(app.undo_edit());
        assert_eq!(app.input, "fix the bug");
        assert_eq!(app.input_cursor, 11);
        app.edit_key(KeyKind::UndoEdit);
        assert_eq!(app.input, "fix the");
        app.edit_key(KeyKind::UndoEdit);
        assert_eq!(app.input, "fix");
        app.edit_key(KeyKind::UndoEdit);
        assert_eq!(app.input, "");
        assert!(!app.undo_edit(), "nothing left to undo");
    }

    #[test]
    fn backspace_runs_undo_as_one_step_and_submit_clears_history() {
        let mut app = App::default();
        typed(&mut app, "abcd");
        app.edit_key(KeyKind::Backspace);
        app.edit_key(KeyKind::Backspace);
        assert_eq!(app.input, "ab");
        app.undo_edit();
        assert_eq!(app.input, "abcd");
        assert_eq!(
            app.edit_key(KeyKind::Enter),
            InputOutcome::Submit("abcd".into())
        );
        assert!(!app.undo_edit());
        app.edit_key(KeyKind::KillLineBack);
        assert!(app.input.is_empty());
    }

    #[test]
    fn replacing_the_draft_is_one_undo_step() {
        let mut app = App::default();
        typed(&mut app, "draft");
        app.replace_input("edited in vim".into());
        assert_eq!(app.input_cursor, app.input.len());
        app.undo_edit();
        assert_eq!(app.input, "draft");
    }

    #[test]
    fn killed_text_uses_the_cursor_not_a_text_diff() {
        assert_eq!(killed_text("ab ab", 0, "ab", 0, true), "ab ");
        assert_eq!(killed_text("héllo wörld", 13, "héllo ", 7, false), "wörld");
        let mut app = App::default();
        typed(&mut app, "ab ab");
        app.input_cursor = 0;
        app.edit_key(KeyKind::KillLineForward);
        assert_eq!(app.input_edit.kill_ring(), ["ab ab"]);
    }
}
