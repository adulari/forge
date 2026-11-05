//! The bottom panel: project checks (with `file:line` jumps), background shells with a live tail,
//! and a real terminal on the daemon drawn from the escape-parsed cell grid.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{cells, clip, fill, put, rule_row};
use crate::ide::checks::{check_state, refs_in, strip_ansi, CheckState};
use crate::ide::state::{BottomTab, Hit, IdeApp, Pane};
use crate::ide::term_grid::{CellStyle, Hue};
use crate::jobs_overlay::age_label;
use crate::theme::glyphs::g;
use crate::theme::styles;

pub(super) fn draw(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    if rect.height < 3 {
        return;
    }
    rule_row(buf, rect, rect.y);
    let focused = app.focus == Pane::Bottom;
    let mut x = rect.x + 1;
    for (tab, label) in [
        (BottomTab::Checks, "Checks"),
        (BottomTab::Shells, "Shells"),
        (BottomTab::Terminal, "Terminal"),
    ] {
        let active = app.bottom_tab == tab;
        let style = if active && focused {
            styles::selected_row()
        } else if active {
            styles::fg().bold()
        } else {
            styles::muted()
        };
        let text = format!(" {label} ");
        let w = cells(&text) as u16;
        put(buf, x, rect.y + 1, &text, style, w);
        hits.push((
            Rect {
                x,
                y: rect.y + 1,
                width: w,
                height: 1,
            },
            Hit::BottomTab(tab),
        ));
        x += w + 1;
    }
    let close = "t close";
    let cw = cells(close) as u16;
    put(
        buf,
        rect.right() - cw - 1,
        rect.y + 1,
        close,
        styles::faint(),
        cw,
    );
    hits.push((
        Rect {
            x: rect.right() - cw - 1,
            y: rect.y + 1,
            width: cw,
            height: 1,
        },
        Hit::BottomToggle,
    ));
    let body = Rect {
        x: rect.x,
        y: rect.y + 2,
        width: rect.width,
        height: rect.height.saturating_sub(2),
    };
    match app.bottom_tab {
        BottomTab::Checks => checks(app, buf, body, hits),
        BottomTab::Shells => shells(app, buf, body, hits),
        BottomTab::Terminal => terminal(app, buf, body, hits),
    }
}

fn note(buf: &mut Buffer, body: Rect, lines: &[(&str, Style)]) {
    for (i, (text, style)) in lines.iter().enumerate() {
        if i as u16 >= body.height {
            break;
        }
        put(
            buf,
            body.x + 2,
            body.y + i as u16,
            &clip(text, usize::from(body.width).saturating_sub(3)),
            *style,
            body.width.saturating_sub(3),
        );
    }
}

fn hint_row(buf: &mut Buffer, body: Rect, pairs: &[(&str, &str)]) {
    if body.height == 0 {
        return;
    }
    let y = body.bottom() - 1;
    let mut x = body.x + 2;
    for (key, label) in pairs {
        let need = (cells(key) + cells(label) + 4) as u16;
        if x + need > body.right() {
            break;
        }
        x = put(buf, x, y, key, styles::key_chip(), body.right() - x);
        x = put(
            buf,
            x,
            y,
            &format!(" {label}  "),
            styles::faint(),
            body.right().saturating_sub(x),
        );
    }
}

