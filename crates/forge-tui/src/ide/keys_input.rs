//! Keys that go to the text editor and to the terminal, and the bytes a shell expects for a key.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::{next_bottom, prev_bottom};
use super::state::{IdeAction, IdeApp};

pub(super) fn edit_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let page = app.view_h.saturating_sub(1).max(1);
    let loaded = app.tools.edit.as_ref().is_some_and(|e| e.buf.is_some());
    if !loaded {
        if matches!(key.code, KeyCode::Esc) {
            app.leave_edit();
        }
        return Vec::new();
    }
    match (key.code, ctrl) {
        (KeyCode::Char('s'), true) => return app.save_edit(false),
        (KeyCode::Char('z'), true) => app.edit_op(false, |b| {
            b.undo();
        }),
        (KeyCode::Char('y'), true) => app.edit_op(false, |b| {
            b.redo();
        }),
        (KeyCode::Char('w'), true) => return app.toggle_why(),
        (KeyCode::Char('t'), true) => app.open_why(),
        (KeyCode::Esc, _) => app.request_leave_edit(),
        (KeyCode::Left, _) => app.edit_op(false, |b| b.move_left()),
        (KeyCode::Right, _) => app.edit_op(false, |b| b.move_right()),
        (KeyCode::Up, _) => app.edit_op(false, |b| b.move_up()),
        (KeyCode::Down, _) => app.edit_op(false, |b| b.move_down()),
        (KeyCode::Home, true) => app.edit_op(false, |b| b.doc_start()),
        (KeyCode::End, true) => app.edit_op(false, |b| b.doc_end()),
        (KeyCode::Home, _) => app.edit_op(false, |b| b.home()),
        (KeyCode::End, _) => app.edit_op(false, |b| b.end()),
        (KeyCode::PageUp, _) => app.edit_op(false, |b| b.page_up(page)),
        (KeyCode::PageDown, _) => app.edit_op(false, |b| b.page_down(page)),
        (KeyCode::Tab, _) => app.edit_op(true, |b| b.indent()),
        (KeyCode::BackTab, _) => app.edit_op(true, |b| b.outdent()),
        (KeyCode::Enter, _) => app.edit_op(true, |b| b.newline()),
        (KeyCode::Backspace, _) => app.edit_op(true, |b| b.backspace()),
        (KeyCode::Delete, _) => app.edit_op(true, |b| b.delete()),
        (KeyCode::Char(c), false) if !key.modifiers.contains(KeyModifiers::ALT) => {
            app.edit_op(true, |b| b.insert_char(c));
        }
        _ => {}
    }
    Vec::new()
}

/// The bytes a shell expects for `key`.
pub fn key_bytes(key: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut out: Vec<u8> = match key.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => {
            vec![(c.to_ascii_lowercase() as u8) & 0x1f]
        }
        KeyCode::Char(' ') if ctrl => vec![0],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => return None,
        },
        _ => return None,
    };
    if alt && !matches!(key.code, KeyCode::Esc) {
        out.insert(0, 0x1b);
    }
    Some(out)
}

pub(super) fn term_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Left => return app.set_bottom_tab(prev_bottom(app.bottom_tab)),
            KeyCode::Right => return app.set_bottom_tab(next_bottom(app.bottom_tab)),
            _ => {}
        }
    }
    let is_esc = matches!(key.code, KeyCode::Esc);
    match key_bytes(&key) {
        Some(bytes) => app.term_key(bytes, is_esc),
        None => Vec::new(),
    }
}
