//! The `?` help overlay: every IDE key, one line each.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::ide::keys::HELP;
use crate::surface::{self, SurfaceTone};
use crate::theme::styles;

pub(super) fn draw(frame: &mut Frame, area: Rect) {
    let width = area.width.saturating_sub(4).min(96);
    let height = (HELP.len() as u16 + 4).min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let block = surface::panel(
        surface::title("IDE keys", SurfaceTone::Brand),
        SurfaceTone::Brand,
    );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let key_w = HELP
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(8)
        .min(24);
    let lines: Vec<Line> = HELP
        .iter()
        .take(usize::from(inner.height).saturating_sub(1))
        .map(|(keys, what)| {
            Line::from(vec![
                Span::styled(format!(" {keys:<key_w$}  "), styles::key_chip()),
                Span::styled((*what).to_string(), styles::muted()),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
    let hint = Rect {
        y: inner.bottom().saturating_sub(1),
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(" any key closes", styles::faint()))),
        hint,
    );
}