fn checks(app: &mut IdeApp, buf: &mut Buffer, body: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let state = &app.tools.checks;
    if !state.loaded {
        note(
            buf,
            body,
            &[("reading the project's actions…", styles::faint())],
        );
        return;
    }
    if let Some(message) = &state.note {
        note(buf, body, &[(message.as_str(), styles::err())]);
        return;
    }
    if state.rows.is_empty() {
        if state.untrusted {
            note(
                buf,
                body,
                &[
                    (
                        "This project's actions are hidden until you trust the folder.",
                        styles::muted(),
                    ),
                    (
                        "Open it once with `forge` in a terminal and accept the trust prompt.",
                        styles::faint(),
                    ),
                ],
            );
        } else {
            note(
                buf,
                body,
                &[
                    ("No project checks yet.", styles::muted()),
                    ("Add [[actions]] (id, name, command) to .forge/config.toml and they run here.", styles::faint()),
                ],
            );
        }
        return;
    }
    let tick = app.tick as usize;
    let list_h = usize::from(body.height).saturating_sub(1);
    if state.show_output {
        output(app, buf, body, hits);
        return;
    }
    let focused = app.focus == Pane::Bottom;
    let cursor = state.cursor;
    let start = (cursor + 1).saturating_sub(list_h);
    for (index, row) in state.rows.iter().enumerate().skip(start).take(list_h) {
        let y = body.y + (index - start) as u16;
        let row_rect = Rect {
            y,
            height: 1,
            ..body
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
        let (glyph, style) = match check_state(row) {
            CheckState::Running => (g().spin(tick).to_string(), styles::spinner(tick)),
            CheckState::Pass => (g().ok.to_string(), styles::ok()),
            CheckState::Fail => (g().fail.to_string(), styles::err()),
            CheckState::Idle => (g().task_open.to_string(), styles::faint()),
        };
        put(buf, body.x + 1, y, &glyph, style.patch(base), 2);
        let status = match check_state(row) {
            CheckState::Running => "running".to_string(),
            CheckState::Pass => "passed".to_string(),
            CheckState::Fail => format!("failed ({})", row.exit_code.unwrap_or(-1)),
            CheckState::Idle => "not run".to_string(),
        };
        let name_w = 18usize.min(usize::from(body.width) / 3);
        let x = put(
            buf,
            body.x + 4,
            y,
            &clip(&row.name, name_w),
            styles::fg().bold().patch(base),
            name_w as u16,
        );
        let status_x = body.x + 4 + name_w as u16 + 1;
        let _ = x;
        put(
            buf,
            status_x,
            y,
            &status,
            styles::muted().patch(base),
            body.right().saturating_sub(status_x),
        );
        let cmd_x = status_x + 14;
        if cmd_x + 6 < body.right() {
            put(
                buf,
                cmd_x,
                y,
                &clip(&row.command, usize::from(body.right() - cmd_x - 1)),
                styles::faint().patch(base),
                body.right() - cmd_x - 1,
            );
        }
        hits.push((row_rect, Hit::Check(index)));
    }
    hint_row(
        buf,
        body,
        &[
            ("Enter", "run"),
            ("o", "output"),
            ("f", "ask agent to fix"),
            ("x", "stop"),
            ("[ ]", "tab"),
        ],
    );
}

/// The line each distinct ref sits on, in the order `parse_file_refs` yields them.
fn ref_lines(lines: &[String]) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        for (found, _) in refs_in(line) {
            if seen.insert((found.path, found.line)) {
                out.push(i);
            }
        }
    }
    out
}

