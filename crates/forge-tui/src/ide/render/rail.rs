//! The left rail: the agents on the project, then Changes / Files / Git for the agent in view.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{cells, clip, fill, put, rule_row, section_label, split_row};
use crate::ide::hunk::Bucket;
use crate::ide::state::{Hit, IdeApp, Pane, RailFocus, RailTab};
use crate::theme::glyphs::g;
use crate::theme::{styles, t};

mod queue;
use queue::queue;

/// Agent rows shown before the list scrolls.
const MAX_AGENT_ROWS: usize = 7;

pub(super) fn draw(
    app: &mut IdeApp,
    buf: &mut Buffer,
    rect: Rect,
    narrow: bool,
    hits: &mut Vec<(Rect, Hit)>,
) {
    if rect.height < 4 {
        return;
    }
    if narrow {
        match app.rail_focus {
            RailFocus::Agents => {
                agents(app, buf, rect, hits);
            }
            RailFocus::List => body(app, buf, rect, hits),
        }
        return;
    }
    let list_h = (app.agents.len() + 1).min(MAX_AGENT_ROWS + 1) as u16;
    let agents_rect = Rect {
        height: (list_h + 1).min(rect.height.saturating_sub(5)),
        ..rect
    };
    agents(app, buf, agents_rect, hits);
    let y = rect.y + agents_rect.height;
    tabs(
        app,
        buf,
        Rect {
            y,
            height: 1,
            ..rect
        },
        hits,
    );
    let body_rect = Rect {
        y: y + 1,
        height: rect.height.saturating_sub(agents_rect.height + 1),
        ..rect
    };
    body(app, buf, body_rect, hits);
}

fn dot(app: &IdeApp, waiting: bool, busy: bool) -> (&'static str, ratatui::style::Style) {
    if waiting {
        (g().awaiting, styles::warn())
    } else if busy {
        let glyph = if t().animates() {
            g().spinner[(app.tick as usize / 2) % g().spinner.len()]
        } else {
            g().spinner_still
        };
        (glyph, styles::brand())
    } else {
        (g().live_dot, styles::faint())
    }
}

fn agents(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let waiting = app.waiting_count();
    let (right, right_style) = if waiting > 0 {
        (format!("{waiting} waiting "), styles::warn())
    } else {
        (format!("{} ", app.agents.len()), styles::faint())
    };
    section_label(buf, rect, rect.y, "Agents", &right, right_style);
    let focused = app.focus == Pane::Rail && app.rail_focus == RailFocus::Agents;
    let y = rect.y + 1;
    if app.agents.is_empty() {
        let note = if app.changes_note.is_some() {
            "daemon unavailable"
        } else {
            "no agents on this project yet"
        };
        put(
            buf,
            rect.x + 1,
            y,
            note,
            styles::faint(),
            rect.width.saturating_sub(2),
        );
        return;
    }
    // "All agents" first, then the agents; the cursor row stays on screen.
    let room = usize::from(rect.height.saturating_sub(1));
    let total = app.agents.len() + 1;
    let cursor = app.agent_cursor + 1;
    let start = (cursor + 1)
        .saturating_sub(room)
        .min(total.saturating_sub(room));
    for (offset, index) in (start..total.min(start + room)).enumerate() {
        let y = rect.y + 1 + offset as u16;
        let row_rect = Rect {
            y,
            height: 1,
            ..rect
        };
        if index == 0 {
            let selected = app.scope.is_none();
            let style = if selected {
                styles::selected_row()
            } else {
                styles::fg()
            };
            if selected {
                fill(buf, row_rect, style);
            }
            let caret = if focused && app.agent_cursor + 1 == 0 {
                g().prompt
            } else {
                " "
            };
            put(
                buf,
                rect.x,
                y,
                &format!("{caret} All agents"),
                style,
                rect.width,
            );
            hits.push((row_rect, Hit::AllAgents));
        } else {
            let agent = &app.agents[index - 1];
            let is_cursor = focused && app.agent_cursor == index - 1;
            let scoped = app.scope.as_deref() == Some(agent.id.as_str());
            let style = if scoped || is_cursor {
                styles::selected_row()
            } else {
                styles::fg()
            };
            if scoped || is_cursor {
                fill(buf, row_rect, style);
            }
            let (glyph, glyph_style) = dot(app, agent.waiting, agent.busy);
            put(
                buf,
                rect.x,
                y,
                if is_cursor { g().prompt } else { " " },
                styles::brand().patch(style),
                1,
            );
            put(buf, rect.x + 2, y, glyph, glyph_style.patch(style), 1);
            let right = if agent.waiting {
                "needs you".to_string()
            } else if agent.adds + agent.dels > 0 {
                format!("+{} {}{}", agent.adds, g().diff_minus, agent.dels)
            } else {
                String::new()
            };
            let right_style = if agent.waiting {
                styles::warn()
            } else {
                styles::faint()
            };
            let name_room = usize::from(rect.width).saturating_sub(5 + cells(&right) + 1);
            put(
                buf,
                rect.x + 4,
                y,
                &clip(&agent.title, name_room),
                style,
                name_room as u16,
            );
            if !right.is_empty() {
                let w = cells(&right) as u16;
                put(
                    buf,
                    rect.right() - w - 1,
                    y,
                    &right,
                    right_style.patch(style),
                    w,
                );
            }
            hits.push((row_rect, Hit::Agent(agent.id.clone())));
        }
    }
}

