//! The light editor view: one file as editable text with a line-number gutter, syntect highlighting
//! of the visible lines only (cached per buffer version and scroll position), the why-trace gutter
//! and a cursor. Nothing here touches more than a screenful of lines.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

use super::{cells, clip, put, rule_row};
use crate::ide::state::{Hit, HlCache, IdeApp, Pane};
use crate::theme::glyphs::g;
use crate::theme::styles;

const TAB_WIDTH: usize = 4;
/// Lines highlighted above the window so a multi-line construct that began off screen still
/// colours right.
const HL_LEAD: usize = 40;

fn display_col(line: &str, col: usize) -> usize {
    line.chars()
        .take(col)
        .map(|c| {
            if c == '\t' {
                TAB_WIDTH
            } else {
                c.width().unwrap_or(0)
            }
        })
        .sum()
}

/// The char index of the character drawn at display cell `target` of `line` (tabs and wide chars
/// span several cells), or the line's length when `target` lies past its end.
pub(crate) fn char_at_display_col(line: &str, target: usize) -> usize {
    let mut cells = 0;
    for (index, c) in line.chars().enumerate() {
        cells += display_col(&c.to_string(), 1);
        if cells > target {
            return index;
        }
    }
    line.chars().count()
}

fn language_of(path: &str) -> &str {
    path.rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.contains('/'))
        .unwrap_or("")
}

