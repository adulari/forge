use super::*;

/// Inner text width available to the input: box width minus the two borders, the 1-col horizontal
/// padding each side. The leading `› ` prompt (2 cols) eats into the first row, handled by callers.
fn input_inner_width(box_width: u16) -> usize {
    (box_width as usize).saturating_sub(4).max(1)
}

/// How many visual text rows the input occupies once wrapped at `box_width` (accounting for the
/// `› ` prompt on the first row and any explicit newlines), before clamping. Drives both the box
/// height and the scroll-to-cursor offset, so wrapping never hides what's being typed.
pub(crate) fn input_text_rows(input: &str, box_width: u16) -> u16 {
    let inner = input_inner_width(box_width);
    let mut rows = 0usize;
    for (i, line) in input.split('\n').enumerate() {
        // Cell WIDTH, not char count — ratatui wraps on terminal columns, so a CJK/emoji glyph
        // (2 cells) counted as 1 char under-counted rows and hid the cursor below the input box.
        let cols = unicode_width::UnicodeWidthStr::width(line) + if i == 0 { 2 } else { 0 }; // prompt on row 0
        rows += cols.saturating_sub(1) / inner + 1; // ≥1 row per logical line
    }
    rows.max(1) as u16
}

fn input_cursor_row(input: &str, cursor: usize, box_width: u16) -> u16 {
    let inner = input_inner_width(box_width);
    let mut row = 0usize;
    let mut offset = 0usize;
    for (index, line) in input.split('\n').enumerate() {
        let end = offset + line.len();
        let cursor_in_line = cursor.clamp(offset, end) - offset;
        if cursor <= end {
            let cols = unicode_width::UnicodeWidthStr::width(&line[..cursor_in_line])
                + if index == 0 { 2 } else { 0 };
            return (row + cols / inner) as u16;
        }
        let cols = unicode_width::UnicodeWidthStr::width(line) + if index == 0 { 2 } else { 0 };
        row += cols.saturating_sub(1) / inner + 1;
        offset = end + 1;
    }
    row as u16
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
    let (tone, title_text) = if app.awaiting_question() {
        (surface::SurfaceTone::Warning, "◉ answer".to_string())
    } else if app.prompt.is_some() {
        (surface::SurfaceTone::Warning, "◉ respond".to_string())
    } else if app.busy {
        (
            surface::SurfaceTone::Accent,
            format!("▸ {}", app.turn_activity.phase.input_label()),
        )
    } else {
        (surface::SurfaceTone::Brand, "✦ message".to_string())
    };
    let block =
        surface::panel(surface::title(title_text, tone), tone).padding(Padding::horizontal(1));

    // Build one ratatui Line per explicit input line so pasted newlines render as separate rows;
    // long lines are then soft-wrapped by `Wrap`. Slash-command highlighting + block cursor apply
    // to the line that contains the cursor; later lines render plain.
    let mut cursor = app.input_cursor.min(app.input.len());
    while !app.input.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }
    // Cursor appearance: a solid orange block when focused, suppressed on the blink "off" frame,
    // and a dim hollow (underline) when the terminal window has lost focus.
    let cursor_style = if app.unfocused {
        Style::default().fg(DIM).add_modifier(Modifier::UNDERLINED)
    } else if app.cursor_hidden {
        Style::default()
    } else {
        Style::default().fg(STATUSBG).bg(ORANGE)
    };
    let input_lines: Vec<&str> = app.input.split('\n').collect();
    let mut byte_off = 0usize;
    let mut text_lines: Vec<TextLine> = Vec::with_capacity(input_lines.len().max(1));
    for (i, line) in input_lines.iter().enumerate() {
        let line_end = byte_off + line.len();
        let cursor_col = if cursor >= byte_off && cursor <= line_end {
            Some(cursor - byte_off)
        } else {
            None
        };
        byte_off = line_end + 1; // skip the \n separator

        let mut spans = Vec::new();
        if i == 0 {
            spans.push(Span::styled("› ", Style::default().fg(ORANGE).bold()));
        }
        if let Some(col) = cursor_col {
            spans.extend(line_spans_with_cursor(line, col, i == 0, cursor_style));
        } else if i == 0 {
            spans.extend(input_spans(line));
        } else {
            spans.push(Span::raw(line.to_string()));
        }
        text_lines.push(TextLine::from(spans));
    }

    // Ghost placeholder on an empty, idle prompt. When the last turn predicted a likely next
    // prompt, show THAT (dim, like Claude Code) with a "tab to accept" hint instead of the
    // generic discoverability cues — it never affects sizing/cursor math, purely an extra span
    // appended after the (empty) first line.
    if app.input.is_empty() && !app.busy && app.prompt.is_none() {
        if let Some(first) = text_lines.first_mut() {
            match &app.suggested_prompt {
                Some(suggestion) => {
                    first
                        .spans
                        .push(Span::styled(suggestion.clone(), Style::default().fg(DIM)));
                    first.spans.push(Span::styled(
                        "  ⇥ tab",
                        Style::default().fg(DIM).add_modifier(Modifier::DIM),
                    ));
                }
                None => {
                    first.spans.push(Span::styled(
                        "Message…   / commands  ·  @ files  ·  ? keys",
                        Style::default().fg(DIM),
                    ));
                }
            }
        }
    }

    let inner = block.inner(area);
    let visible_rows = inner.height.max(1);
    let total_rows = input_text_rows(&app.input, area.width);
    let cursor_row = input_cursor_row(&app.input, cursor, area.width);
    let max_scroll = total_rows.saturating_sub(visible_rows);
    let scroll = cursor_row
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(max_scroll);
    frame.render_widget(
        Paragraph::new(ratatui::text::Text::from(text_lines))
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

/// Build the styled spans for the input buffer, highlighting a `/command` token wherever it
/// appears on the line (not only as the first word) so e.g. `please run /orchestrate` shows
/// `/orchestrate` in the command accent. The cursor sits at the end of the buffer, so the token
/// being edited is selected from there. A `//literal` escape stays plain.
pub(crate) fn input_spans(input: &str) -> Vec<Span<'static>> {
    if input.is_empty() {
        return vec![];
    }
    match crate::commands::slash_token_at(input, input.len()) {
        Some(tok) => {
            let mut out = Vec::with_capacity(3);
            if tok.start > 0 {
                out.push(Span::raw(input[..tok.start].to_string()));
            }
            out.push(Span::styled(
                input[tok.start..tok.end].to_string(),
                Style::default().fg(ACCENT).bold(),
            ));
            if tok.end < input.len() {
                out.push(Span::raw(input[tok.end..].to_string()));
            }
            out
        }
        None => vec![Span::raw(input.to_string())],
    }
}

