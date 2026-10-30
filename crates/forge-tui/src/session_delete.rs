//! The permanent-delete confirmation for a session: what goes, and a typed short id to confirm.
//! Plain data and key handling; the host (`forge chat`) fills the facts from the store and does
//! the deleting, so the dialog can be tested without either.

use crate::app::KeyKind;

/// Characters of the session id the person types to confirm.
pub const CONFIRM_ID_LEN: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionDeleteDialog {
    pub id: String,
    pub title: String,
    /// The chat's own session: deleting it also starts a fresh one.
    pub current: bool,
    /// What the delete removes, one line each.
    pub facts: Vec<String>,
    /// What deliberately stays.
    pub kept: Vec<String>,
    /// Why the delete cannot go ahead, and what to do instead.
    pub refusal: Option<(String, String)>,
    pub typed: String,
}

/// What a key press asks the host to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteKeyOutcome {
    Nothing,
    Cancel,
    Confirm,
}

impl SessionDeleteDialog {
    pub fn short_id(&self) -> String {
        self.id.chars().take(CONFIRM_ID_LEN).collect()
    }

    pub fn confirmed(&self) -> bool {
        self.refusal.is_none() && self.typed.trim() == self.short_id()
    }

    pub fn handle_key(&mut self, key: KeyKind) -> DeleteKeyOutcome {
        if self.refusal.is_some() {
            return match key {
                KeyKind::Esc | KeyKind::Enter | KeyKind::Interrupt => DeleteKeyOutcome::Cancel,
                _ => DeleteKeyOutcome::Nothing,
            };
        }
        match key {
            KeyKind::Esc | KeyKind::Interrupt => DeleteKeyOutcome::Cancel,
            KeyKind::Enter if self.confirmed() => DeleteKeyOutcome::Confirm,
            KeyKind::Char(c) if self.typed.chars().count() < 2 * CONFIRM_ID_LEN => {
                self.typed.push(c);
                DeleteKeyOutcome::Nothing
            }
            KeyKind::Backspace => {
                self.typed.pop();
                DeleteKeyOutcome::Nothing
            }
            KeyKind::KillLineBack | KeyKind::DeleteWordBack => {
                self.typed.clear();
                DeleteKeyOutcome::Nothing
            }
            _ => DeleteKeyOutcome::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> SessionDeleteDialog {
        SessionDeleteDialog {
            id: "0123456789abcdef".into(),
            title: "t".into(),
            ..Default::default()
        }
    }

    fn type_str(d: &mut SessionDeleteDialog, s: &str) {
        for c in s.chars() {
            d.handle_key(KeyKind::Char(c));
        }
    }

    #[test]
    fn enter_alone_never_confirms() {
        let mut d = dialog();
        assert_eq!(d.handle_key(KeyKind::Enter), DeleteKeyOutcome::Nothing);
        type_str(&mut d, "0123");
        assert_eq!(d.handle_key(KeyKind::Enter), DeleteKeyOutcome::Nothing);
    }

    #[test]
    fn the_typed_short_id_confirms_and_backspace_edits() {
        let mut d = dialog();
        type_str(&mut d, "01234567x");
        assert!(!d.confirmed());
        d.handle_key(KeyKind::Backspace);
        assert_eq!(d.handle_key(KeyKind::Enter), DeleteKeyOutcome::Confirm);
    }

    #[test]
    fn esc_cancels_and_a_refusal_only_closes() {
        let mut d = dialog();
        assert_eq!(d.handle_key(KeyKind::Esc), DeleteKeyOutcome::Cancel);
        d.refusal = Some(("open in a terminal".into(), "run /delete there".into()));
        type_str(&mut d, "01234567");
        assert_eq!(d.handle_key(KeyKind::Enter), DeleteKeyOutcome::Cancel);
        assert!(!d.confirmed());
    }
}
