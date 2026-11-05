//! The Files tab: the project tree with changed files highlighted.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{clip, fill, put};
use crate::ide::state::{Hit, IdeApp, Pane, RailFocus};
use crate::theme::glyphs::g;
use crate::theme::styles;

pub(super) fn draw(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    if app.panel_session().is_none() {
        put(
            buf,
            rect.x + 1,
            rect.y + 1,
            "pick an agent first: its folder is what the tree shows",
            styles::faint(),
            rect.width.saturating_sub(2),
        );
        return;
    }
    let rows = app.files_rows();
    if rows.is_empty() {
        let (text, style) = match &app.tools.files.error {
            Some(e) => (e.clone(), styles::err()),
            None => ("reading the folder…".to_string(), styles::faint()),
        };
        put(
            buf,
            rect.x + 1,
            rect.y + 1,
            &clip(&text, usize::from(rect.width).saturating_sub(2)),
            style,
            rect.width.saturating_sub(2),
        );
        return;
    }
    let focused = app.focus == Pane::Rail && app.rail_focus == RailFocus::List;
    let room = usize::from(rect.height);
    let cursor = app.tools.files.cursor.min(rows.len() - 1);
    let start = (cursor + 1).saturating_sub(room);
    for (offset, row) in rows.iter().enumerate().skip(start).take(room) {
        let y = rect.y + (offset - start) as u16;
        let row_rect = Rect {
            y,
            height: 1,
            ..rect
        };
        let selected = focused && offset == cursor;
        let base = if selected {
            styles::selected_row()
        } else {
            styles::fg()
        };
        if selected {
            fill(buf, row_rect, base);
        }
        let indent = (row.depth * 2) as u16;
        let marker = if row.dir {
            if row.open {
                "v"
            } else {
                ">"
            }
        } else {
            " "
        };
        put(
            buf,
            rect.x + 1 + indent,
            y,
            marker,
            styles::faint().patch(base),
            1,
        );
        let name_style = if row.changed {
            styles::warn().patch(base)
        } else if row.dir {
            styles::muted().patch(base)
        } else {
            base
        };
        let room_w = usize::from(rect.width).saturating_sub(usize::from(indent) + 5);
        let name = if row.dir {
            format!("{}/", row.name)
        } else {
            row.name.clone()
        };
        put(
            buf,
            rect.x + 3 + indent,
            y,
            &clip(&name, room_w),
            name_style,
            room_w as u16,
        );
        if row.changed && !row.dir {
            put(
                buf,
                rect.right() - 2,
                y,
                g().live_dot,
                styles::warn().patch(base),
                1,
            );
        }
        hits.push((row_rect, Hit::File(offset)));
    }
    if app.tools.files.truncated > 0 && room > rows.len() {
        put(
            buf,
            rect.x + 1,
            rect.y + rows.len() as u16,
            &format!("{} more not shown", app.tools.files.truncated),
            styles::faint(),
            rect.width.saturating_sub(2),
        );
    }
}