/// Render one input line that contains the cursor, producing spans with a block cursor
/// (the character under the cursor shown with inverted fg/bg). For the first input line
/// (`first_line = true`) a slash-command token anywhere on the line is highlighted in orange;
/// the highlight continues correctly even when the cursor is inside the command name.
fn line_spans_with_cursor(
    line: &str,
    col: usize,
    first_line: bool,
    cursor_style: Style,
) -> Vec<Span<'static>> {
    let tok = if first_line {
        crate::commands::slash_token_at(line, line.len())
    } else {
        None
    };

    // The character at `col` (or a space if at end) becomes the cursor cell, styled by the caller
    // (solid block / blink-off / hollow-when-unfocused).
    let at_bytes = &line[col..];
    let (cursor_ch, cursor_len) = at_bytes
        .chars()
        .next()
        .map(|c| (c, c.len_utf8()))
        .unwrap_or((' ', 0));
    let cursor_span = Span::styled(cursor_ch.to_string(), cursor_style);

    match tok {
        Some(ref tok) => {
            let tok_start = tok.start;
            let tok_end = tok.end;

            let tok_span = |s: &str| -> Span<'static> {
                Span::styled(s.to_string(), Style::default().fg(ACCENT).bold())
            };

            if col < tok_start {
                // cursor is before the token
                let mut out = vec![];
                if col > 0 {
                    out.push(Span::raw(line[..col].to_string()));
                }
                out.push(cursor_span);
                let between = &line[col + cursor_len..tok_start];
                if !between.is_empty() {
                    out.push(Span::raw(between.to_string()));
                }
                out.push(tok_span(&line[tok_start..tok_end]));
                if tok_end < line.len() {
                    out.push(Span::raw(line[tok_end..].to_string()));
                }
                out
            } else if col >= tok_end {
                // cursor is after the token
                let mut out = vec![];
                if tok_start > 0 {
                    out.push(Span::raw(line[..tok_start].to_string()));
                }
                out.push(tok_span(&line[tok_start..tok_end]));
                let between = &line[tok_end..col];
                if !between.is_empty() {
                    out.push(Span::raw(between.to_string()));
                }
                out.push(cursor_span);
                let rest = &line[col + cursor_len..];
                if !rest.is_empty() {
                    out.push(Span::raw(rest.to_string()));
                }
                out
            } else {
                // cursor is inside the token
                let mut out = vec![];
                if tok_start > 0 {
                    out.push(Span::raw(line[..tok_start].to_string()));
                }
                let pre_in_tok = &line[tok_start..col];
                if !pre_in_tok.is_empty() {
                    out.push(tok_span(pre_in_tok));
                }
                out.push(cursor_span);
                let post_in_tok = &line[col + cursor_len..tok_end];
                if !post_in_tok.is_empty() {
                    out.push(tok_span(post_in_tok));
                }
                if tok_end < line.len() {
                    out.push(Span::raw(line[tok_end..].to_string()));
                }
                out
            }
        }
        None => {
            // No slash token — just render with block cursor.
            let mut out = vec![];
            if col > 0 {
                out.push(Span::raw(line[..col].to_string()));
            }
            out.push(cursor_span);
            let rest = &line[col + cursor_len..];
            if !rest.is_empty() {
                out.push(Span::raw(rest.to_string()));
            }
            out
        }
    }
}
