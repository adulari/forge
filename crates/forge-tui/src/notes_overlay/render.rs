//! Renders `/notes`: a centered card with the checklist on top and the scratchpad below, so it
//! reads the same at 80x24 and on a wide terminal.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{Mode, NotesOverlay, Tone};
use crate::app::App;
use crate::surface::{self, SurfaceTone};
use crate::theme::{glyphs::g, styles};

fn trunc(text: &str, max: usize) -> String {
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        let next = format!("{out}{c}");
        if UnicodeWidthStr::width(next.as_str()) + 1 > max {
            break;
        }
        out.push(c);
    }
    out.push_str(g().ellipsis);
    out
}

fn project_name(project: &str) -> &str {
    project
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(project)
}

/// The key hints for the current mode. When the card is too narrow for all of them the later,
/// less common ones drop out first; the way out always stays.
fn footer(m: &NotesOverlay, max: usize) -> String {
    let mut pairs: Vec<(&str, &str)> = match &m.mode {
        Mode::Browse => vec![
            (g().up_down, "move"),
            ("Space", "tick"),
            ("a", "add"),
            ("Enter", "thread"),
            ("e", "edit"),
            ("d", "delete"),
            ("p", "pad"),
            ("Esc", "close"),
        ],
        Mode::Add | Mode::Edit(_) => vec![("Enter", "save"), ("Esc", "cancel")],
        Mode::Pad => vec![("Esc", "save"), ("Ctrl-C", "discard")],
        Mode::ConfirmDelete(_) => vec![("y", "delete"), ("n", "keep")],
    };
    loop {
        let text = format!(
            " {} ",
            pairs
                .iter()
                .map(|(k, l)| format!("{k} {l}"))
                .collect::<Vec<_>>()
                .join(&format!(" {} ", g().sep))
        );
        if UnicodeWidthStr::width(text.as_str()) <= max || pairs.len() <= 2 {
            return text;
        }
        pairs.remove(pairs.len() - 2);
    }
}

fn todo_lines(m: &NotesOverlay, width: usize, rows: usize) -> Vec<Line<'static>> {
    let todos = &m.data.todos;
    if todos.is_empty() {
        return vec![Line::from(Span::styled(
            "Nothing on the list. Press a to add what you want to get to.".to_string(),
            styles::faint(),
        ))];
    }
    let rows = rows.max(1);
    let start = m
        .cursor
        .saturating_sub(rows.saturating_sub(2))
        .min(todos.len().saturating_sub(rows));
    todos
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .map(|(i, todo)| {
            let focused = i == m.cursor;
            let caret = if focused { g().prompt } else { " " };
            let mark = if todo.done {
                g().task_done
            } else {
                g().task_open
            };
            let thread = todo
                .thread
                .as_deref()
                .map(|id| format!("  {} {id}", g().branch))
                .unwrap_or_default();
            let text_w = width.saturating_sub(4 + UnicodeWidthStr::width(thread.as_str()));
            let text_style = if todo.done {
                styles::faint().add_modifier(ratatui::style::Modifier::CROSSED_OUT)
            } else {
                styles::fg()
            };
            let mut line = Line::from(vec![
                Span::styled(
                    format!("{caret} "),
                    if focused {
                        styles::brand()
                    } else {
                        styles::faint()
                    },
                ),
                Span::styled(
                    format!("{mark} "),
                    if todo.done {
                        styles::ok()
                    } else {
                        styles::muted()
                    },
                ),
                Span::styled(trunc(&todo.text, text_w), text_style),
                Span::styled(thread, styles::faint()),
            ]);
            if focused {
                line.style = styles::selected_row();
            }
            line
        })
        .collect()
}

fn pad_lines(m: &NotesOverlay, width: usize, rows: usize) -> Vec<Line<'static>> {
    let editing = m.mode == Mode::Pad;
    let source = if editing { &m.input } else { &m.data.body };
    if source.trim().is_empty() && !editing {
        return vec![Line::from(Span::styled(
            "Empty. Press p to write plans, links and reminders.".to_string(),
            styles::faint(),
        ))];
    }
    let mut lines: Vec<&str> = source.split('\n').collect();
    // While editing, keep the tail (where the cursor is) in view; otherwise show the head.
    if editing && lines.len() > rows {
        lines = lines.split_off(lines.len() - rows);
    }
    let last = lines.len().saturating_sub(1);
    lines
        .into_iter()
        .enumerate()
        .take(rows)
        .map(|(i, text)| {
            let mut shown = trunc(text, width.saturating_sub(2));
            if editing && i == last {
                shown.push('\u{258c}');
            }
            Line::from(Span::styled(format!("  {shown}"), styles::fg()))
        })
        .collect()
}

