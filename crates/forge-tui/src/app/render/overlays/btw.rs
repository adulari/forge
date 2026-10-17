//! The `/btw` side-question card: the question in bold, the answer markdown-rendered and
//! scrollable, and where it came from in the footer border. Never written to the scrollback.

use super::*;
use crate::theme::glyphs::g;

/// Widest the card grows, border included.
const MAX_WIDTH: u16 = 84;
use super::pad::{PAD_X, PAD_Y};

/// Rows a line takes when wrapped to `width` cells.
fn wrapped_rows(line: &TextLine<'_>, width: usize) -> usize {
    (line.width().max(1)).div_ceil(width.max(1))
}

/// Remove the indent the transcript's markdown renderer gives every line, keeping relative
/// indentation (nested lists, code) intact. The card has its own padding.
fn dedent(mut lines: Vec<TextLine<'static>>) -> Vec<TextLine<'static>> {
    let lead = |line: &TextLine<'_>| -> Option<usize> {
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        (!text.trim().is_empty()).then(|| text.len() - text.trim_start_matches(' ').len())
    };
    let strip = lines.iter().filter_map(lead).min().unwrap_or(0);
    if strip == 0 {
        return lines;
    }
    for line in &mut lines {
        let mut left = strip;
        for span in &mut line.spans {
            if left == 0 {
                break;
            }
            let take = span
                .content
                .chars()
                .take(left)
                .take_while(|c| *c == ' ')
                .count();
            span.content = span.content.chars().skip(take).collect::<String>().into();
            left -= take;
            if !span.content.is_empty() {
                break;
            }
        }
    }
    lines
}

/// The card body: question, blank row, then the answer, or the spinner row while it runs.
fn body_lines(card: &crate::app_btw::BtwOverlay, app: &App) -> Vec<TextLine<'static>> {
    let mut lines = vec![
        TextLine::from(Span::styled(card.question.clone(), styles::fg().bold())),
        TextLine::default(),
    ];
    match &card.answer {
        None => {
            let model =
                crate::app::render::model_short(app.routing.as_ref().map(|r| r.model.as_str()));
            lines.push(TextLine::from(vec![
                Span::styled(
                    format!("{} ", g().spin(app.tick)),
                    styles::spinner(app.tick),
                ),
                Span::styled(format!("asking {model}{}", g().ellipsis), styles::muted()),
            ]));
        }
        Some(answer) if card.failed => lines.push(TextLine::from(vec![
            Span::styled(format!("{} ", g().warn), styles::warn()),
            Span::styled(answer.clone(), styles::warn()),
        ])),
        Some(answer) => lines.extend(dedent(crate::render::markdown_to_lines(answer))),
    }
    lines
}

/// Draw the `/btw` card over the live view, centred, at most [`MAX_WIDTH`] wide.
pub fn render_btw_overlay(f: &mut Frame, app: &App) {
    let Some(card) = &app.btw else {
        return;
    };
    let area = f.area();
    let width = area
        .width
        .saturating_sub(4)
        .clamp(10, MAX_WIDTH)
        .min(area.width);
    let text_w = width.saturating_sub(2 + PAD_X * 2).max(1) as usize;
    let lines = body_lines(card, app);
    let rows: usize = lines.iter().map(|l| wrapped_rows(l, text_w)).sum();
    let height = (rows as u16)
        .saturating_add(2 + PAD_Y * 2)
        .min(area.height.saturating_sub(4).max(5));
    let card_area = surface::modal_area(area, width, height);

    let provenance = if card.meta.is_empty() {
        String::new()
    } else {
        format!("{} {} ", card.meta, g().sep)
    };
    let keys = if card.answer.is_some() {
        format!("{}scroll {} Esc close", g().up_down, g().sep)
    } else {
        "Esc close".to_string()
    };
    let body = super::pad::padded_panel(
        f,
        card_area,
        surface::title("btw", surface::SurfaceTone::Accent),
        Some(Span::styled(
            format!(" side question {} not saved ", g().sep),
            styles::faint(),
        )),
        Some(Span::styled(
            format!(" {provenance}{keys} "),
            styles::faint(),
        )),
        surface::SurfaceTone::Accent,
    );
    if body.width == 0 || body.height == 0 {
        return;
    }
    let max_scroll = (rows as u16).saturating_sub(body.height);
    card.max_scroll.set(max_scroll);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((card.scroll.min(max_scroll), 0)),
        body,
    );
}
