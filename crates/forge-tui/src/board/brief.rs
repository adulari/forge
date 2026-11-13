//! "While you were away": the overlay the board opens after a gap, and on `B`. The host turns the
//! daemon's `GET /api/brief` digest into this renderer-independent shape (ADR-0004), so the board
//! never parses the wire format or talks to the daemon itself.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::surface::{self, SurfaceTone};
use crate::theme::styles;

use super::state::BoardApp;
use super::widgets::{clip_cells, Fit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BriefTone {
    /// Finished.
    Good,
    /// Failed, or blocked on the person.
    Bad,
    /// Interrupted.
    Warn,
    /// Still working.
    Quiet,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefLine {
    pub session_id: String,
    pub text: String,
    /// A second, dimmer line: what the session last said.
    pub detail: String,
    pub tone: BriefTone,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefCard {
    pub title: String,
    pub headline: String,
    pub lines: Vec<BriefLine>,
}

pub(super) fn draw_brief(app: &BoardApp, frame: &mut Frame, area: Rect) {
    let Some(card) = app.brief.as_ref() else {
        return;
    };
    if area.width < 24 || area.height < 7 {
        return;
    }
    let width = area.width.saturating_sub(8).clamp(32, 110);
    let rows = card.lines.len() as u16 * 2;
    let height = (rows + 4).min(area.height.saturating_sub(2)).max(6);
    let rect = surface::modal_area(area, width, height);
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title(clip_cells(&card.title, 70), SurfaceTone::Brand),
        Some(surface::hint("↑↓ move · Enter open · Esc dismiss")),
        SurfaceTone::Brand,
    );
    if inner.width < 8 || inner.height < 2 {
        return;
    }
    let w = inner.width as usize;
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(clip_cells(&card.headline, w), styles::muted())),
        Line::default(),
    ];
    let room = (inner.height as usize).saturating_sub(2) / 2;
    let first = app.brief_cursor.saturating_sub(room.saturating_sub(1));
    for (i, line) in card.lines.iter().enumerate().skip(first).take(room) {
        let selected = i == app.brief_cursor;
        let tone = match line.tone {
            BriefTone::Good => styles::ok(),
            BriefTone::Bad => styles::err(),
            BriefTone::Warn => styles::warn(),
            BriefTone::Quiet => styles::muted(),
        };
        let mut fit = Fit::new(w);
        let row = if selected {
            styles::fg().patch(styles::selected_row())
        } else {
            styles::fg()
        };
        fit.add(if selected { "▸ " } else { "  " }, tone);
        fit.add(clip_cells(&line.text, w.saturating_sub(2)), row);
        lines.push(fit.line());
        lines.push(Line::from(Span::styled(
            format!("  {}", clip_cells(&line.detail, w.saturating_sub(2))),
            styles::muted(),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