fn heading(title: &str, right: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(title.to_string(), styles::fg().bold()),
        Span::styled(format!("  {right}"), styles::faint()),
    ])
}

/// Card height for `content` rows: borders, plus the blank rows top and bottom that
/// `padded_panel` only draws once a card is tall enough.
fn card_rows(content: u16) -> u16 {
    let padded = content + 2 + 2 * crate::overlays::PAD_Y;
    if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        content + 2
    }
}

fn build(m: &NotesOverlay, width: usize, max_rows: usize) -> Vec<Line<'static>> {
    let done = m.data.todos.len() - m.open_count();
    let input_rows = usize::from(matches!(m.mode, Mode::Add | Mode::Edit(_)));
    let notice_rows = usize::from(m.notice.is_some() || matches!(m.mode, Mode::ConfirmDelete(_)));
    // Two headings, a spacer, the input line and the notice line.
    let chrome = 3 + input_rows + notice_rows;
    let body_rows = max_rows.saturating_sub(chrome).max(2);
    let editing = m.mode == Mode::Pad;
    let pad_want = if editing {
        body_rows.saturating_sub(1).max(1)
    } else {
        m.data
            .body
            .split('\n')
            .count()
            .clamp(1, 6)
            .min((body_rows / 3).max(1))
    };
    let todo_rows = if editing {
        1
    } else {
        m.data
            .todos
            .len()
            .max(1)
            .min(body_rows.saturating_sub(pad_want).max(1))
    };

    let mut lines: Vec<Line> = vec![heading(
        "To-dos",
        format!("{} open {} {} done", m.open_count(), g().sep, done),
    )];
    lines.extend(todo_lines(m, width, todo_rows));
    if let Mode::Add | Mode::Edit(_) = &m.mode {
        let label = if matches!(m.mode, Mode::Add) {
            "add"
        } else {
            "edit"
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", g().prompt), styles::brand()),
            Span::styled(format!("{label}: "), styles::faint()),
            Span::styled(
                format!("{}\u{258c}", trunc(&m.input, width.saturating_sub(12))),
                styles::fg(),
            ),
        ]));
    }
    lines.push(Line::default());
    lines.push(heading(
        "Scratchpad",
        if editing {
            "editing".into()
        } else {
            "markdown".into()
        },
    ));
    lines.extend(pad_lines(m, width, pad_want));
    if let Mode::ConfirmDelete(id) = &m.mode {
        let name = m
            .data
            .todos
            .iter()
            .find(|t| &t.id == id)
            .map_or("this to-do", |t| t.text.as_str());
        lines.push(Line::from(Span::styled(
            trunc(&format!("{} Delete \"{name}\"? y / n", g().warn), width),
            styles::warn(),
        )));
    } else if let Some(notice) = &m.notice {
        let (glyph, style): (&str, Style) = match notice.tone {
            Tone::Ok => (g().ok, styles::ok()),
            Tone::Warn => (g().warn, styles::warn()),
        };
        lines.push(Line::from(Span::styled(
            trunc(&format!("{glyph} {}", notice.text), width),
            style,
        )));
    }
    lines
}

pub(crate) fn render_notes_overlay(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.notes_overlay;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 84);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let max_rows = (region.height as usize).saturating_sub(2 + 2 * crate::overlays::PAD_Y as usize);
    let lines = build(m, inner_w, max_rows);
    let card_h = card_rows(lines.len() as u16).min(region.height);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("Notes", SurfaceTone::Brand),
        Some(Span::styled(
            format!(" {} ", project_name(&m.data.project)),
            styles::faint(),
        )),
        Some(Span::styled(
            footer(m, (card.width as usize).saturating_sub(6)),
            styles::faint(),
        )),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 3 {
        return;
    }
    f.render_widget(Paragraph::new(lines), inner);
}