fn output(app: &mut IdeApp, buf: &mut Buffer, body: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let Some(row) = app.selected_check().cloned() else {
        return;
    };
    let state = &app.tools.checks;
    let raw = state.outputs.get(&row.id).map_or("", String::as_str);
    let lines: Vec<String> = strip_ansi(raw).lines().map(str::to_string).collect();
    let header = format!(
        "{} {} {}",
        row.name,
        g().sep,
        match check_state(&row) {
            CheckState::Running => "running".to_string(),
            CheckState::Pass => "passed".to_string(),
            CheckState::Fail => format!("failed ({})", row.exit_code.unwrap_or(-1)),
            CheckState::Idle => "not run".to_string(),
        }
    );
    put(
        buf,
        body.x + 2,
        body.y,
        &clip(&header, usize::from(body.width).saturating_sub(3)),
        styles::fg().bold(),
        body.width.saturating_sub(3),
    );
    let text_h = usize::from(body.height).saturating_sub(2);
    if lines.is_empty() {
        put(
            buf,
            body.x + 2,
            body.y + 1,
            if row.running {
                "waiting for output…"
            } else {
                "no output kept for this run"
            },
            styles::faint(),
            body.width.saturating_sub(3),
        );
    }
    let ref_at = ref_lines(&lines);
    let selected_line = ref_at.get(state.ref_cursor).copied();
    let end = match selected_line {
        Some(line) if line + 1 + text_h / 2 < lines.len() && lines.len() > text_h => {
            (line + 1 + text_h / 2).max(text_h)
        }
        _ => lines.len(),
    };
    let first = end.saturating_sub(text_h);
    let focused = app.focus == Pane::Bottom;
    for (offset, line) in lines[first..end].iter().enumerate() {
        let y = body.y + 1 + offset as u16;
        let index = first + offset;
        let selected = focused && selected_line == Some(index);
        let base = if selected {
            styles::selected_row()
        } else {
            styles::muted()
        };
        if selected {
            fill(
                buf,
                Rect {
                    y,
                    height: 1,
                    ..body
                },
                base,
            );
        }
        let room = usize::from(body.width).saturating_sub(3);
        let clean = line.replace('\t', "    ");
        let mut x = body.x + 2;
        let mut cursor_byte = 0;
        let mut refs = refs_in(&clean);
        refs.sort_by_key(|(_, r)| r.start);
        let mut used = 0usize;
        for (_, range) in &refs {
            if range.start < cursor_byte {
                continue;
            }
            let before = clip_raw(&clean[cursor_byte..range.start], room.saturating_sub(used));
            used += cells(&before);
            x = put(buf, x, y, &before, base, body.right().saturating_sub(x));
            let link = clip_raw(&clean[range.clone()], room.saturating_sub(used));
            used += cells(&link);
            x = put(
                buf,
                x,
                y,
                &link,
                styles::brand()
                    .underlined()
                    .patch(if selected { base } else { Style::default() }),
                body.right().saturating_sub(x),
            );
            cursor_byte = range.end;
        }
        let rest = clip_raw(&clean[cursor_byte..], room.saturating_sub(used));
        put(buf, x, y, &rest, base, body.right().saturating_sub(x));
    }
    let _ = hits;
    hint_row(
        buf,
        body,
        &[
            ("j/k", "link"),
            ("Enter", "open file:line"),
            ("f", "fix"),
            ("Esc", "list"),
        ],
    );
}

