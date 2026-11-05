//! The editor region: one changed file as a diff, with the hunk under the cursor marked. Only the
//! visible rows are built; the parsed diff and its row layout are cached per file and hash.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use super::{cells, clip, put, put_line, rule_row};
use crate::diff_overlay::render::{fit, half, pad, unified_row, SPLIT_MIN_WIDTH};
use crate::diff_overlay::Row;
use crate::ide::hunk::{actions_for, Bucket, HunkAction};
use crate::ide::state::{Hit, IdeApp, Pane};
use crate::theme::glyphs::g;
use crate::theme::styles;

pub(super) fn draw(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    if rect.height < 3 {
        return;
    }
    let focused = app.focus == Pane::Editor;
    let Some(open) = &app.open else {
        empty(app, buf, rect);
        return;
    };
    let (session_id, path) = (open.session_id.clone(), open.path.clone());
    let loading = open.loading;
    let error = open.error.clone();
    let hunk_cursor = open.hunk;
    let scroll = open.scroll;
    let parsed = app.parsed();
    // Header: path, bucket, reviewed mark, hunk position.
    let reviewed = app.open_is_reviewed();
    let mut x = rect.x + 1;
    let status = app
        .open_row()
        .map_or("M", |r| r.file.status.as_str())
        .to_string();
    x = put(buf, x, rect.y, &format!("{status} "), styles::muted(), 3);
    let head_style = if focused {
        styles::fg().bold()
    } else {
        styles::fg()
    };
    let path_room = usize::from(rect.width).saturating_sub(34);
    x = put(
        buf,
        x,
        rect.y,
        &clip(&path, path_room.max(10)),
        head_style,
        rect.right().saturating_sub(x),
    );
    if reviewed {
        x = put(
            buf,
            x,
            rect.y,
            &format!(" {} reviewed", g().ok),
            styles::ok(),
            rect.right().saturating_sub(x),
        );
    }
    if let Some((parsed, bucket)) = &parsed {
        let label = match bucket {
            Bucket::Staged => "staged",
            Bucket::Unstaged => "unstaged",
            Bucket::Untracked => "untracked",
            Bucket::Worktree => "vs base",
        };
        let hunks = parsed.wire.hunks.len();
        let tail = format!(
            "{label} {} hunk {}/{}{}",
            g().sep,
            if hunks == 0 { 0 } else { hunk_cursor + 1 },
            hunks,
            if app.split { " split" } else { "" }
        );
        let w = cells(&tail) as u16;
        if x + w + 2 < rect.right() {
            put(buf, rect.right() - w - 1, rect.y, &tail, styles::faint(), w);
        }
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
    app.view_h = usize::from(body.height);
    let Some((parsed, bucket)) = parsed else {
        let (text, style) = match (&error, loading) {
            (Some(e), _) => (e.clone(), styles::err()),
            (None, true) => ("reading the diff…".to_string(), styles::faint()),
            (None, false) => ("no diff to show".to_string(), styles::faint()),
        };
        put(
            buf,
            body.x + 1,
            body.y,
            &clip(&text, usize::from(body.width).saturating_sub(2)),
            style,
            body.width.saturating_sub(2),
        );
        action_bar(app, buf, bar, None);
        return;
    };
    let split = app.split && body.width >= SPLIT_MIN_WIDTH;
    let (rows, starts) = parsed.layout(split);
    let overlay = &parsed.overlay;
    let width = usize::from(body.width);
    let max_scroll = rows.len().saturating_sub(usize::from(body.height).max(1));
    let scroll = scroll.min(max_scroll);
    let mut y = body.y;
    if let Some(note) = &overlay.note {
        put(
            buf,
            body.x + 1,
            y,
            &clip(note, width.saturating_sub(2)),
            styles::warn(),
            body.width.saturating_sub(2),
        );
        y += 1;
    }
    if overlay.hunks.is_empty() && overlay.note.is_none() {
        put(
            buf,
            body.x + 1,
            y,
            "no textual changes",
            styles::faint(),
            body.width.saturating_sub(2),
        );
    }
    for (offset, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(body.bottom().saturating_sub(y)))
    {
        let row_rect = Rect {
            y,
            height: 1,
            ..body
        };
        let hunk_index = starts.iter().rposition(|&s| s <= offset).unwrap_or(0);
        match row {
            Row::Hunk(h) => {
                let active = *h == hunk_cursor;
                let marker = if active { g().prompt } else { " " };
                let style = if active {
                    styles::brand().bold().patch(styles::surface())
                } else {
                    styles::brand().patch(styles::surface())
                };
                let text = pad(
                    fit(&format!("{marker}{}", overlay.hunks[*h].header), width),
                    width,
                );
                put(buf, body.x, y, &text, style, body.width);
            }
            Row::Line(h, l) => {
                let line = unified_row(&overlay.hunks[*h].lines[*l], parsed.gutter, width);
                put_line(buf, body.x, y, &line, body.width);
            }
            Row::Pair(h, left, right) => {
                let lines = &overlay.hunks[*h].lines;
                let col = width.saturating_sub(1) / 2;
                let mut spans: Vec<Span<'static>> =
                    half(left.map(|i| &lines[i]), parsed.gutter, col, true);
                spans.push(Span::styled(
                    if g().unicode { "│" } else { "|" },
                    styles::line(),
                ));
                spans.extend(half(
                    right.map(|i| &lines[i]),
                    parsed.gutter,
                    width - col - 1,
                    false,
                ));
                put_line(buf, body.x, y, &Line::from(spans), body.width);
            }
        }
        hits.push((row_rect, Hit::Hunk(hunk_index)));
        y += 1;
    }
    let read_only = app
        .agents
        .iter()
        .any(|a| a.id == app.route_session(&session_id) && a.read_only);
    let actions = actions_for(bucket, &parsed.wire, read_only);
    action_bar(
        app,
        buf,
        bar,
        Some((&actions, hunk_cursor, parsed.wire.hunks.len())),
    );
}

fn empty(app: &IdeApp, buf: &mut Buffer, rect: Rect) {
    let lines: Vec<(String, ratatui::style::Style)> = if let Some(note) = &app.changes_note {
        vec![(note.clone(), styles::err())]
    } else if app.rows.is_empty() {
        vec![
            ("Nothing to review yet.".to_string(), styles::muted()),
            (
                "When an agent changes files they queue up on the left.".to_string(),
                styles::faint(),
            ),
        ]
    } else {
        let first = app.rows.first().map(|r| r.path.clone()).unwrap_or_default();
        vec![
            (
                format!("{} files to review", app.to_review()),
                styles::muted(),
            ),
            (
                format!("press j to open the first one ({first})"),
                styles::faint(),
            ),
        ]
    };
    let top = rect.y + rect.height / 3;
    for (i, (text, style)) in lines.iter().enumerate() {
        let w = cells(text).min(usize::from(rect.width)) as u16;
        let x = rect.x + rect.width.saturating_sub(w) / 2;
        put(buf, x, top + i as u16, text, *style, w);
    }
}

/// The one-line action bar under the diff: what a keypress does to the hunk under the cursor.
fn action_bar(
    app: &IdeApp,
    buf: &mut Buffer,
    bar: Rect,
    hunk: Option<(&[HunkAction], usize, usize)>,
) {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some((actions, cursor, total)) = hunk {
        if total > 0 {
            if actions.contains(&HunkAction::Stage) {
                pairs.push(("a", format!("stage hunk {}/{}", cursor + 1, total)));
            } else if actions.contains(&HunkAction::Unstage) {
                pairs.push(("a", format!("unstage hunk {}/{}", cursor + 1, total)));
            }
            if actions.contains(&HunkAction::Revert) {
                pairs.push(("x", "revert".to_string()));
            }
        }
    }
    if !app.undo.is_empty() {
        pairs.push(("u", format!("undo ({})", app.undo.len())));
    }
    if app.open.is_some() {
        pairs.push((
            "r",
            if app.open_is_reviewed() {
                "unmark reviewed"
            } else {
                "reviewed"
            }
            .to_string(),
        ));
        pairs.push(("X", "revert file".to_string()));
        pairs.push(("v", if app.split { "unified" } else { "split" }.to_string()));
    }
    let mut x = bar.x + 1;
    for (key, label) in pairs {
        let need = (cells(key) + 1 + cells(&label) + 2) as u16;
        if x + need > bar.right() {
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
