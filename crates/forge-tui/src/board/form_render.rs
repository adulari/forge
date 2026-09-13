//! Drawing the plan-and-dispatch form: a centred panel with the project, a prompt box that wraps
//! and grows, and one row per choice with its meaning spelled out under whichever is focused.
//! Every value is clickable; the prompt border flashes red and shakes on an empty submit.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

use crate::surface::{
    self, SurfaceTone, ACCENT, DIM, ERRRED, OKGREEN, SELECT_BG, SEPARATOR, SURFACE_BG, TEXT,
    VERY_DIM,
};

use super::form::{DispatchForm, FormField, FormHit};
use super::model::project_name;
use super::state::{BoardApp, Hit};
use super::widgets::{self as w, Fit, Row};

pub const PLACEHOLDER: &str =
    "Describe the work. Forge reads the project, proposes a split, and waits for your approval.";
const MIN_BOX_ROWS: usize = 3;
const MAX_BOX_ROWS: usize = 8;
/// Rows besides the prompt box: `in …`, the explanation, four fields, a gap, the buttons.
const FIXED_ROWS: u16 = 8;

pub(super) fn draw_form(app: &mut BoardApp, frame: &mut Frame, area: Rect) {
    let Some(form) = app.dispatch.form.clone() else {
        return;
    };
    if area.width < 12 || area.height < 3 {
        return;
    }
    let shaking = app.form_shaking();
    let width = area.width.saturating_sub(4).clamp(1, 84);
    // Panel border (2) + box border (2) + box side margin (2).
    let text_w = width.saturating_sub(6).max(1) as usize;
    let wanted = if form.text.text.is_empty() {
        w::wrap(PLACEHOLDER, text_w).len()
    } else {
        layout(&form.text.text, text_w).len()
    }
    .clamp(MIN_BOX_ROWS, MAX_BOX_ROWS) as u16;
    let max_h = area.height.saturating_sub(2).max(3);
    let box_rows = wanted.min(max_h.saturating_sub(2 + FIXED_ROWS + 2)).max(1);
    let height = (box_rows + 2 + FIXED_ROWS + 2).min(max_h);
    let rect = surface::modal_area(area, width, height);
    let tone = SurfaceTone::Brand;
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title("Plan & dispatch", tone),
        Some(surface::hint(
            "Enter start · Tab next · ←→ change · Esc cancel",
        )),
        tone,
    );
    if inner.width < 6 || inner.height == 0 {
        return;
    }
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    let bottom = inner.y + inner.height;
    let mut y = inner.y;

    let mut fit = Fit::new(inner.width as usize);
    fit.add(" in ", Style::default().fg(DIM));
    fit.add(project_name(&form.cwd), Style::default().fg(TEXT).bold());
    fit.add(format!("  {}", form.cwd), Style::default().fg(VERY_DIM));
    line_at(frame, inner, y, fit.line());
    y += 1;

    if y < bottom {
        let bh = (box_rows + 2).min(bottom - y);
        let shift = match (shaking, app.tick % 2) {
            (true, 0) => 0,
            (true, _) => 2,
            (false, _) => 1,
        };
        let brect = Rect {
            x: inner.x + shift,
            y,
            width: inner.width.saturating_sub(2),
            height: bh,
        };
        let focused = form.field == FormField::Prompt;
        let color = if shaking {
            ERRRED
        } else if focused {
            ACCENT
        } else {
            SEPARATOR
        };
        let text_area = w::framed(frame, brect, None, color, focused || shaking);
        draw_prompt(frame, text_area, &form, focused);
        w::push_hit(
            &mut hits,
            brect,
            inner,
            Hit::Form(FormHit::Field(FormField::Prompt)),
        );
        y += bh;
    }
    if form.field == FormField::Prompt {
        y = explain(frame, inner, y, &form);
    }
    for field in [
        FormField::Worktree,
        FormField::Mode,
        FormField::Running,
        FormField::Items,
    ] {
        if y >= bottom {
            break;
        }
        field_row(frame, inner, y, &form, field, &mut hits);
        y += 1;
        if form.field == field {
            y = explain(frame, inner, y, &form);
        }
    }
    y += 1;
    if y < bottom {
        let mut row = Row::at(inner.x + 1, y);
        let r = row.add(
            "[ Enter  start ]",
            Style::default().fg(OKGREEN).bg(SELECT_BG).bold(),
        );
        w::push_hit(&mut hits, r, inner, Hit::Form(FormHit::Start));
        row.add("  ", Style::default());
        let r = row.add("[ Esc  cancel ]", Style::default().fg(DIM));
        w::push_hit(&mut hits, r, inner, Hit::Form(FormHit::Cancel));
        row.render(frame, inner);
    }
    app.hits.extend(hits);
}

