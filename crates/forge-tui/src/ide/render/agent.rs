//! The agent panel: header, the needs-you card, a compact timeline, tasks and the composer.
//! Built from the agent's `/ws` snapshot and history page; it adds no socket of its own.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{cells, clip, fill, money, put, rule_row, section_label, split_row};
use crate::board::model_short;
use crate::ide::state::{Hit, IdeApp, NeedsHit, Pane};
use crate::ide::timeline::TimelineKind;
use crate::theme::glyphs::g;
use crate::theme::styles;

pub(super) fn draw(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    if rect.height < 6 {
        return;
    }
    hits.push((rect, Hit::Pane(Pane::Agent)));
    let Some(id) = app.panel_session() else {
        put(
            buf,
            rect.x + 2,
            rect.y + 2,
            "pick an agent to see what it is doing",
            styles::faint(),
            rect.width.saturating_sub(3),
        );
        return;
    };
    let agent = app.panel_agent().cloned();
    let snap = app.snapshots.get(&id).cloned();
    let waiting =
        snap.as_ref().is_some_and(|s| s.waiting()) || agent.as_ref().is_some_and(|a| a.waiting);
    let busy = snap
        .as_ref()
        .map_or(agent.as_ref().is_some_and(|a| a.busy), |s| s.busy);
    let mut y = rect.y;
    let inner = Rect {
        x: rect.x + 1,
        width: rect.width.saturating_sub(1),
        ..rect
    };

    // Header: title, then model · state · cost.
    let title = agent.as_ref().map_or_else(
        || format!("session {}", &id[..id.len().min(8)]),
        |a| a.title.clone(),
    );
    let (glyph, glyph_style) = if waiting {
        (g().awaiting, styles::warn())
    } else if busy {
        (g().live_dot, styles::brand())
    } else {
        (g().live_dot, styles::faint())
    };
    put(buf, inner.x + 1, y, glyph, glyph_style, 1);
    put(
        buf,
        inner.x + 3,
        y,
        &clip(&title, usize::from(inner.width).saturating_sub(4)),
        styles::fg().bold(),
        inner.width.saturating_sub(4),
    );
    y += 1;
    let model = snap
        .as_ref()
        .map(|s| s.model.clone())
        .filter(|m| !m.is_empty())
        .or_else(|| agent.as_ref().map(|a| a.model.clone()))
        .unwrap_or_default();
    let cost = snap
        .as_ref()
        .map(|s| s.cost_usd)
        .or_else(|| agent.as_ref().map(|a| a.cost_usd))
        .unwrap_or(0.0);
    let state = if waiting {
        "needs you".to_string()
    } else if busy {
        "working".to_string()
    } else {
        "idle".to_string()
    };
    let state_style = if waiting {
        styles::warn()
    } else {
        styles::muted()
    };
    let left = format!("   {} {} {state}", model_short(&model), g().sep);
    split_row(
        buf,
        inner,
        y,
        &left,
        state_style,
        &format!("{} ", money(cost)),
        styles::muted(),
    );
    y += 1;
    rule_row(buf, inner, y);
    y += 1;

    // The needs-you card.
    if let Some(snap) = &snap {
        if let Some(prompt) = &snap.permission_prompt {
            y = card_permission(buf, inner, y, prompt, hits);
        } else if let Some(question) = &snap.question {
            y = card_question(buf, inner, y, question, &snap.question_options, hits);
        }
    }

    // Reserve the bottom: composer (2 rows) and tasks.
    let tasks = snap.as_ref().map(|s| s.tasks.clone()).unwrap_or_default();
    let task_rows = if tasks.is_empty() {
        0
    } else {
        tasks.len().min(4) as u16 + 1
    };
    let composer_h = 2u16;
    let drafts = app.tools.drafts.count(&id);
    let draft_rows = if drafts > 0 { 2u16 } else { 0 };
    let timeline_bottom = rect
        .bottom()
        .saturating_sub(composer_h + task_rows + draft_rows);
    if drafts > 0 {
        let dy = rect.bottom() - composer_h - draft_rows;
        let label = format!(
            " Review draft {} {}",
            g().sep,
            crate::ide::review::draft_label(drafts)
        );
        put(
            buf,
            inner.x,
            dy,
            &clip(&label, usize::from(inner.width)),
            styles::brand().bold(),
            inner.width,
        );
        let send = " S send review ";
        let sw = cells(send) as u16;
        put(buf, inner.x + 1, dy + 1, send, styles::selected_row(), sw);
        hits.push((
            Rect {
                x: inner.x + 1,
                y: dy + 1,
                width: sw,
                height: 1,
            },
            Hit::Draft,
        ));
        put(
            buf,
            inner.x + sw + 2,
            dy + 1,
            "d drop last",
            styles::faint(),
            inner.width.saturating_sub(sw + 3),
        );
    }
    if y + 2 > timeline_bottom {
        composer(app, buf, inner, rect.bottom() - composer_h, hits);
        return;
    }
    section_label(buf, inner, y, "Activity", "", styles::faint());
    y += 1;
    let room = usize::from(timeline_bottom - y);
    if app.timeline.is_empty() {
        put(
            buf,
            inner.x + 1,
            y,
            "no activity read yet",
            styles::faint(),
            inner.width.saturating_sub(2),
        );
    }
    let cursor = app.timeline_cursor;
    let start = (cursor + 1).saturating_sub(room);
    let focused = app.focus == Pane::Agent && !app.composer.active;
    for (index, item) in app.timeline.iter().enumerate().skip(start).take(room) {
        let row_rect = Rect {
            y,
            height: 1,
            ..inner
        };
        let selected = focused && index == cursor;
        let base = if selected {
            styles::selected_row()
        } else {
            styles::fg()
        };
        if selected {
            fill(buf, row_rect, base);
        }
        let (glyph, mut style) = match item.kind {
            TimelineKind::Prompt => ("›", styles::user()),
            TimelineKind::Reply => ("‹", styles::muted()),
            TimelineKind::Read => ("·", styles::faint()),
            TimelineKind::Edit => ("±", styles::ok()),
            TimelineKind::Command => ("$", styles::muted()),
            TimelineKind::Tool => ("·", styles::faint()),
        };
        if item.failed {
            style = styles::err();
        }
        put(buf, inner.x + 1, y, glyph, style.patch(base), 1);
        let stat = if item.kind == TimelineKind::Edit && item.adds + item.dels > 0 {
            format!("+{} {}{}", item.adds, g().diff_minus, item.dels)
        } else {
            String::new()
        };
        let label_room = usize::from(inner.width).saturating_sub(4 + cells(&stat) + 1);
        let label_style = match item.kind {
            TimelineKind::Read | TimelineKind::Tool => styles::muted(),
            _ => styles::fg(),
        };
        put(
            buf,
            inner.x + 3,
            y,
            &clip(&item.label, label_room),
            label_style.patch(base),
            label_room as u16,
        );
        if !stat.is_empty() {
            let w = cells(&stat) as u16;
            put(
                buf,
                inner.right() - w - 1,
                y,
                &stat,
                styles::faint().patch(base),
                w,
            );
        }
        hits.push((row_rect, Hit::Timeline(index)));
        y += 1;
    }

    // Tasks.
    if !tasks.is_empty() {
        let mut ty = timeline_bottom;
        let done = tasks.iter().filter(|t| t.status == "done").count();
        section_label(
            buf,
            inner,
            ty,
            "Tasks",
            &format!("{done}/{} ", tasks.len()),
            styles::faint(),
        );
        ty += 1;
        for task in tasks.iter().take(4) {
            let (glyph, style) = match task.status.as_str() {
                "done" => (g().task_done, styles::faint()),
                "in_progress" => (g().task_active, styles::fg()),
                _ => (g().task_open, styles::muted()),
            };
            put(buf, inner.x + 1, ty, glyph, style, 1);
            put(
                buf,
                inner.x + 3,
                ty,
                &clip(&task.title, usize::from(inner.width).saturating_sub(4)),
                style,
                inner.width.saturating_sub(4),
            );
            ty += 1;
        }
    }
    composer(app, buf, inner, rect.bottom() - composer_h, hits);
}

