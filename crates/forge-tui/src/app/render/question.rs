//! The `ask_user` question block: the decision block in the `user` hue. The question on top, its
//! options as numbered rows with the `❯` focus, the `Other…` row with its inline field, and the
//! optional note row. There is no second input box: free text is typed into the row itself.

use unicode_width::UnicodeWidthStr;

use super::chrome::{hint, Hint};
use super::list_view::{fit, plain};
use super::permission::{content_width, split_row, wrap, Decision};
use super::*;
use crate::question_form::{Editing, QuestionForm, Row};
use crate::theme::glyphs::g;
use crate::theme::styles;

pub(super) fn question_block(app: &App, width: u16, budget: usize) -> Decision {
    let form = app.form.as_ref().expect("question block needs a form");
    let w = content_width(width);
    let q = form.question();
    let mut rows: Vec<TextLine<'static>> = Vec::new();

    let kind = if q.multi {
        "  (choose all that apply)"
    } else {
        ""
    };
    let right = vec![Span::styled(
        format!("question {}/{}", form.current + 1, form.len()),
        styles::faint(),
    )];
    let right_w = format!("question {}/{}", form.current + 1, form.len()).width() + 2;
    for (i, text) in wrap(
        &plain(&format!("{}{kind}", q.text)),
        w.saturating_sub(right_w),
    )
    .into_iter()
    .enumerate()
    {
        let left = vec![Span::styled(text, styles::user().bold())];
        rows.push(if i == 0 {
            split_row(left, right.clone(), w)
        } else {
            TextLine::from(left)
        });
    }
    if form.len() > 1 {
        let mut spans = Vec::new();
        for (i, qi) in form.questions.iter().enumerate() {
            let label = if qi.header.is_empty() {
                format!("{}", i + 1)
            } else {
                fit(&qi.header, 16)
            };
            let (mark, style) = if i == form.current {
                (g().disclose_closed, styles::user().bold())
            } else if form.answered(i) {
                (g().ok, styles::ok())
            } else {
                (g().info, styles::muted())
            };
            spans.push(Span::styled(format!("{mark} {label}"), style));
            spans.push(Span::raw("  "));
        }
        rows.push(TextLine::from(spans));
    } else if !q.header.is_empty() {
        rows.push(TextLine::from(Span::styled(
            plain(&q.header).into_owned(),
            styles::muted(),
        )));
    }
    rows.push(TextLine::default());

    let body = body_rows(form, w);
    let flash = form
        .flash
        .map(|f| TextLine::from(Span::styled(format!("{} {f}", g().warn), styles::warn())));
    let fixed = rows.len() + 1 + usize::from(flash.is_some());
    let avail = budget.saturating_sub(fixed).max(3);
    let cursor_pos = form_rows(form)
        .iter()
        .position(|r| *r == form.cursor)
        .unwrap_or(0);
    if body.len() <= avail {
        rows.extend(body);
    } else {
        let shown = avail - 1;
        let start = cursor_pos.saturating_sub(shown / 2).min(body.len() - shown);
        rows.extend(body[start..start + shown].iter().cloned());
        rows.push(TextLine::from(Span::styled(
            format!("{} {} more", g().ellipsis, body.len() - shown),
            styles::faint(),
        )));
    }
    rows.extend(flash);
    Decision {
        rail: styles::user(),
        rows,
        hints: question_hints(app),
    }
}

fn other_label() -> String {
    format!("Other{}", g().ellipsis)
}

fn form_rows(form: &QuestionForm) -> Vec<Row> {
    form.rows()
}

