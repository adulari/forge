//! Shared visual primitives for Forge's interactive terminal surfaces.
//!
//! Every overlay and fullscreen view uses this module for the palette, rounded frame,
//! title treatment, and responsive geometry. Content remains local to its feature, while
//! the surrounding UI reads as one coherent Forge surface.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear};
use ratatui::Frame;

use crate::theme::{glyphs::g, styles};

/// The semantic purpose of a framed surface. This keeps border color meaningful and consistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceTone {
    Brand,
    Accent,
    Tool,
    Success,
    Warning,
    Danger,
}

impl SurfaceTone {
    /// The tone's full style (colour plus the modifiers that carry it in mono).
    pub(crate) fn style(self) -> Style {
        styles::tone_token(self).style()
    }
}

pub(crate) fn title(text: impl Into<String>, tone: SurfaceTone) -> Span<'static> {
    Span::styled(format!(" {} ", text.into()), tone.style().bold())
}

pub(crate) fn hint(text: impl Into<String>) -> Span<'static> {
    Span::styled(format!(" {} ", text.into()), styles::muted())
}

fn border_style_for(tone: SurfaceTone) -> Style {
    styles::frame(
        tone,
        matches!(tone, SurfaceTone::Warning | SurfaceTone::Danger),
    )
}

/// A framed surface: the title sits in the top border after one `─`.
pub(crate) fn panel(title: Span<'static>, tone: SurfaceTone) -> Block<'static> {
    let border_style = border_style_for(tone);
    Block::bordered()
        .border_set(g().border)
        .title(Line::from(vec![
            Span::styled(g().border.horizontal_top, border_style),
            title,
        ]))
        .style(styles::surface())
        .border_style(border_style)
}

/// A framed surface with a footer right-aligned in the bottom border, closed by one `─`.
pub(crate) fn panel_with_footer(
    title: Span<'static>,
    footer: Span<'static>,
    tone: SurfaceTone,
) -> Block<'static> {
    let border_style = border_style_for(tone);
    panel(title, tone).title_bottom(
        Line::from(vec![
            footer,
            Span::styled(g().border.horizontal_bottom, border_style),
        ])
        .right_aligned(),
    )
}

thread_local! {
    static PANELS: std::cell::RefCell<Vec<Rect>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Rectangles of every framed surface drawn since the last call, bottom-most first. The draw
/// pass is single-threaded, so a thread-local avoids threading a recorder through every overlay.
pub(crate) fn take_panels() -> Vec<Rect> {
    PANELS.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Record a framed surface drawn outside this module (the shared list view) so panel-aware
/// consumers of [`take_panels`] see it.
pub(crate) fn note_panel(area: Rect) {
    PANELS.with(|p| p.borrow_mut().push(area));
}

/// Clear and frame a surface, returning the usable content rectangle.
pub(crate) fn render_panel(
    frame: &mut Frame,
    area: Rect,
    title: Span<'static>,
    footer: Option<Span<'static>>,
    tone: SurfaceTone,
) -> Rect {
    render_panel_with_context(frame, area, title, None, footer, tone)
}

/// [`render_panel`] with a `context` (a count or the command that opened it) right-aligned in
/// the top border.
pub(crate) fn render_panel_with_context(
    frame: &mut Frame,
    area: Rect,
    title: Span<'static>,
    context: Option<Span<'static>>,
    footer: Option<Span<'static>>,
    tone: SurfaceTone,
) -> Rect {
    // Styling a Block only changes cell style; it does not replace existing glyphs from the chat
    // underneath. Clear first so every framed surface is genuinely opaque.
    PANELS.with(|p| p.borrow_mut().push(area));
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(styles::surface()), area);
    let mut block = match footer {
        Some(footer) => panel_with_footer(title, footer, tone),
        None => panel(title, tone),
    };
    if let Some(context) = context {
        let border_style = border_style_for(tone);
        block = block.title(
            Line::from(vec![
                context,
                Span::styled(g().border.horizontal_top, border_style),
            ])
            .right_aligned(),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Clear the frame before rendering a fullscreen view so nothing from the chat leaks through
/// alternate-screen help and configuration views. The terminal background is never painted.
pub(crate) fn render_backdrop(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
}

/// Re-colour everything already drawn in `area` as faint, so the modal drawn next reads as the
/// foreground. Cells keep their glyphs and modifiers; fills are dropped.
pub(crate) fn dim_backdrop(frame: &mut Frame, area: Rect) {
    let faint = styles::faint();
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(x, y)) {
                cell.set_style(Style::reset().patch(faint));
                // set_style patches; the symbol stays as drawn.
            }
        }
    }
}

/// A responsive centered dialog with a modest margin on terminals that have room for it.
pub(crate) fn modal_area(area: Rect, preferred_width: u16, preferred_height: u16) -> Rect {
    let width = preferred_width.min(area.width.saturating_sub(2)).max(1);
    let height = preferred_height.min(area.height.saturating_sub(2)).max(1);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// The standard, readable size for data-heavy inspectors such as usage and routing.
/// `content_rows` excludes the two border rows; the cap keeps a long inspector navigable rather
/// than turning it into a near-fullscreen wall of text.
pub(crate) fn inspector_area(area: Rect, content_rows: u16) -> Rect {
    modal_area(
        area,
        area.width.saturating_sub(8).min(108),
        content_rows.clamp(4, 30).saturating_add(2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;

    #[test]
    fn modal_respects_small_terminals() {
        assert_eq!(
            modal_area(Rect::new(0, 0, 20, 8), 92, 18),
            Rect::new(1, 1, 18, 6)
        );
    }

    #[test]
    fn inspector_is_centered_with_room_for_the_underlying_context() {
        assert_eq!(
            inspector_area(Rect::new(3, 4, 40, 20), 12),
            Rect::new(7, 7, 32, 14)
        );
    }

    #[test]
    fn tone_colors_are_distinct_for_status_states() {
        assert_ne!(SurfaceTone::Success.style(), SurfaceTone::Danger.style());
        assert_ne!(SurfaceTone::Warning.style(), SurfaceTone::Accent.style());
    }

    #[test]
    fn framed_surface_replaces_glyphs_underneath_it() {
        let backend = TestBackend::new(12, 5);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                frame.render_widget(Paragraph::new("underlying text"), area);
                render_panel(
                    frame,
                    Rect::new(1, 1, 10, 3),
                    title("Modal", SurfaceTone::Accent),
                    None,
                    SurfaceTone::Accent,
                );
            })
            .expect("test draw");

        assert_eq!(terminal.backend().buffer()[(2, 2)].symbol(), " ");
    }
}
