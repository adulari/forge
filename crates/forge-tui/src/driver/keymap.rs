//! The built-in key map: what a key press means when no configured action bind claimed it.
//!
//! Readline/Claude Code chords live here (Ctrl+A/E/W/U/K/Y, Alt+Y, Ctrl+D, Ctrl+G, Ctrl+P/N,
//! Ctrl+_). A Ctrl or Alt chord with no meaning is dropped, never typed into the draft as its
//! bare letter.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::KeyKind;

fn alt_only(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::ALT) && !k.modifiers.contains(KeyModifiers::CONTROL)
}

fn ctrl(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL)
}

/// Ctrl+_ (undo the last input edit). Legacy terminals send 0x1F, which crossterm reports as
/// Ctrl+7; the kitty protocol reports Ctrl+_ or Ctrl+Shift+-.
fn is_undo_chord(k: &KeyEvent) -> bool {
    ctrl(k)
        && match k.code {
            KeyCode::Char('_') | KeyCode::Char('7') => true,
            KeyCode::Char('-') => k.modifiers.contains(KeyModifiers::SHIFT),
            _ => false,
        }
}

/// Alt+_ (redo what Ctrl+_ undid). Ctrl+Shift+_ cannot be told apart from the undo chord on a
/// legacy terminal, so redo takes the Alt form, which every terminal reports as ESC then `_`.
fn is_redo_chord(k: &KeyEvent) -> bool {
    alt_only(k)
        && match k.code {
            KeyCode::Char('_') => true,
            KeyCode::Char('-') => k.modifiers.contains(KeyModifiers::SHIFT),
            _ => false,
        }
}