fn line_at(frame: &mut Frame, inner: Rect, y: u16, line: Line<'static>) {
    if y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(line),
            Rect {
                y,
                height: 1,
                ..inner
            },
        );
    }
}

fn explain(frame: &mut Frame, inner: Rect, y: u16, form: &DispatchForm) -> u16 {
    // Under a choice the explanation lines up with the values; when that would clip it, or under
    // the prompt box, it sits at a small indent instead.
    let text = form.explanation();
    let value_col = 16;
    let indent =
        if form.field != FormField::Prompt && value_col + w::cells(text) <= inner.width as usize {
            value_col
        } else {
            4
        };
    let mut fit = Fit::new(inner.width as usize);
    fit.add(" ".repeat(indent), Style::default());
    fit.add(text, w::italic(DIM));
    line_at(frame, inner, y, fit.line());
    y + 1
}

fn field_row(
    frame: &mut Frame,
    inner: Rect,
    y: u16,
    form: &DispatchForm,
    field: FormField,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let focused = form.field == field;
    w::push_hit(
        hits,
        Rect {
            y,
            height: 1,
            ..inner
        },
        inner,
        Hit::Form(FormHit::Field(field)),
    );
    let mut row = Row::at(inner.x, y);
    row.add(
        if focused { " ▸ " } else { "   " },
        Style::default().fg(ACCENT).bold(),
    );
    let label = match field {
        FormField::Worktree => "Worktrees",
        FormField::Mode => "Sessions may",
        FormField::Running => "Run at once",
        FormField::Items => "At most",
        FormField::Prompt => "",
    };
    row.add(
        format!("{label:<13}"),
        if focused {
            Style::default().fg(TEXT).bold()
        } else {
            Style::default().fg(DIM)
        },
    );
    match field {
        FormField::Worktree => {
            for (on, text) in [(true, "one per session"), (false, "shared directory")] {
                let chosen = form.worktree == on;
                let mut style = if chosen {
                    Style::default().fg(TEXT).bold()
                } else {
                    Style::default().fg(DIM)
                };
                if chosen && focused {
                    style = style.bg(SELECT_BG);
                }
                let r = row.add(format!("{} {text}", if chosen { "●" } else { "○" }), style);
                w::push_hit(hits, r, inner, Hit::Form(FormHit::Worktree(on)));
                row.add("   ", Style::default());
            }
        }
        FormField::Mode => stepper(&mut row, form.mode_label(), "", field, focused, inner, hits),
        FormField::Running => stepper(
            &mut row,
            &form.max_running.to_string(),
            "",
            field,
            focused,
            inner,
            hits,
        ),
        FormField::Items => stepper(
            &mut row,
            &form.max_items.to_string(),
            " sessions",
            field,
            focused,
            inner,
            hits,
        ),
        FormField::Prompt => {}
    }
    row.render(frame, inner);
}

