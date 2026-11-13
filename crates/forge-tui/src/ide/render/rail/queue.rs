//! The rail's Changes tab: the change queue grouped by agent, look-here files before routine ones.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::super::{cells, clip, fill, put};
use super::dot;
use crate::ide::model::{risk_totals, Risk, Tier};
use crate::ide::state::{Hit, IdeApp};
use crate::theme::glyphs::g;
use crate::theme::styles;

enum Item {
    /// "9 look here · 31 routine" over the whole queue in view.
    Summary,
    Group(usize),
    /// The "Look here" / "Routine" label over a run of unreviewed files, with the run's length.
    Section(Risk, usize),
    File(usize, usize),
}

/// A look-here file says why on a second line; reviewed and routine rows stay one line.
fn shows_reason(row: &crate::ide::model::QueueRow) -> bool {
    row.tier != Tier::Reviewed
        && row.file.risk == Some(Risk::Look)
        && !row.file.risk_reason.is_empty()
}

pub(super) fn queue(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    if app.groups.is_empty() {
        let lines: Vec<String> = if let Some(note) = &app.changes_note {
            vec![note.clone()]
        } else if app.scope.is_some() {
            vec!["This agent has not changed anything yet.".to_string()]
        } else {
            vec![
                "No changes yet.".to_string(),
                "Agents' edits show up here.".to_string(),
            ]
        };
        for (i, line) in lines.iter().enumerate() {
            put(
                buf,
                rect.x + 1,
                rect.y + i as u16,
                &clip(line, usize::from(rect.width) - 2),
                styles::faint(),
                rect.width.saturating_sub(2),
            );
        }
        return;
    }
    // Flat items so the selected row can be kept on screen and only visible ones are drawn.
    let mut items: Vec<Item> = Vec::new();
    let totals = risk_totals(&app.rows);
    if totals.ranked && totals.look_hunks + totals.routine_hunks > 0 {
        items.push(Item::Summary);
    }
    for (g_i, group) in app.groups.iter().enumerate() {
        items.push(Item::Group(g_i));
        let ranked = group.rows.iter().any(|row| row.file.risk.is_some());
        let mut section: Option<Risk> = None;
        for (r_i, row) in group.rows.iter().enumerate() {
            if ranked && row.tier != Tier::Reviewed {
                let risk = row.file.risk.unwrap_or(Risk::Look);
                if section != Some(risk) {
                    section = Some(risk);
                    let count = group
                        .rows
                        .iter()
                        .filter(|other| {
                            other.tier != Tier::Reviewed
                                && other.file.risk.unwrap_or(Risk::Look) == risk
                        })
                        .count();
                    items.push(Item::Section(risk, count));
                }
            }
            items.push(Item::File(g_i, r_i));
        }
    }
    let open_key = app.open_row().map(|r| r.key.clone());
    let selected = open_key.and_then(|key| {
        items.iter().position(|item| match item {
            Item::File(g_i, r_i) => app.groups[*g_i].rows[*r_i].key == key,
            _ => false,
        })
    });
    let height = usize::from(rect.height);
    // Groups take two rows, and so does a look-here file (its reason sits under the name); count
    // rows, not items, when finding the first visible item.
    let height_of = |item: &Item| -> usize {
        match item {
            Item::Group(_) => 2,
            Item::File(g_i, r_i) if shows_reason(&app.groups[*g_i].rows[*r_i]) => 2,
            _ => 1,
        }
    };
    let row_of = |i: usize| -> usize { items[..i].iter().map(height_of).sum() };
    let mut first_row = 0usize;
    if let Some(sel) = selected {
        let at = row_of(sel);
        if at + 1 > height {
            first_row = at + 1 - height;
        }
    }
    let mut y = rect.y;
    let mut row = 0usize;
    for item in &items {
        let item_h = height_of(item);
        if row + item_h <= first_row {
            row += item_h;
            continue;
        }
        if y >= rect.bottom() {
            break;
        }
        match item {
            Item::Summary => {
                let right = if totals.routine_hunks > 0 && usize::from(rect.width) > 30 {
                    "A accept routine"
                } else {
                    ""
                };
                let right_w = cells(right) as u16;
                let room = usize::from(rect.width).saturating_sub(2 + usize::from(right_w) + 1);
                put(
                    buf,
                    rect.x + 1,
                    y,
                    &clip(&totals.line(), room),
                    styles::fg().bold(),
                    room as u16,
                );
                if right_w > 0 {
                    put(
                        buf,
                        rect.right() - right_w - 1,
                        y,
                        right,
                        styles::faint(),
                        right_w,
                    );
                }
                y += 1;
            }
            Item::Section(risk, count) => {
                let noun = if *count == 1 { "file" } else { "files" };
                let label = match risk {
                    Risk::Look => format!("Look here {} {count} {noun}", g().sep),
                    Risk::Routine => format!("Routine {} {count} {noun}", g().sep),
                };
                let style = match risk {
                    Risk::Look => styles::muted().bold(),
                    Risk::Routine => styles::faint(),
                };
                put(
                    buf,
                    rect.x + 2,
                    y,
                    &clip(&label, usize::from(rect.width).saturating_sub(3)),
                    style,
                    rect.width.saturating_sub(3),
                );
                y += 1;
            }
            Item::Group(g_i) => {
                let group = &app.groups[*g_i];
                let (glyph, glyph_style) = dot(app, group.waiting, group.busy);
                put(buf, rect.x + 1, y, glyph, glyph_style, 1);
                put(
                    buf,
                    rect.x + 3,
                    y,
                    &clip(&group.name, usize::from(rect.width).saturating_sub(4)),
                    styles::fg().bold(),
                    rect.width.saturating_sub(4),
                );
                if y + 1 < rect.bottom() {
                    let style = if group.waiting {
                        styles::warn()
                    } else {
                        styles::faint()
                    };
                    put(
                        buf,
                        rect.x + 3,
                        y + 1,
                        &clip(&group.summary, usize::from(rect.width).saturating_sub(4)),
                        style,
                        rect.width.saturating_sub(4),
                    );
                }
                y += 2;
            }
            Item::File(g_i, r_i) => {
                let group = &app.groups[*g_i];
                let queue_row = &group.rows[*r_i];
                let is_open = selected.is_some_and(
                    |s| matches!(items[s], Item::File(a, b) if a == *g_i && b == *r_i),
                );
                let row_rect = Rect {
                    y,
                    height: 1,
                    ..rect
                };
                let style = if is_open {
                    styles::selected_row()
                } else {
                    styles::fg()
                };
                if is_open {
                    fill(buf, row_rect, style);
                }
                let (mark, mark_style) = match queue_row.tier {
                    Tier::Reviewed => (g().ok, styles::ok()),
                    Tier::Waiting => (g().awaiting, styles::warn()),
                    Tier::Conflict => (g().warn, styles::err()),
                    Tier::Failing => (g().fail, styles::err()),
                    Tier::Unreviewed => (" ", styles::faint()),
                };
                put(
                    buf,
                    rect.x,
                    y,
                    if is_open { g().prompt } else { " " },
                    styles::brand().patch(style),
                    1,
                );
                put(buf, rect.x + 2, y, mark, mark_style.patch(style), 1);
                put(
                    buf,
                    rect.x + 4,
                    y,
                    &queue_row.file.status,
                    styles::muted().patch(style),
                    1,
                );
                let stat = format!(
                    "+{} {}{}",
                    queue_row.file.adds,
                    g().diff_minus,
                    queue_row.file.dels
                );
                let name_style = if queue_row.tier == Tier::Reviewed {
                    styles::faint().patch(style)
                } else {
                    style
                };
                let room = usize::from(rect.width).saturating_sub(7 + cells(&stat) + 1);
                put(
                    buf,
                    rect.x + 6,
                    y,
                    &clip(&queue_row.path, room),
                    name_style,
                    room as u16,
                );
                let w = cells(&stat) as u16;
                put(
                    buf,
                    rect.right() - w - 1,
                    y,
                    &stat,
                    styles::faint().patch(style),
                    w,
                );
                let mut row_rect = row_rect;
                if shows_reason(queue_row) && y + 1 < rect.bottom() {
                    put(
                        buf,
                        rect.x + 6,
                        y + 1,
                        &clip(
                            &queue_row.file.risk_reason,
                            usize::from(rect.width).saturating_sub(7),
                        ),
                        styles::faint().patch(style),
                        rect.width.saturating_sub(7),
                    );
                    row_rect.height = 2;
                }
                hits.push((row_rect, Hit::Queue(queue_row.key.clone())));
                y += item_h as u16;
            }
        }
        row += item_h;
    }
}
