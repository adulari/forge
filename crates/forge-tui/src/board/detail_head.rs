//! The detail pane's chrome: the tab strip and the "this session is blocked on you" box.

use crate::theme::{color, styles};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::model::Card;
use super::state::{BoardApp, Button, DetailTab, Hit};
use super::widgets as w;
use super::wire::LiveSnapshot;

// ───────────────────────────────────────────────────────────────────── tabs

pub(super) fn draw_tabs(
    app: &BoardApp,
    frame: &mut Frame,
    inner: Rect,
    card: &Card,
    snap: Option<&LiveSnapshot>,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let done = snap.map_or(0, |s| s.tasks.iter().filter(|t| t.status == "done").count());
    let total = snap.map_or(0, |s| s.tasks.len());
    let changes = snap
        .and_then(|s| s.diff.as_ref())
        .map_or(0, |d| d.files.len())
        + app.git.get(&card.id).map_or(0, |g| g.dirty_count());
    let tools = super::detail::tool_calls(app, card).len();

    let mut row = w::Row::at(inner.x, inner.y);
    for (i, tab) in DetailTab::tabs(card.dispatch.is_some()).iter().enumerate() {
        if i > 0 {
            row.sep();
        }
        let count = match tab {
            DetailTab::Tasks if total > 0 => format!(" {done}/{total}"),
            DetailTab::Changes if changes > 0 => format!(" {changes}"),
            DetailTab::Tools if tools > 0 => format!(" {tools}"),
            DetailTab::Dispatch => super::dispatch_render::tab_count(app, card),
            _ => String::new(),
        };
        let style = if *tab == app.effective_tab() {
            styles::user().patch(styles::selected_row()).bold()
        } else {
            styles::muted()
        };
        let r = row.add(format!(" {}{count} ", tab.label()), style);
        w::push_hit(hits, r, inner, Hit::DetailTab(*tab));
    }
    row.render(frame, inner);

    if app.detail_tab == DetailTab::Tail {
        let text = if app.show_tools {
            " t tools on "
        } else {
            " t tools off "
        };
        let mut right = w::Row::at(
            inner.x + inner.width.saturating_sub(w::cells(text) as u16),
            inner.y,
        );
        let r = right.add(
            text,
            Style::default().fg(if app.show_tools {
                color::fg_muted()
            } else {
                color::fg_faint()
            }),
        );
        w::push_hit(hits, r, inner, Hit::Button(Button::ToggleTools));
        right.render(frame, inner);
    }
}

// ──────────────────────────────────────────────────────────────── attention

/// One row of the "this session is blocked on you" box.
pub(super) enum Att {
    Text(String, Color),
    Buttons(Vec<(String, Button, Color)>),
}

pub(super) fn attention_plan(snap: &LiveSnapshot, width: usize) -> Vec<Att> {
    let w_in = width.saturating_sub(4).max(8);
    let mut out = Vec::new();
    if let Some(prompt) = &snap.permission_prompt {
        for l in w::wrap(prompt, w_in).into_iter().take(8) {
            out.push(Att::Text(l, color::fg()));
        }
        out.push(Att::Buttons(vec![
            ("[ y  allow ]".into(), Button::Allow, color::ok()),
            ("[ n  deny ]".into(), Button::Deny, color::err()),
        ]));
        return out;
    }
    if let Some(q) = &snap.question {
        for l in w::wrap(q, w_in).into_iter().take(4) {
            out.push(Att::Text(l, color::fg()));
        }
        for (i, opt) in snap.question_options.iter().enumerate().take(6) {
            let desc = if opt.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", opt.description)
            };
            out.push(Att::Buttons(vec![(
                w::clip_cells(&format!("[ {} ] {}{desc}", i + 1, opt.label), w_in),
                Button::Answer(i + 1),
                color::user(),
            )]));
        }
        out.push(Att::Text("[ e ] type an answer".into(), color::fg_muted()));
    }
    out
}

pub(super) fn draw_attention(
    frame: &mut Frame,
    area: Rect,
    plan: &[Att],
    hits: &mut Vec<(Rect, Hit)>,
) {
    let inner = w::framed(
        frame,
        area,
        Some(Line::from(Span::styled(
            " needs you ",
            styles::err().bold(),
        ))),
        color::err(),
        true,
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    for (i, item) in plan.iter().take(inner.height as usize).enumerate() {
        let y = inner.y + i as u16;
        match item {
            Att::Text(t, color) => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        w::clip_cells(t, inner.width as usize),
                        Style::default().fg(*color),
                    ))),
                    Rect {
                        y,
                        height: 1,
                        ..inner
                    },
                );
            }
            Att::Buttons(buttons) => {
                let mut row = w::Row::at(inner.x, y);
                for (label, button, color) in buttons {
                    let r = row.add(
                        format!("{label}  "),
                        Style::default()
                            .fg(*color)
                            .patch(styles::selected_row())
                            .bold(),
                    );
                    w::push_hit(hits, r, inner, Hit::Button(*button));
                }
                row.render(frame, inner);
            }
        }
    }
}
