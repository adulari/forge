//! The key-hints bar. It says what a keypress does right now: a pending question first, then a
//! toast, then the keys of the focused pane. Pairs are dropped whole from the end when they do not
//! fit, so the keys at the front are never clipped.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{cells, clip, put};
use crate::board::{ConnState, ToastLevel};
use crate::ide::state::{Hit, IdeApp, Pane, RailFocus, RailTab};
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
        let pairs = [
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
                ("P", "push"),
                ("Enter", "diff"),
            ]);
        }
        Pane::Agent => {
            pairs.extend([
                ("j/k", "timeline"),
                ("Enter", "open edit"),
                ("i", "message"),
            ]);
        }
        Pane::Bottom => pairs.extend([("h/l", "tab"), ("t", "close")]),
        _ => pairs.extend([
            ("j/k", "files"),
            ("n/p", "hunks"),
            ("a", "accept"),
            ("x", "revert"),
            ("u", "undo"),
            ("r", "reviewed"),
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