fn wrapped(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let mut line = String::new();
        for word in raw.split_whitespace() {
            if !line.is_empty() && cells(&line) + 1 + cells(word) > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
        if lines.len() >= max_lines {
            break;
        }
    }
    lines.truncate(max_lines);
    lines
}

fn card_permission(
    buf: &mut Buffer,
    rect: Rect,
    mut y: u16,
    prompt: &str,
    hits: &mut Vec<(Rect, Hit)>,
) -> u16 {
    put(
        buf,
        rect.x + 1,
        y,
        &format!("{} needs your permission", g().awaiting),
        styles::warn().bold(),
        rect.width.saturating_sub(2),
    );
    y += 1;
    for line in wrapped(prompt, usize::from(rect.width).saturating_sub(3), 3) {
        put(
            buf,
            rect.x + 2,
            y,
            &line,
            styles::fg(),
            rect.width.saturating_sub(3),
        );
        y += 1;
    }
    let allow = " y allow ";
    let deny = " n deny ";
    let aw = cells(allow) as u16;
    let dw = cells(deny) as u16;
    put(buf, rect.x + 1, y, allow, styles::ok().bold(), aw);
    hits.push((
        Rect {
            x: rect.x + 1,
            y,
            width: aw,
            height: 1,
        },
        Hit::Needs(NeedsHit::Allow),
    ));
    put(buf, rect.x + 2 + aw, y, deny, styles::err().bold(), dw);
    hits.push((
        Rect {
            x: rect.x + 2 + aw,
            y,
            width: dw,
            height: 1,
        },
        Hit::Needs(NeedsHit::Deny),
    ));
    y += 1;
    rule_row(buf, rect, y);
    y + 1
}