fn stepper(
    row: &mut Row,
    value: &str,
    suffix: &str,
    field: FormField,
    focused: bool,
    clip: Rect,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let arrow = Style::default()
        .fg(if focused { ACCENT } else { VERY_DIM })
        .bold();
    let r = row.add("‹ ", arrow);
    w::push_hit(hits, r, clip, Hit::Form(FormHit::Step(field, -1)));
    let mut vstyle = Style::default().fg(TEXT).bold();
    if focused {
        vstyle = vstyle.bg(SELECT_BG);
    }
    row.add(value.to_string(), vstyle);
    let r = row.add(" ›", arrow);
    w::push_hit(hits, r, clip, Hit::Form(FormHit::Step(field, 1)));
    if !suffix.is_empty() {
        row.add(suffix.to_string(), Style::default().fg(DIM));
    }
}

/// Visual rows of the prompt as char ranges `[start, end)`, word-wrapped to `width` cells. A row
/// that exactly fills the width at the end of a line gets an empty row after it, so the cursor
/// always has a cell to sit on.
pub(super) fn layout(text: &str, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let chars: Vec<char> = text.chars().collect();
    let mut rows = Vec::new();
    let mut line_start = 0;
    loop {
        let end = chars[line_start..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(chars.len(), |p| line_start + p);
        let mut s = line_start;
        if s == end {
            rows.push((s, s));
        }
        while s < end {
            let mut e = s;
            let mut used = 0;
            while e < end {
                let cw = UnicodeWidthChar::width(chars[e]).unwrap_or(0);
                if used + cw > width {
                    break;
                }
                used += cw;
                e += 1;
            }
            if e < end {
                if let Some(space) = (s + 1..e).rev().find(|i| chars[*i] == ' ') {
                    e = space + 1;
                }
            }
            if e == s {
                e = s + 1;
            }
            rows.push((s, e));
            if e == end && used >= width {
                rows.push((end, end));
            }
            s = e;
        }
        if end >= chars.len() {
            break;
        }
        line_start = end + 1;
    }
    rows
}

/// The visual row the cursor sits on: inside a row, else at the end of one (a line's end).
pub(super) fn cursor_row(rows: &[(usize, usize)], cursor: usize) -> usize {
    rows.iter()
        .position(|(s, e)| *s <= cursor && cursor < *e)
        .or_else(|| rows.iter().rposition(|(_, e)| *e == cursor))
        .unwrap_or(rows.len().saturating_sub(1))
}

fn draw_prompt(frame: &mut Frame, area: Rect, form: &DispatchForm, focused: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let cursor_style = Style::default().fg(SURFACE_BG).bg(ACCENT);
    if form.text.text.is_empty() {
        let lines: Vec<Line> = w::wrap(PLACEHOLDER, width)
            .into_iter()
            .take(area.height as usize)
            .enumerate()
            .map(|(i, l)| {
                let mut spans = Vec::new();
                let mut chars = l.chars();
                if i == 0 && focused {
                    let first = chars.next().map(String::from).unwrap_or_default();
                    spans.push(Span::styled(first, cursor_style));
                }
                spans.push(Span::styled(chars.collect::<String>(), w::italic(VERY_DIM)));
                Line::from(spans)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let chars: Vec<char> = form.text.text.chars().collect();
    let cursor = form.text.cursor.min(chars.len());
    let rows = layout(&form.text.text, width);
    let current = cursor_row(&rows, cursor);
    let h = area.height as usize;
    let top = (current + 1).saturating_sub(h);
    let text = Style::default().fg(TEXT);
    let mut lines: Vec<Line> = Vec::new();
    for (i, (s, e)) in rows.iter().copied().enumerate().skip(top).take(h) {
        let seg =
            |a: usize, b: usize| -> String { chars[a..b].iter().filter(|c| **c != '\n').collect() };
        if focused && i == current {
            let at = chars
                .get(cursor)
                .filter(|c| cursor < e && **c != '\n')
                .map_or(" ".to_string(), char::to_string);
            let after_start = if cursor < e { cursor + 1 } else { e };
            lines.push(Line::from(vec![
                Span::styled(seg(s, cursor), text),
                Span::styled(at, cursor_style),
                Span::styled(seg(after_start, e), text),
            ]));
        } else {
            lines.push(Line::from(Span::styled(seg(s, e), text)));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}