fn clip_raw(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > max {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

fn shells(app: &mut IdeApp, buf: &mut Buffer, body: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let state = &app.tools.jobs;
    if !state.loaded {
        note(
            buf,
            body,
            &[("reading background shells…", styles::faint())],
        );
        return;
    }
    if let Some(message) = &state.note {
        note(buf, body, &[(message.as_str(), styles::err())]);
        return;
    }
    if let Some(job) = state.viewing {
        let title = format!(
            "job {job} {} {}",
            g().sep,
            if state.tail_status.is_empty() {
                "reading…"
            } else {
                state.tail_status.as_str()
            }
        );
        put(
            buf,
            body.x + 2,
            body.y,
            &clip(&title, usize::from(body.width).saturating_sub(3)),
            styles::fg().bold(),
            body.width.saturating_sub(3),
        );
        let text_h = usize::from(body.height).saturating_sub(2);
        if state.tail.is_empty() {
            put(
                buf,
                body.x + 2,
                body.y + 1,
                "(no output yet)",
                styles::faint(),
                body.width.saturating_sub(3),
            );
        }
        let first = state.tail.len().saturating_sub(text_h);
        for (offset, line) in state.tail[first..].iter().enumerate() {
            put(
                buf,
                body.x + 2,
                body.y + 1 + offset as u16,
                &clip_raw(&strip_ansi(line), usize::from(body.width).saturating_sub(3)),
                styles::muted(),
                body.width.saturating_sub(3),
            );
        }
        hint_row(buf, body, &[("Esc", "list"), ("[ ]", "tab")]);
        return;
    }
    if state.rows.is_empty() {
        note(
            buf,
            body,
            &[
                ("No background shells in this workspace.", styles::muted()),
                ("The agent starts one with shell{background:true}; it shows up here with a live tail.", styles::faint()),
            ],
        );
        return;
    }
    let tick = app.tick as usize;
    let list_h = usize::from(body.height).saturating_sub(1);
    let focused = app.focus == Pane::Bottom;
    let cursor = state.cursor;
    let start = (cursor + 1).saturating_sub(list_h);
    let now = app.now;
    for (index, job) in state.rows.iter().enumerate().skip(start).take(list_h) {
        let y = body.y + (index - start) as u16;
        let row_rect = Rect {
            y,
            height: 1,
            ..body
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
        let (glyph, style) = if job.running {
            (g().spin(tick).to_string(), styles::spinner(tick))
        } else if job.status.starts_with("exited 0") {
            (g().ok.to_string(), styles::ok())
        } else {
            (g().fail.to_string(), styles::err())
        };
        put(buf, body.x + 1, y, &glyph, style.patch(base), 2);
        let age = age_label(now.saturating_sub(job.started_at).max(0) as u64);
        let head = format!("{:<6} {:<10} {:>4}  ", job.id, job.status, age);
        let hx = put(
            buf,
            body.x + 4,
            y,
            &head,
            styles::muted().patch(base),
            body.width.saturating_sub(5),
        );
        put(
            buf,
            hx,
            y,
            &clip(
                &job.command,
                usize::from(body.right().saturating_sub(hx + 1)),
            ),
            base,
            body.right().saturating_sub(hx + 1),
        );
        hits.push((row_rect, Hit::Job(index)));
    }
    hint_row(
        buf,
        body,
        &[("j/k", "pick"), ("Enter", "follow log"), ("[ ]", "tab")],
    );
}

fn term_color(color: Hue) -> Option<ratatui::style::Color> {
    match color {
        Hue::Default => None,
        Hue::Indexed(n) => Some(styles::terminal_indexed(n)),
        Hue::Rgb(r, g, b) => Some(styles::terminal_rgb(r, g, b)),
    }
}

fn term_style(cell: &CellStyle) -> Style {
    let mut style = Style::default();
    if let Some(fg) = term_color(cell.fg) {
        style = style.fg(fg);
    }
    if let Some(bg) = term_color(cell.bg) {
        style = style.bg(bg);
    }
    let mut modifier = Modifier::empty();
    for (on, flag) in [
        (cell.bold, Modifier::BOLD),
        (cell.dim, Modifier::DIM),
        (cell.italic, Modifier::ITALIC),
        (cell.underline, Modifier::UNDERLINED),
        (cell.reverse, Modifier::REVERSED),
    ] {
        if on {
            modifier |= flag;
        }
    }
    style.add_modifier(modifier)
}

fn terminal(app: &mut IdeApp, buf: &mut Buffer, body: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let inner = Rect {
        x: body.x + 1,
        y: body.y,
        width: body.width.saturating_sub(2),
        height: body.height.saturating_sub(1),
    };
    app.term_size_hint(inner.width, inner.height);
    hits.push((body, Hit::Terminal));
    let focused = app.term_focused();
    let ended = app.tools.term.ended;
    let Some(grid) = app.tools.term.grid.as_ref() else {
        note(
            buf,
            body,
            &[("Terminal on the daemon: opening…", styles::faint())],
        );
        return;
    };
    let (cols, rows) = grid.size();
    for row in 0..rows.min(inner.height) {
        for col in 0..cols.min(inner.width) {
            let cell = grid.cell(row, col);
            if cell.ch == '\0' {
                continue;
            }
            let target = &mut buf[(inner.x + col, inner.y + row)];
            let mut text = [0u8; 4];
            target.set_symbol(cell.ch.encode_utf8(&mut text));
            target.set_style(term_style(&cell.style));
        }
    }
    let (cx, cy, visible) = grid.cursor();
    if focused && visible && cx < inner.width && cy < inner.height {
        let target = &mut buf[(inner.x + cx, inner.y + cy)];
        let style = target.style().add_modifier(Modifier::REVERSED);
        target.set_style(style);
    }
    let footer = body.bottom() - 1;
    let text = match ended {
        Some(code) => format!(
            "shell exited{}: press any key to start a new one",
            code.map_or(String::new(), |c| format!(" ({c})"))
        ),
        None if focused => "keys go to the shell · Esc Esc leaves · Alt+Left/Right tab".to_string(),
        None => "Tab to focus the terminal".to_string(),
    };
    put(
        buf,
        body.x + 2,
        footer,
        &clip(&text, usize::from(body.width).saturating_sub(3)),
        if ended.is_some() {
            styles::warn()
        } else {
            styles::faint()
        },
        body.width.saturating_sub(3),
    );
    let _ = (app.focus, Pane::Bottom);
}