fn card_question(
    buf: &mut Buffer,
    rect: Rect,
    mut y: u16,
    question: &str,
    options: &[crate::board::QOption],
    hits: &mut Vec<(Rect, Hit)>,
) -> u16 {
    put(
        buf,
        rect.x + 1,
        y,
        &format!("{} asks you", g().awaiting),
        styles::warn().bold(),
        rect.width.saturating_sub(2),
    );
    y += 1;
    for line in wrapped(question, usize::from(rect.width).saturating_sub(3), 3) {
        put(
            buf,
            rect.x + 2,
            y,
            &line,
            styles::fg(),
            rect.width.saturating_sub(3),
        );
        y += 1;
    }
    for (i, option) in options.iter().take(6).enumerate() {
        let text = format!("{} {}", i + 1, option.label);
        put(
            buf,
            rect.x + 2,
            y,
            &clip(&text, usize::from(rect.width).saturating_sub(3)),
            styles::fg(),
            rect.width.saturating_sub(3),
        );
        hits.push((
            Rect {
                y,
                height: 1,
                ..rect
            },
            Hit::Needs(NeedsHit::Option(i + 1)),
        ));
        y += 1;
    }
    rule_row(buf, rect, y);
    y + 1
}

fn composer(app: &IdeApp, buf: &mut Buffer, rect: Rect, y: u16, hits: &mut Vec<(Rect, Hit)>) {
    rule_row(buf, rect, y);
    let row = Rect {
        y: y + 1,
        height: 1,
        ..rect
    };
    let read_only = app.panel_agent().is_some_and(|a| a.read_only);
    let (text, style) = if read_only {
        (
            "view only: this session runs in a terminal".to_string(),
            styles::faint(),
        )
    } else if app.composer.active {
        let room = usize::from(rect.width).saturating_sub(4);
        let text = &app.composer.text;
        let shown: String = if cells(text) > room {
            let skip = text.chars().count().saturating_sub(room);
            text.chars().skip(skip).collect()
        } else {
            text.clone()
        };
        (format!("› {shown}▏"), styles::fg())
    } else {
        (
            "m message   C continue in Claude Code".to_string(),
            styles::faint(),
        )
    };
    put(
        buf,
        rect.x + 1,
        row.y,
        &clip(&text, usize::from(rect.width).saturating_sub(2)),
        style,
        rect.width.saturating_sub(2),
    );
    hits.push((row, Hit::Composer));
}