/// Paint `spans` from display column `skip`, at most `max` cells wide.
fn paint(buf: &mut Buffer, x: u16, y: u16, spans: &[Span<'static>], skip: usize, max: usize) {
    let mut col = 0usize;
    let mut out = 0usize;
    for span in spans {
        for ch in span.content.chars() {
            let (text, w) = if ch == '\t' {
                ("    ".to_string(), TAB_WIDTH)
            } else {
                (ch.to_string(), ch.width().unwrap_or(0))
            };
            if col + w <= skip {
                col += w;
                continue;
            }
            if out + w > max {
                return;
            }
            if w == 0 {
                col += w;
                continue;
            }
            put(buf, x + out as u16, y, &text, span.style, w as u16);
            col += w;
            out += w;
        }
    }
}

pub(super) fn draw(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let focused = app.focus == Pane::Editor;
    let Some(open) = &app.open else {
        return;
    };
    let path = open.path.clone();
    let why = app.tools.why;
    let blame = app.blame_for_open().cloned();
    let view_h = usize::from(rect.height.saturating_sub(3));
    app.view_h = view_h;
    let Some(edit) = app.tools.edit.as_mut() else {
        return;
    };
    // Header.
    let mut x = rect.x + 1;
    x = put(buf, x, rect.y, "edit ", styles::brand(), 6);
    let tag_w = 40usize;
    let path_room = usize::from(rect.width).saturating_sub(tag_w).max(10);
    x = put(
        buf,
        x,
        rect.y,
        &clip(&path, path_room),
        if focused {
            styles::fg().bold()
        } else {
            styles::fg()
        },
        rect.right().saturating_sub(x),
    );
    if edit.dirty() {
        x = put(
            buf,
            x,
            rect.y,
            &format!(" {} modified", g().live_dot),
            styles::warn(),
            rect.right().saturating_sub(x),
        );
    }
    if edit.read_only {
        put(
            buf,
            x,
            rect.y,
            " read-only",
            styles::faint(),
            rect.right().saturating_sub(x),
        );
    }
    rule_row(buf, rect, rect.y + 1);
    let body = Rect {
        y: rect.y + 2,
        height: rect.height.saturating_sub(3),
        ..rect
    };
    let bar = Rect {
        y: rect.bottom() - 1,
        height: 1,
        ..rect
    };
    let Some(editor) = edit.buf.as_ref() else {
        let (text, style) = match (&edit.error, edit.loading) {
            (Some(e), _) => (e.clone(), styles::err()),
            _ => ("reading the file…".to_string(), styles::faint()),
        };
        put(
            buf,
            body.x + 1,
            body.y,
            &clip(&text, usize::from(body.width).saturating_sub(2)),
            style,
            body.width.saturating_sub(2),
        );
        bar_row(buf, bar, edit.loading, false, false);
        return;
    };
    let total = editor.line_count();
    let (row, col) = editor.cursor();
    let digits = total.to_string().len().max(3);
    let gutter = digits + 1 + if why { 2 } else { 0 };
    let text_x = body.x + gutter as u16;
    let text_w = usize::from(body.width).saturating_sub(gutter + 1).max(1);
    let max_scroll = total.saturating_sub(view_h.max(1));
    let scroll = edit.scroll.min(max_scroll);
    edit.scroll = scroll;
    let end = (scroll + view_h).min(total);
    // Highlight only what is on screen, cached while neither the text nor the window moves.
    let version = editor.version();
    let stale = edit
        .hl
        .as_ref()
        .is_none_or(|c| c.version != version || c.first != scroll || c.rows.len() != end - scroll);
    if stale {
        let lead = scroll.saturating_sub(HL_LEAD);
        let rows = crate::render::highlight_code(language_of(&path), &editor.lines()[lead..end]);
        edit.hl = Some(HlCache {
            version,
            first: scroll,
            rows: rows.into_iter().skip(scroll - lead).collect(),
        });
    }
    let cursor_line = editor.lines().get(row).map_or("", String::as_str);
    let cursor_col = display_col(cursor_line, col);
    let hscroll = (cursor_col + 1).saturating_sub(text_w);
    let hl = edit.hl.as_ref().map(|c| c.rows.as_slice()).unwrap_or(&[]);
    for (offset, index) in (scroll..end).enumerate() {
        let y = body.y + offset as u16;
        let current = index == row;
        let number = format!("{:>digits$} ", index + 1);
        put(
            buf,
            body.x,
            y,
            &number,
            if current {
                styles::fg()
            } else {
                styles::faint()
            },
            (digits + 1) as u16,
        );
        if why {
            if let Some(info) = blame.as_ref().and_then(|b| b.lines.get(&(index + 1))) {
                let above = blame
                    .as_ref()
                    .and_then(|b| b.lines.get(&index))
                    .is_some_and(|a| a.tool_call_id == info.tool_call_id && a.turn == info.turn);
                put(
                    buf,
                    body.x + digits as u16 + 1,
                    y,
                    if above { "│" } else { "┃" },
                    styles::brand(),
                    1,
                );
            }
        }
        match hl.get(offset) {
            Some(spans) => paint(buf, text_x, y, spans, hscroll, text_w),
            None => {
                let plain = [Span::styled(editor.lines()[index].clone(), styles::fg())];
                paint(buf, text_x, y, &plain, hscroll, text_w);
            }
        }
        hits.push((
            Rect {
                y,
                height: 1,
                ..body
            },
            Hit::EditLine(index, text_x, hscroll),
        ));
    }
    if focused && row >= scroll && row < end {
        let cx = text_x + (cursor_col - hscroll.min(cursor_col)) as u16;
        if cx < body.right() {
            let cell = &mut buf[(cx, body.y + (row - scroll) as u16)];
            let style = cell.style().add_modifier(Modifier::REVERSED);
            cell.set_style(style);
        }
    }
    let position = format!("Ln {}, Col {}  ", row + 1, col + 1);
    let w = cells(&position) as u16;
    put(buf, bar.right() - w, bar.y, &position, styles::faint(), w);
    bar_row(buf, bar, false, edit.dirty(), why);
}

fn bar_row(buf: &mut Buffer, bar: Rect, loading: bool, dirty: bool, why: bool) {
    let mut pairs: Vec<(&str, &str)> = vec![("Esc", "back")];
    if !loading {
        pairs.push(("Ctrl+S", if dirty { "save" } else { "saved" }));
        pairs.push(("Ctrl+Z", "undo"));
        pairs.push(("Ctrl+Y", "redo"));
        pairs.push(("Ctrl+W", if why { "hide why" } else { "why" }));
        pairs.push(("Ctrl+T", "who wrote this"));
    }
    let mut x = bar.x + 1;
    for (key, label) in pairs {
        let need = (cells(key) + 1 + cells(label) + 2) as u16;
        if x + need > bar.right().saturating_sub(16) {
            break;
        }
        x = put(buf, x, bar.y, key, styles::key_chip(), bar.right() - x);
        x = put(
            buf,
            x,
            bar.y,
            &format!(" {label}  "),
            styles::faint(),
            bar.right().saturating_sub(x),
        );
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;

    #[test]
    fn a_click_cell_maps_to_the_char_drawn_there() {
        assert_eq!(char_at_display_col("abc", 1), 1);
        // 日 and 本 take two cells each: cells 0-1 are 日, 2-3 are 本, 4 is ' '.
        assert_eq!(char_at_display_col("日本 foo", 3), 1);
        assert_eq!(char_at_display_col("日本 foo", 5), 3);
        // A tab spans TAB_WIDTH cells.
        assert_eq!(char_at_display_col("\tfoo", 2), 0);
        assert_eq!(char_at_display_col("\tfoo", 4), 1);
        assert_eq!(char_at_display_col("ab", 9), 2);
    }
}
