//! The `ask_user` form in the live region: a tab strip when several questions were asked, the
//! current question, its options as radio buttons / checkboxes, the "Other…" row and the note
//! row, with the row being typed into drawn inline.

use super::*;
use crate::question_form::{Editing, Row};

pub(crate) fn render_question_form(frame: &mut Frame, area: Rect, app: &App) {
    let Some(form) = &app.form else {
        return;
    };
    if area.height == 0 || area.width < 8 {
        return;
    }
    let width = area.width as usize;
    let q = form.question();
    let mut lines: Vec<TextLine> = Vec::new();

    // Heading: "❓ question 2 of 3" + tab strip (✓ answered, ▸ current, · pending).
    let mut head = vec![Span::styled("  ❓ ", Style::default().fg(ORANGE).bold())];
    if form.len() > 1 {
        head.push(Span::styled(
            format!("question {} of {}   ", form.current + 1, form.len()),
            Style::default().fg(ORANGE).bold(),
        ));
        for (i, qi) in form.questions.iter().enumerate() {
            let label = if qi.header.is_empty() {
                format!("{}", i + 1)
            } else {
                truncate(&qi.header, 16)
            };
            let current = i == form.current;
            let mark = if form.answered(i) {
                "✓"
            } else if current {
                "▸"
            } else {
                "·"
            };
            let style = if current {
                Style::default().fg(STATUSBG).bg(ORANGE).bold()
            } else if form.answered(i) {
                Style::default().fg(OKGREEN)
            } else {
                Style::default().fg(DIM)
            };
            head.push(Span::styled(format!(" {mark} {label} "), style));
            head.push(Span::raw(" "));
        }
    } else {
        head.push(Span::styled(
            "question".to_string(),
            Style::default().fg(ORANGE).bold(),
        ));
    }
    lines.push(TextLine::from(head));

    // The question text, wrapped; a multi-select says so.
    let kind = if q.multi {
        "  (choose all that apply)"
    } else {
        ""
    };
    for row in wrap_words(&format!("{}{kind}", q.text), width.saturating_sub(5)) {
        lines.push(TextLine::from(vec![
            Span::raw("   "),
            Span::styled(row, Style::default().fg(USER).bold()),
        ]));
    }
    lines.push(TextLine::default());

    // Rows.
    let draft = form.draft();
    let rows = form.rows();
    let label_w = q
        .options
        .iter()
        .map(|o| o.label.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(6, 28);
    let mut body: Vec<TextLine> = Vec::with_capacity(rows.len());
    for row in &rows {
        let at = *row == form.cursor;
        let caret = if at { "▸ " } else { "  " };
        let caret_style = Style::default().fg(ORANGE).bold();
        match *row {
            Row::Option(i) => {
                let o = &q.options[i];
                let chosen = draft.selected.contains(&i);
                let box_ = match (q.multi, chosen) {
                    (true, true) => "☑",
                    (true, false) => "☐",
                    (false, true) => "◉",
                    (false, false) => "○",
                };
                let box_style = if chosen {
                    Style::default().fg(OKGREEN).bold()
                } else if at {
                    Style::default().fg(ORANGE)
                } else {
                    Style::default().fg(DIM)
                };
                let label_style = if at {
                    Style::default().fg(TEXT).bold()
                } else if chosen {
                    Style::default().fg(OKGREEN)
                } else {
                    Style::default().fg(TEXT)
                };
                let mut spans = vec![
                    Span::styled(format!("   {caret}"), caret_style),
                    Span::styled(format!("{box_} "), box_style),
                    Span::styled(format!("{} ", i + 1), Style::default().fg(DIM)),
                    Span::styled(
                        format!("{:<label_w$}", truncate(&o.label, label_w)),
                        label_style,
                    ),
                ];
                if !o.description.is_empty() {
                    let cap = width.saturating_sub(label_w + 12);
                    spans.push(Span::styled(
                        format!("  {}", truncate(&o.description, cap.max(8))),
                        Style::default().fg(DIM),
                    ));
                }
                body.push(TextLine::from(spans));
            }
            Row::Other => {
                let editing = form.editing == Some(Editing::Other);
                let chosen = !draft.other.trim().is_empty();
                let mark = if chosen { "◉" } else { "○" };
                let mut spans = vec![
                    Span::styled(format!("   {caret}"), caret_style),
                    Span::styled(
                        format!("{mark} "),
                        Style::default().fg(if chosen { OKGREEN } else { DIM }),
                    ),
                    Span::styled(
                        "Other… ".to_string(),
                        Style::default()
                            .fg(if at || editing { TEXT } else { DIM })
                            .bold(),
                    ),
                ];
                if editing {
                    spans.extend(inline_field(&form.buffer, form.buffer_cursor));
                } else if chosen {
                    spans.push(Span::styled(
                        truncate(&draft.other, width.saturating_sub(16)),
                        Style::default().fg(OKGREEN),
                    ));
                } else {
                    spans.push(Span::styled(
                        "type your own answer".to_string(),
                        Style::default().fg(DIM),
                    ));
                }
                body.push(TextLine::from(spans));
            }
            Row::Note => {
                let editing = form.editing == Some(Editing::Note);
                let has = !draft.note.trim().is_empty();
                let mut spans = vec![
                    Span::styled(format!("   {caret}"), caret_style),
                    Span::styled(
                        "✎ ".to_string(),
                        Style::default().fg(if has { WARNYEL } else { DIM }),
                    ),
                    Span::styled(
                        "note ".to_string(),
                        Style::default()
                            .fg(if at || editing { TEXT } else { DIM })
                            .bold(),
                    ),
                ];
                if editing {
                    spans.extend(inline_field(&form.buffer, form.buffer_cursor));
                } else if has {
                    spans.push(Span::styled(
                        truncate(&draft.note, width.saturating_sub(14)),
                        Style::default().fg(WARNYEL),
                    ));
                } else {
                    spans.push(Span::styled(
                        "optional — anything the model should know".to_string(),
                        Style::default().fg(DIM),
                    ));
                }
                body.push(TextLine::from(spans));
            }
        }
    }

    // Fit: keep the heading + question, scroll the rows around the cursor.
    let h = area.height as usize;
    let flash_h = usize::from(form.flash.is_some());
    let avail = h.saturating_sub(lines.len() + flash_h).max(1);
    let cursor_pos = rows.iter().position(|r| *r == form.cursor).unwrap_or(0);
    let start = if body.len() <= avail {
        0
    } else {
        cursor_pos.saturating_sub(avail - 1).min(body.len() - avail)
    };
    lines.extend(body.into_iter().skip(start).take(avail));
    if let Some(flash) = form.flash {
        lines.push(TextLine::from(Span::styled(
            format!("   ⚠ {flash}"),
            Style::default().fg(WARNYEL),
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The text being typed, with a block cursor, as spans.
fn inline_field(buffer: &str, cursor: usize) -> Vec<Span<'static>> {
    let cursor = cursor.min(buffer.len());
    let (before, after) = buffer.split_at(cursor);
    let mut chars = after.chars();
    let under = chars
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    let rest: String = chars.collect();
    vec![
        Span::styled(before.to_string(), Style::default().fg(TEXT)),
        Span::styled(under, Style::default().fg(STATUSBG).bg(TEXT)),
        Span::styled(rest, Style::default().fg(TEXT)),
    ]
}

/// Greedy word wrap on display width (no hyphenation; a single over-long word is truncated).
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let w = word.chars().count();
        let cur = line.chars().count();
        if cur == 0 {
            line = truncate(word, width);
        } else if cur + 1 + w <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            out.push(std::mem::take(&mut line));
            line = truncate(word, width);
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_words_breaks_on_width_and_keeps_words_whole() {
        let rows = wrap_words("which database should the service use for sessions", 20);
        assert!(rows.iter().all(|r| r.chars().count() <= 20), "{rows:?}");
        assert_eq!(
            rows.join(" "),
            "which database should the service use for sessions"
        );
    }
}