/// Map a key press to its built-in meaning. `ctrl_r_reasoning` is whether the built-in Ctrl+R
/// still belongs to the reasoning toggle (it is released once that action is rebound).
pub(super) fn static_key(k: &KeyEvent, ctrl_r_reasoning: bool) -> Option<KeyKind> {
    if is_undo_chord(k) {
        return Some(KeyKind::UndoEdit);
    }
    if is_redo_chord(k) {
        return Some(KeyKind::RedoEdit);
    }
    let key = match k.code {
        // AltGr arrives as Ctrl+Alt on Windows: a non-letter it produces (`@`, `€`, `ł`) is text.
        KeyCode::Char(c)
            if ctrl(k) && k.modifiers.contains(KeyModifiers::ALT) && !c.is_ascii_alphabetic() =>
        {
            KeyKind::Char(c)
        }
        KeyCode::Char(c) if ctrl(k) => match c.to_ascii_lowercase() {
            'c' => KeyKind::Interrupt,
            'o' => KeyKind::ToggleSubagentDetail,
            't' => KeyKind::ToggleToolCard,
            'j' => KeyKind::InsertNewline,
            'w' => KeyKind::DeleteWordBack,
            'u' => KeyKind::KillLineBack,
            'k' => KeyKind::KillLineForward,
            'y' => KeyKind::Yank,
            'a' => KeyKind::Home,
            'e' => KeyKind::End,
            'f' => KeyKind::FindTranscript,
            'd' => KeyKind::ExitOrDelete,
            'g' => KeyKind::ExternalEditor,
            'p' => KeyKind::Up,
            'n' => KeyKind::Down,
            'r' if ctrl_r_reasoning => KeyKind::ToggleReasoning,
            // An unbound Ctrl chord (Ctrl+B, Ctrl+X, Ctrl+Q…) must not type its letter.
            _ => return None,
        },
        KeyCode::Char('b') if alt_only(k) => KeyKind::WordLeft,
        KeyCode::Char('f') if alt_only(k) => KeyKind::WordRight,
        KeyCode::Char('y') if alt_only(k) => KeyKind::YankPop,
        // An unbound Alt chord must not type its letter into the draft.
        KeyCode::Char(_) if alt_only(k) => return None,
        KeyCode::Char(c) => KeyKind::Char(c),
        KeyCode::Backspace if alt_only(k) => KeyKind::DeleteWordBack,
        KeyCode::Backspace => KeyKind::Backspace,
        KeyCode::Delete => KeyKind::DeleteForward,
        KeyCode::Enter
            if k.modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
        {
            KeyKind::InsertNewline
        }
        KeyCode::Enter => KeyKind::Enter,
        KeyCode::Esc => KeyKind::Esc,
        KeyCode::BackTab => KeyKind::CycleTemper,
        KeyCode::Tab if k.modifiers.contains(KeyModifiers::SHIFT) => KeyKind::CycleTemper,
        KeyCode::Up => KeyKind::Up,
        KeyCode::Down => KeyKind::Down,
        KeyCode::Left if ctrl(k) || alt_only(k) => KeyKind::WordLeft,
        KeyCode::Left => KeyKind::Left,
        KeyCode::Right if ctrl(k) || alt_only(k) => KeyKind::WordRight,
        KeyCode::Right => KeyKind::Right,
        KeyCode::Home if ctrl(k) => KeyKind::JumpPrevUser,
        KeyCode::Home => KeyKind::Home,
        KeyCode::End if ctrl(k) => KeyKind::JumpBottom,
        KeyCode::End => KeyKind::End,
        KeyCode::PageUp => KeyKind::PageUp,
        KeyCode::PageDown => KeyKind::PageDown,
        KeyCode::Tab => KeyKind::Tab,
        _ => return None,
    };
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, mods: KeyModifiers) -> Option<KeyKind> {
        static_key(&KeyEvent::new(code, mods), true)
    }

    fn ctrl_char(c: char) -> Option<KeyKind> {
        press(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn no_ctrl_chord_ever_types_its_letter() {
        for c in 'a'..='z' {
            assert_ne!(ctrl_char(c), Some(KeyKind::Char(c)), "Ctrl+{c}");
            let alt = press(KeyCode::Char(c), KeyModifiers::ALT);
            assert_ne!(alt, Some(KeyKind::Char(c)), "Alt+{c}");
        }
        for c in ['b', 'q', 'v', 's', 'z', 'l'] {
            assert_eq!(ctrl_char(c), None, "Ctrl+{c} is unbound by default");
        }
    }

    #[test]
    fn readline_chords_have_their_claude_code_meaning() {
        assert_eq!(ctrl_char('d'), Some(KeyKind::ExitOrDelete));
        assert_eq!(ctrl_char('g'), Some(KeyKind::ExternalEditor));
        assert_eq!(ctrl_char('y'), Some(KeyKind::Yank));
        assert_eq!(ctrl_char('k'), Some(KeyKind::KillLineForward));
        assert_eq!(ctrl_char('p'), Some(KeyKind::Up));
        assert_eq!(ctrl_char('n'), Some(KeyKind::Down));
        assert_eq!(
            press(KeyCode::Char('y'), KeyModifiers::ALT),
            Some(KeyKind::YankPop)
        );
        assert_eq!(ctrl_char('_'), Some(KeyKind::UndoEdit));
        assert_eq!(ctrl_char('7'), Some(KeyKind::UndoEdit), "legacy 0x1F");
        assert_eq!(
            press(
                KeyCode::Char('-'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            Some(KeyKind::UndoEdit)
        );
        assert_eq!(
            ctrl_char('-'),
            None,
            "plain Ctrl+- stays the terminal's zoom"
        );
        assert_eq!(
            press(KeyCode::Char('_'), KeyModifiers::ALT),
            Some(KeyKind::RedoEdit)
        );
        assert_eq!(
            press(KeyCode::Char('-'), KeyModifiers::ALT | KeyModifiers::SHIFT),
            Some(KeyKind::RedoEdit)
        );
    }

    #[test]
    fn ctrl_r_is_released_once_reasoning_is_rebound() {
        assert_eq!(ctrl_char('r'), Some(KeyKind::ToggleReasoning));
        let rebound = static_key(
            &KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            false,
        );
        assert_eq!(rebound, None);
    }

    #[test]
    fn altgr_text_still_types() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(press(KeyCode::Char('@'), altgr), Some(KeyKind::Char('@')));
        assert_eq!(press(KeyCode::Char('€'), altgr), Some(KeyKind::Char('€')));
        assert_eq!(press(KeyCode::Char('q'), altgr), None);
    }

    #[test]
    fn plain_and_shifted_letters_still_type() {
        assert_eq!(
            press(KeyCode::Char('x'), KeyModifiers::NONE),
            Some(KeyKind::Char('x'))
        );
        assert_eq!(
            press(KeyCode::Char('X'), KeyModifiers::SHIFT),
            Some(KeyKind::Char('X'))
        );
    }
}
