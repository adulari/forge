use super::*;
use crate::theme::styles;

/// Cells of text per row: the box minus its borders and padding, the 2-cell gutter (the prompt on
/// the first row, a hanging indent below it) and one spare cell for the cursor at a row's end.
fn text_width(box_width: u16) -> usize {
    (box_width as usize).saturating_sub(7).max(1)
}

/// One wrapped row of the input: a byte range of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InputRow {
    start: usize,
    end: usize,
    /// The last row of its logical line (the one a cursor at the line's end sits on).
    last: bool,
    /// The first row of its logical line.
    first: bool,
}

/// Wrap `input` into rows of at most `width` cells, breaking after the last space that fits and
/// hard-breaking a word longer than a row. Explicit newlines always start a new row.
fn wrap_rows(input: &str, width: usize) -> Vec<InputRow> {
    let mut rows = Vec::new();
    let mut line_start = 0usize;
    for line in input.split('\n') {
        let line_end = line_start + line.len();
        let mut start = line_start;
        let mut first = true;
        loop {
            let mut cells = 0usize;
            let mut break_after: Option<usize> = None;
            let mut cut: Option<usize> = None;
            for (i, c) in input[start..line_end].char_indices() {
                let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                if cells + w > width {
                    cut = Some(break_after.unwrap_or(start + i).max(start + 1));
                    break;
                }
                cells += w;
                if c == ' ' {
                    break_after = Some(start + i + 1);
                }
            }
            match cut {
                Some(at) => {
                    rows.push(InputRow {
                        start,
                        end: at,
                        last: false,
                        first,
                    });
                    start = at;
                    first = false;
                }
                None => {
                    rows.push(InputRow {
                        start,
                        end: line_end,
                        last: true,
                        first,
                    });
                    break;
                }
            }
        }
        line_start = line_end + 1;
    }
    rows
}

/// How many visual text rows the input occupies once wrapped at `box_width` (the prompt gutter
/// and any explicit newlines included), before clamping. Drives both the box height and the
/// scroll-to-cursor offset, so wrapping never hides what's being typed.
pub(crate) fn input_text_rows(input: &str, box_width: u16) -> u16 {
    wrap_rows(input, text_width(box_width)).len().max(1) as u16
}

fn input_cursor_row(input: &str, cursor: usize, box_width: u16) -> u16 {
    let rows = wrap_rows(input, text_width(box_width));
    rows.iter()
        .position(|r| cursor < r.end || (r.last && cursor <= r.end))
        .unwrap_or(rows.len().saturating_sub(1)) as u16
}

/// Dynamic input-box height: grows from [`INPUT_H`] to [`INPUT_MAX_H`] with the wrapped content.
pub fn input_box_height(input: &str, box_width: u16) -> u16 {
    (input_text_rows(input, box_width) + 2).clamp(INPUT_H, INPUT_MAX_H)
}

/// For a multiline input, the cursor position one logical line up (same column, snapped to a UTF-8
/// boundary), or `None` when the cursor is on the first row — in which case the caller recalls
/// prompt history instead of clobbering a multiline draft.
pub fn input_cursor_up(input: &str, cursor: usize) -> Option<usize> {
    let mut cursor = cursor.min(input.len());
    while !input.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }
    if cursor == 0 || !input[..cursor].contains('\n') {
        return None;
    }
    let line_start = input[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = cursor - line_start;
    let prev_nl = line_start - 1; // the '\n' that ends the previous line
    let prev_start = input[..prev_nl].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let prev_line = &input[prev_start..prev_nl];
    let mut target = col.min(prev_line.len());
    while target > 0 && !prev_line.is_char_boundary(target) {
        target -= 1;
    }
    Some(prev_start + target)
}

