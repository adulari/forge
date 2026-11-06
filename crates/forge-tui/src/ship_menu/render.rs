//! Draws `/ship`: a centered card with the four progress rows and the headline.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{headline, rows, RowState, ShipMenu};
use crate::app::App;
use crate::surface::{self, SurfaceTone};
use crate::theme::{glyphs::g, styles};

fn trunc(text: &str, max: usize) -> String {
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if UnicodeWidthStr::width(out.as_str()) + 2 > max {
            break;
        }
        out.push(c);
    }
    out.push_str(g().ellipsis);
    out
}

fn footer(m: &ShipMenu) -> String {
    let sep = g().sep;
    if m.running {
        " Shipping ".to_string()
    } else if m.succeeded() {
        format!(" Enter done {sep} Esc close ")
    } else if m.outcome.is_some() || m.error.is_some() {
        format!(" Enter retry {sep} Esc close ")
    } else {
        format!(" Enter ship {sep} Esc close ")
    }
}

pub(crate) fn render_ship_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.ship_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 100);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let want = 9u16;
    let padded = want + 2 + 2 * crate::overlays::PAD_Y;
    let card_h = if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        want + 2
    }
    .min(region.height);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("Ship to a merged PR", SurfaceTone::Brand),
        None,
        Some(Span::styled(footer(m), styles::faint())),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 6 {
        return;
    }
    let head = match &m.error {
        Some(e) => (format!("{} {e}", g().fail), styles::err()),
        None => {
            let refused = m.outcome.as_ref().is_some_and(|o| !o.ok);
            (
                headline(m.outcome.as_ref(), m.running),
                if refused {
                    styles::err()
                } else {
                    styles::muted()
                },
            )
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(trunc(&head.0, inner_w), head.1))),
        Rect { height: 1, ..inner },
    );
    let lines: Vec<Line> = rows(m.outcome.as_ref(), m.running)
        .into_iter()
        .map(|row| {
            let (glyph, style) = match row.state {
                RowState::Pending | RowState::Blocked => (g().sep, styles::faint()),
                RowState::Active => (g().spin(m.tick as usize), styles::brand()),
                RowState::Done => (g().ok, styles::ok()),
                RowState::Skipped => (g().sep, styles::muted()),
                RowState::Refused => (g().fail, styles::err()),
            };
            let label_w = 20;
            let detail_w = inner_w.saturating_sub(label_w + 3);
            Line::from(vec![
                Span::styled(format!("{glyph} "), style),
                Span::styled(
                    format!("{:<label_w$}", row.label),
                    if matches!(row.state, RowState::Blocked | RowState::Pending) {
                        styles::faint()
                    } else {
                        styles::fg()
                    },
                ),
                Span::styled(trunc(&row.detail, detail_w), styles::muted()),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            y: inner.y + 2,
            height: 4,
            ..inner
        },
    );
}