fn body_rows(form: &QuestionForm, w: usize) -> Vec<TextLine<'static>> {
    let q = form.question();
    let draft = form.draft();
    let label_w = q
        .options
        .iter()
        .map(|o| o.label.width())
        .max()
        .unwrap_or(0)
        .clamp(6, 20)
        .max(other_label().width());
    let other_no = q.options.len() + 1;
    let mut out = Vec::new();
    for row in form.rows() {
        let at = row == form.cursor;
        let caret = if at {
            format!("{} ", g().prompt)
        } else {
            "  ".to_string()
        };
        let mut spans: Vec<Span<'static>> = vec![Span::styled(caret, styles::brand().bold())];
        let label_style = if at {
            styles::fg().bold()
        } else {
            styles::fg()
        };
        let mut used = 2;
        let (number, label, desc_spans): (String, String, Vec<Span<'static>>) = match row {
            Row::Option(i) => {
                let o = &q.options[i];
                if q.multi {
                    let chosen = draft.selected.contains(&i);
                    let (mark, style) = if chosen {
                        (g().ok, styles::ok().bold())
                    } else {
                        (g().task_open, styles::muted())
                    };
                    spans.push(Span::styled(format!("{mark} "), style));
                    used += 2;
                } else if draft.selected.contains(&i) {
                    // A single-select answer already given stays visible on re-visit.
                    spans.push(Span::styled(format!("{} ", g().ok), styles::ok()));
                    used += 2;
                }
                (
                    (i + 1).to_string(),
                    o.label.clone(),
                    vec![Span::styled(o.description.clone(), styles::muted())],
                )
            }
            Row::Other => {
                let editing = form.editing == Some(Editing::Other);
                let chosen = !draft.other.trim().is_empty();
                let spans_desc = if editing && q.secret {
                    let (masked, at) = masked_buffer(&form.buffer, form.buffer_cursor);
                    inline_field(&masked, at, w.saturating_sub(label_w + 10))
                } else if editing {
                    inline_field(
                        &form.buffer,
                        form.buffer_cursor,
                        w.saturating_sub(label_w + 10),
                    )
                } else if chosen && q.secret {
                    vec![Span::styled(masked_buffer(&draft.other, 0).0, styles::ok())]
                } else if chosen {
                    vec![Span::styled(draft.other.clone(), styles::ok())]
                } else if q.secret {
                    vec![Span::styled(
                        "hidden as you type".to_string(),
                        styles::faint(),
                    )]
                } else {
                    vec![Span::styled(
                        "type your own answer".to_string(),
                        styles::faint(),
                    )]
                };
                let label = if q.secret {
                    "Value".to_string()
                } else {
                    other_label()
                };
                (other_no.to_string(), label, spans_desc)
            }
            Row::Note => {
                let editing = form.editing == Some(Editing::Note);
                let has = !draft.note.trim().is_empty();
                let spans_desc = if editing {
                    inline_field(
                        &form.buffer,
                        form.buffer_cursor,
                        w.saturating_sub(label_w + 10),
                    )
                } else if has {
                    vec![Span::styled(draft.note.clone(), styles::warn())]
                } else {
                    vec![Span::styled(
                        "optional, anything the model should know".to_string(),
                        styles::faint(),
                    )]
                };
                ("n".to_string(), "note".to_string(), spans_desc)
            }
        };
        spans.push(Span::styled(number.clone(), styles::muted()));
        spans.push(Span::raw("  "));
        used += number.width() + 2;
        let label = fit(&label, label_w);
        used += label.width();
        spans.push(Span::styled(
            label.clone(),
            if matches!(row, Row::Note) && !at {
                styles::muted()
            } else {
                label_style
            },
        ));
        let pad = label_w.saturating_sub(label.width()) + 2;
        spans.push(Span::raw(" ".repeat(pad)));
        used += pad;
        let room = w.saturating_sub(used);
        let mut taken = 0;
        for span in desc_spans {
            let text = fit(&span.content, room.saturating_sub(taken));
            taken += text.width();
            spans.push(Span::styled(text, span.style));
        }
        used += taken;
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        let line = TextLine::from(spans);
        out.push(if at {
            line.style(styles::selected_row())
        } else {
            line
        });
    }
    out
}

/// A secret answer as bullets, one per typed character, with the cursor mapped to the bullet
/// string's byte offset so the block cursor sits where the next character lands.
fn masked_buffer(buffer: &str, cursor: usize) -> (String, usize) {
    let cursor = cursor.min(buffer.len());
    let before = buffer.get(..cursor).map_or(0, |b| b.chars().count());
    let total = buffer.chars().count();
    let dot = '\u{2022}';
    (
        std::iter::repeat_n(dot, total).collect(),
        before * dot.len_utf8(),
    )
}

/// The text being typed, with a block cursor, clipped to `room` cells from its tail.
fn inline_field(buffer: &str, cursor: usize, room: usize) -> Vec<Span<'static>> {
    let cursor = cursor.min(buffer.len());
    let (before, after) = buffer.split_at(cursor);
    let mut chars = after.chars();
    let under = chars
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    let rest: String = chars.collect();
    // Keep the end of long text visible: drop leading characters until it fits.
    let mut before = before.to_string();
    while before.width() + 1 + rest.width() > room.max(4) && !before.is_empty() {
        before.remove(0);
    }
    vec![
        Span::styled(before, styles::fg()),
        Span::styled(under, styles::cursor()),
        Span::styled(rest, styles::fg()),
    ]
}

pub(super) fn question_hints(app: &App) -> Vec<Hint> {
    let Some(form) = &app.form else {
        return Vec::new();
    };
    if form.editing.is_some() {
        return vec![
            hint("Enter", "save"),
            hint("Esc", "cancel"),
            hint("?", "keys"),
        ];
    }
    let q = form.question();
    let picks = q.options.len() + usize::from(q.allow_other);
    let mut hints = vec![
        hint("↑↓", "choose"),
        hint("Enter", "confirm"),
        Hint {
            key: format!("1-{}", picks.max(1)),
            label: "pick",
        },
    ];
    if q.allow_other {
        hints.push(hint("o", "other"));
    }
    if q.allow_note {
        hints.push(hint("n", "add note"));
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_on_width_and_keeps_words_whole() {
        let rows = wrap("which database should the service use for sessions", 20);
        assert!(rows.iter().all(|r| r.width() <= 20), "{rows:?}");
        assert_eq!(
            rows.join(" "),
            "which database should the service use for sessions"
        );
    }
}