pub(crate) fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let mut block = chrome::composer_block(app, area.width);

    let mut cursor = app.input_cursor.min(app.input.len());
    while !app.input.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }
    // Cursor appearance: a solid brand block when focused, suppressed on the blink "off" frame,
    // and a dim hollow (underline) when the terminal window has lost focus.
    let cursor_style = if app.unfocused {
        styles::muted().add_modifier(Modifier::UNDERLINED)
    } else if app.cursor_hidden {
        Style::default()
    } else {
        styles::cursor()
    };
    let rows = wrap_rows(&app.input, text_width(area.width));
    let tokens = token_styles(&app.input);
    let style_at = |i: usize| {
        tokens
            .iter()
            .find(|(s, e, _)| i >= *s && i < *e)
            .map(|(_, _, st)| *st)
            .unwrap_or_default()
    };
    let mut text_lines: Vec<TextLine> = Vec::with_capacity(rows.len().max(1));
    for (n, row) in rows.iter().enumerate() {
        let gutter = if n == 0 {
            Span::styled(
                format!("{} ", crate::theme::glyphs::g().prompt),
                styles::brand().bold(),
            )
        } else {
            Span::raw("  ")
        };
        let mut spans = vec![gutter];
        let mut run = String::new();
        let mut run_style = Style::default();
        let flush = |spans: &mut Vec<Span<'static>>, run: &mut String, style: Style| {
            if !run.is_empty() {
                spans.push(Span::styled(std::mem::take(run), style));
            }
        };
        for (off, c) in app.input[row.start..row.end].char_indices() {
            let i = row.start + off;
            if i == cursor {
                flush(&mut spans, &mut run, run_style);
                spans.push(Span::styled(c.to_string(), cursor_style));
                continue;
            }
            let st = style_at(i);
            if st != run_style {
                flush(&mut spans, &mut run, run_style);
                run_style = st;
            }
            run.push(c);
        }
        flush(&mut spans, &mut run, run_style);
        if row.last && cursor == row.end {
            spans.push(Span::styled(" ", cursor_style));
        }
        text_lines.push(TextLine::from(spans));
    }
    if text_lines.is_empty() {
        text_lines.push(TextLine::from(vec![
            Span::styled(
                format!("{} ", crate::theme::glyphs::g().prompt),
                styles::brand().bold(),
            ),
            Span::styled(" ", cursor_style),
        ]));
    }

    // Ghost text on an empty prompt: the predicted next prompt (Tab accepts it), otherwise what
    // the prompt does right now. It never affects sizing or cursor math.
    if app.input.is_empty() && app.prompt.is_none() {
        let ghost = if app.busy {
            Some(format!(
                "queue a follow-up{}",
                crate::theme::glyphs::g().ellipsis
            ))
        } else {
            match &app.suggested_prompt {
                Some(suggestion) => Some(format!("try: {suggestion}")),
                None => Some(format!(
                    "Message Forge{}  ! runs a shell command",
                    crate::theme::glyphs::g().ellipsis
                )),
            }
        };
        if let (Some(ghost), Some(first)) = (ghost, text_lines.first_mut()) {
            first.spans.push(Span::styled(
                ghost,
                styles::faint().add_modifier(Modifier::ITALIC),
            ));
        }
    }

    let inner = block.inner(area);
    let visible_rows = inner.height.max(1);
    let total_rows = text_lines.len() as u16;
    let cursor_row = input_cursor_row(&app.input, cursor, area.width);
    let max_scroll = total_rows.saturating_sub(visible_rows);
    let scroll = cursor_row
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(max_scroll);
    if scroll > 0 && area.width >= 60 {
        let marker = format!(
            " {} {scroll} more ",
            if crate::theme::glyphs::g().unicode {
                "↑"
            } else {
                "^"
            }
        );
        block = block
            .title_bottom(TextLine::from(Span::styled(marker, styles::faint())).left_aligned());
    }
    frame.render_widget(
        Paragraph::new(ratatui::text::Text::from(text_lines))
            .block(block)
            .scroll((scroll, 0)),
        area,
    );
}

/// The styled byte ranges of the buffer: a `/command` token on the first line in brand bold, the
/// `!` of a shell-mode line in the warning hue, and every `@path` mention in the `user` hue. A
/// `//literal` escape stays plain.
fn token_styles(input: &str) -> Vec<(usize, usize, Style)> {
    let mut out = Vec::new();
    if crate::bang_command(input).is_some() {
        out.push((0, 1, styles::warn().bold()));
        return out;
    }
    let first_line_end = input.find('\n').unwrap_or(input.len());
    let first = &input[..first_line_end];
    if let Some(tok) = crate::commands::slash_token_at(first, first.len()) {
        out.push((tok.start, tok.end, styles::brand().bold()));
    }
    let mut word_start = None;
    let push_word = |start: usize, end: usize, out: &mut Vec<(usize, usize, Style)>| {
        if input[start..end].starts_with('@') && end - start > 1 {
            out.push((start, end, styles::user()));
        }
    };
    for (i, c) in input.char_indices() {
        match (c.is_whitespace(), word_start) {
            (false, None) => word_start = Some(i),
            (true, Some(start)) => {
                push_word(start, i, &mut out);
                word_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = word_start {
        push_word(start, input.len(), &mut out);
    }
    out
}

/// The styled spans of one buffer, for tests: command and mention tokens in their own spans.
#[cfg(test)]
pub(crate) fn input_spans(input: &str) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut tokens = token_styles(input);
    tokens.sort_by_key(|t| t.0);
    for (start, end, style) in tokens {
        if start > at {
            out.push(Span::raw(input[at..start].to_string()));
        }
        out.push(Span::styled(input[start..end].to_string(), style));
        at = end;
    }
    if at < input.len() {
        out.push(Span::raw(input[at..].to_string()));
    }
    out
}
