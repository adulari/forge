//! The key-hints bar. It says what a keypress does right now: a pending question first, then a
//! toast, then the keys of the focused pane. Pairs are dropped whole from the end when they do not
//! fit, so the keys at the front are never clipped.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{cells, clip, put};
use crate::board::{ConnState, ToastLevel};
use crate::ide::state::{BottomTab, Hit, IdeApp, Pane, RailFocus, RailTab};
use crate::theme::glyphs::g;
use crate::theme::styles;

pub(super) fn draw(app: &IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let (chip, chip_style) = match &app.connection {
        ConnState::Live => (format!("{} live", g().live_dot), styles::ok()),
        ConnState::Connecting => ("connecting".to_string(), styles::faint()),
        ConnState::Reconnecting => ("reconnecting".to_string(), styles::warn()),
        ConnState::Offline(why) => (format!("offline: {}", clip(why, 24)), styles::err()),
    };
    let chip_w = cells(&chip) as u16;
    let left_room = rect.width.saturating_sub(chip_w + 3);
    put(
        buf,
        rect.right().saturating_sub(chip_w + 1),
        rect.y,
        &chip,
        chip_style,
        chip_w,
    );

    if let Some(confirm) = &app.confirm {
        put(
            buf,
            rect.x + 1,
            rect.y,
            &clip(&confirm.text, usize::from(left_room)),
            styles::warn().bold(),
            left_room,
        );
        return;
    }
    if app.is_typing() {
        let pairs: Vec<(&str, &str)> = if app.tools.comment.active {
            vec![
                ("Enter", "add to draft"),
                ("Alt+Enter", "send just this"),
                ("Esc", "cancel"),
                ("Ctrl+U", "clear"),
            ]
        } else {
            vec![
                (
                    "Enter",
                    if app.composer.active {
                        "send"
                    } else {
                        "commit"
                    },
                ),
                ("Esc", "cancel"),
                ("Ctrl+U", "clear"),
            ]
        };
        pairs_row(buf, rect, left_room, &pairs);
        return;
    }
    if app.term_focused() {
        let pairs = [
            ("Esc Esc", "leave terminal"),
            ("Alt+Left/Right", "tab"),
            ("F2", "back"),
        ];
        pairs_row(buf, rect, left_room, &pairs);
        return;
    }
    if app.editing() && app.focus == Pane::Editor {
        let pairs = [
            ("Ctrl+S", "save"),
            ("Ctrl+Z", "undo"),
            ("Esc", "back"),
            ("Ctrl+W", "why"),
            ("F2", "back to chat"),
        ];
        pairs_row(buf, rect, left_room, &pairs);
        return;
    }
    if let Some(toast) = &app.toast {
        let style = match toast.level {
            ToastLevel::Ok => styles::ok(),
            ToastLevel::Error => styles::err(),
            ToastLevel::Info => styles::muted(),
        };
        put(
            buf,
            rect.x + 1,
            rect.y,
            &clip(&toast.text, usize::from(left_room)),
            style,
            left_room,
        );
        return;
    }
    if app.toast.is_none() {
        if let Some((step, text)) = app.coach_line() {
            let line = format!("{}/3  {text}   H hide", step + 1);
            put(
                buf,
                rect.x + 1,
                rect.y,
                &clip(&line, usize::from(left_room)),
                styles::brand(),
                left_room,
            );
            return;
        }
    }
    let waiting = app.snapshot().is_some_and(|s| s.waiting());
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    if waiting {
        pairs.push(("y", "allow"));
        pairs.push(("n", "deny"));
    }
    match app.focus {
        Pane::Rail if app.rail_focus == RailFocus::Agents => {
            pairs.extend([
                ("j/k", "agent"),
                ("Enter", "scope queue"),
                ("0", "all agents"),
            ]);
        }
        Pane::Rail if app.rail_tab == RailTab::Git => {
            pairs.extend([
                ("j/k", "file"),
                ("s", "stage"),
                ("c", "message"),
                ("g", "suggest"),
                ("Z", "ship"),
                ("P", "push"),
                ("Enter", "diff"),
            ]);
        }
        Pane::Rail if app.rail_tab == RailTab::Files => {
            pairs.extend([("j/k", "file"), ("Enter", "open / expand")]);
        }
        Pane::Agent => {
            pairs.extend([
                ("j/k", "timeline"),
                ("Enter", "open edit"),
                ("i", "message"),
                ("C", "continue in Claude Code"),
            ]);
        }
        Pane::Bottom => match app.bottom_tab {
            BottomTab::Checks if app.tools.checks.show_output => pairs.extend([
                ("j/k", "link"),
                ("Enter", "open file:line"),
                ("f", "fix"),
                ("Esc", "list"),
            ]),
            BottomTab::Checks => pairs.extend([
                ("Enter", "run"),
                ("o", "output"),
                ("f", "fix"),
                ("h/l", "tab"),
                ("t", "close"),
            ]),
            BottomTab::Shells => pairs.extend([
                ("Enter", "follow log"),
                ("Esc", "list"),
                ("h/l", "tab"),
                ("t", "close"),
            ]),
            BottomTab::Terminal => pairs.extend([("h/l", "tab"), ("t", "close")]),
        },
        _ => pairs.extend([
            ("j/k", "files"),
            ("n/p", "hunks"),
            ("a", "accept"),
            ("x", "revert"),
            ("u", "undo"),
            ("r", "reviewed"),
            ("c", "comment"),
            ("e", "edit"),
            ("w", "why"),
            ("S", "send review"),
            ("m", "message"),
        ]),
    }
    pairs.extend([("Tab", "pane"), ("?", "help"), ("F2", "back")]);
    pairs_row(buf, rect, left_room, &pairs);
    let _ = hits;
}

fn pairs_row(buf: &mut Buffer, rect: Rect, room: u16, pairs: &[(&str, &str)]) {
    let sep = format!(" {} ", g().sep);
    let mut x = rect.x + 1;
    let end = rect.x + room;
    for (i, (key, label)) in pairs.iter().enumerate() {
        let need = (cells(key) + 1 + cells(label) + if i > 0 { cells(&sep) } else { 0 }) as u16;
        if x + need > end {
            break;
        }
        if i > 0 {
            x = put(buf, x, rect.y, &sep, styles::faint(), end - x);
        }
        x = put(buf, x, rect.y, key, styles::key_chip(), end - x);
        x = put(
            buf,
            x,
            rect.y,
            &format!(" {label}"),
            styles::faint(),
            end.saturating_sub(x),
        );
    }
}