fn tabs(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    rule_row(buf, rect, rect.y);
    let items = [
        (RailTab::Changes, "Changes", Some(app.to_review())),
        (RailTab::Files, "Files", None),
        (RailTab::Git, "Git", None),
    ];
    let mut x = rect.x + 1;
    for (tab, label, count) in items {
        let active = app.rail_tab == tab && app.rail_focus == RailFocus::List;
        let text = match count {
            Some(n) if n > 0 => format!("{label} {n}"),
            _ => label.to_string(),
        };
        let style = if active {
            styles::fg().bold()
        } else {
            styles::muted()
        };
        let w = cells(&text) as u16;
        if x + w + 1 > rect.right() {
            break;
        }
        put(buf, x, rect.y, &text, style, w);
        hits.push((
            Rect {
                x,
                width: w,
                ..rect
            },
            Hit::RailTab(tab),
        ));
        x += w + 2;
    }
}

fn body(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    match app.rail_tab {
        RailTab::Changes => queue(app, buf, rect, hits),
        RailTab::Git => git(app, buf, rect, hits),
        RailTab::Files => super::files::draw(app, buf, rect, hits),
    }
}

fn git(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let Some(id) = app.panel_session() else {
        put(
            buf,
            rect.x + 1,
            rect.y,
            "pick an agent first",
            styles::faint(),
            rect.width.saturating_sub(2),
        );
        return;
    };
    let Some(status) = app.git.get(&id) else {
        put(
            buf,
            rect.x + 1,
            rect.y,
            "reading git…",
            styles::faint(),
            rect.width.saturating_sub(2),
        );
        return;
    };
    let mut y = rect.y;
    let mut head = format!(" {} {}", g().branch, status.branch);
    if status.ahead > 0 {
        head.push_str(&format!("  {}{}", g().up, status.ahead));
    }
    if status.behind > 0 {
        head.push_str(&format!(" {}{}", g().down, status.behind));
    }
    put(
        buf,
        rect.x,
        y,
        &clip(&head, usize::from(rect.width)),
        styles::fg().bold(),
        rect.width,
    );
    y += 1;
    if let Some(op) = &status.operation {
        put(
            buf,
            rect.x + 1,
            y,
            &format!("{op} in progress"),
            styles::warn(),
            rect.width.saturating_sub(2),
        );
        y += 1;
    }
    // The commit message input.
    let input_rect = Rect {
        y,
        height: 1,
        ..rect
    };
    let (text, style) = if app.commit.active {
        (format!(" commit: {}▏", app.commit.text), styles::fg())
    } else if app.commit.text.is_empty() {
        (" commit message (c)".to_string(), styles::faint())
    } else {
        (format!(" commit: {}", app.commit.text), styles::fg())
    };
    put(
        buf,
        rect.x,
        y,
        &clip(&text, usize::from(rect.width)),
        style,
        rect.width,
    );
    hits.push((input_rect, Hit::CommitInput));
    y += 1;
    // Commit / Push buttons.
    let staged = status.staged.len();
    let can_commit = staged > 0 && !app.commit.text.trim().is_empty();
    let commit_label = format!(" Commit ({staged}) ");
    let cw = cells(&commit_label) as u16;
    put(
        buf,
        rect.x + 1,
        y,
        &commit_label,
        if can_commit {
            styles::selected_row()
        } else {
            styles::faint()
        },
        cw,
    );
    hits.push((
        Rect {
            x: rect.x + 1,
            y,
            width: cw,
            height: 1,
        },
        Hit::Commit,
    ));
    if status.has_remote {
        let push_label = if status.ahead > 0 {
            format!(" Push {}{} ", g().up, status.ahead)
        } else {
            " Push ".to_string()
        };
        let pw = cells(&push_label) as u16;
        put(
            buf,
            rect.x + cw + 2,
            y,
            &push_label,
            if status.ahead > 0 {
                styles::selected_row()
            } else {
                styles::faint()
            },
            pw,
        );
        hits.push((
            Rect {
                x: rect.x + cw + 2,
                y,
                width: pw,
                height: 1,
            },
            Hit::Push,
        ));
    }
    y += 1;
    // The ship row: what Ship will do right now, and one click to do it.
    let steps = app.ship_steps();
    let ship_label = if status.has_remote {
        " Ship it "
    } else {
        " Commit all "
    };
    let sw = cells(ship_label) as u16;
    put(
        buf,
        rect.x + 1,
        y,
        ship_label,
        if steps.is_empty() {
            styles::faint()
        } else {
            styles::brand().bold()
        },
        sw,
    );
    hits.push((
        Rect {
            x: rect.x + 1,
            y,
            width: sw,
            height: 1,
        },
        Hit::Ship,
    ));
    let hint_room = rect.width.saturating_sub(sw + 3);
    put(
        buf,
        rect.x + sw + 2,
        y,
        &clip(&app.ship_hint(), usize::from(hint_room)),
        styles::faint(),
        hint_room,
    );
    y += 1;
    let suggest = " write message (g) ";
    let gw = cells(suggest) as u16;
    put(buf, rect.x + 1, y, suggest, styles::muted(), gw);
    hits.push((
        Rect {
            x: rect.x + 1,
            y,
            width: gw,
            height: 1,
        },
        Hit::Suggest,
    ));
    y += 2;
    let list = app.git_list();
    if list.is_empty() {
        put(
            buf,
            rect.x + 1,
            y,
            "working tree clean",
            styles::faint(),
            rect.width.saturating_sub(2),
        );
        return;
    }
    let focused = app.focus == Pane::Rail && app.rail_focus == RailFocus::List;
    let room = usize::from(rect.bottom().saturating_sub(y));
    let cursor = app.git_cursor.min(list.len() - 1);
    let start = (cursor + 2)
        .saturating_sub(room)
        .min(list.len().saturating_sub(1));
    let mut last_bucket: Option<(Bucket, bool)> = None;
    for (index, (bucket, conflicted, row)) in list.iter().enumerate().skip(start) {
        if y >= rect.bottom() {
            break;
        }
        if last_bucket != Some((*bucket, *conflicted)) {
            let (label, count) = match (bucket, conflicted) {
                (_, true) => ("Conflicts", status.conflicted.len()),
                (Bucket::Staged, _) => ("Staged", status.staged.len()),
                (Bucket::Untracked, _) => ("Untracked", status.untracked.len()),
                _ => ("Changes", status.unstaged.len()),
            };
            last_bucket = Some((*bucket, *conflicted));
            put(
                buf,
                rect.x + 1,
                y,
                &format!("{label} ({count})"),
                styles::muted().bold(),
                rect.width.saturating_sub(2),
            );
            y += 1;
            if y >= rect.bottom() {
                break;
            }
        }
        let row_rect = Rect {
            y,
            height: 1,
            ..rect
        };
        let is_cursor = focused && index == cursor;
        let style = if is_cursor {
            styles::selected_row()
        } else {
            styles::fg()
        };
        if is_cursor {
            fill(buf, row_rect, style);
        }
        put(
            buf,
            rect.x,
            y,
            if is_cursor { g().prompt } else { " " },
            styles::brand().patch(style),
            1,
        );
        put(
            buf,
            rect.x + 2,
            y,
            &row.status,
            styles::muted().patch(style),
            1,
        );
        let stat = if row.adds + row.dels > 0 {
            format!("+{} {}{}", row.adds, g().diff_minus, row.dels)
        } else {
            String::new()
        };
        let room = usize::from(rect.width).saturating_sub(5 + cells(&stat) + 1);
        put(
            buf,
            rect.x + 4,
            y,
            &clip(&row.path, room),
            style,
            room as u16,
        );
        if !stat.is_empty() {
            let w = cells(&stat) as u16;
            put(
                buf,
                rect.right() - w - 1,
                y,
                &stat,
                styles::faint().patch(style),
                w,
            );
        }
        hits.push((row_rect, Hit::GitRow(index)));
        y += 1;
    }
    let _ = split_row;
}
