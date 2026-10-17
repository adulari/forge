//! A framed overlay with uniform padding inside the frame (1 row, 2 cells). The padding is
//! marked as decoration so a drag across the overlay copies its text, not its margins.

use super::*;

/// Padding inside the frame: rows top and bottom, cells left and right.
pub(in crate::app::render) const PAD_Y: u16 = 1;
pub(in crate::app::render) const PAD_X: u16 = 2;

/// Clear and frame `area` and return the padded body rectangle.
pub(in crate::app::render) fn padded_panel(
    f: &mut Frame,
    area: Rect,
    title: Span<'static>,
    context: Option<Span<'static>>,
    footer: Option<Span<'static>>,
    tone: surface::SurfaceTone,
) -> Rect {
    let inner = surface::render_panel_with_context(f, area, title, context, footer, tone);
    let decor = crate::copy_mark::decor(Style::default());
    for pad in [
        Rect {
            height: PAD_Y.min(inner.height),
            ..inner
        },
        Rect {
            y: inner.y + inner.height.saturating_sub(PAD_Y),
            height: PAD_Y.min(inner.height),
            ..inner
        },
        Rect {
            width: PAD_X.min(inner.width),
            ..inner
        },
        Rect {
            x: inner.x + inner.width.saturating_sub(PAD_X),
            width: PAD_X.min(inner.width),
            ..inner
        },
    ] {
        f.buffer_mut().set_style(pad, decor);
    }
    Rect {
        x: inner.x + PAD_X,
        y: inner.y + PAD_Y,
        width: inner.width.saturating_sub(PAD_X * 2),
        height: inner.height.saturating_sub(PAD_Y * 2),
    }
}
