//! Shared visual primitives for Forge's interactive terminal surfaces.
//!
//! Every overlay and fullscreen view uses this module for the palette, rounded frame,
//! title treatment, and responsive geometry. Content remains local to its feature, while
//! the surrounding UI reads as one coherent Forge surface.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear};
use ratatui::Frame;

pub(crate) const ORANGE: Color = Color::Rgb(255, 138, 48);
pub(crate) const ACCENT: Color = Color::Rgb(82, 162, 255);
pub(crate) const DIM: Color = Color::Rgb(82, 87, 108);
pub(crate) const VERY_DIM: Color = Color::Rgb(54, 58, 74);
pub(crate) const TEXT: Color = Color::Rgb(208, 213, 224);
pub(crate) const USER: Color = Color::Rgb(122, 183, 255);
pub(crate) const OKGREEN: Color = Color::Rgb(92, 208, 122);
pub(crate) const ERRRED: Color = Color::Rgb(243, 92, 92);
pub(crate) const WARNYEL: Color = Color::Rgb(238, 188, 82);
pub(crate) const TOOLCYAN: Color = Color::Rgb(75, 212, 218);
pub(crate) const SELECT_BG: Color = Color::Rgb(40, 70, 132);
pub(crate) const STATUS_BG: Color = Color::Rgb(14, 15, 21);
pub(crate) const SURFACE_BG: Color = Color::Rgb(10, 12, 18);
pub(crate) const SEPARATOR: Color = Color::Rgb(38, 42, 62);

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
    pub(crate) const fn color(self) -> Color {
        match self {
            Self::Brand => ORANGE,
            Self::Accent => ACCENT,
            Self::Tool => TOOLCYAN,
            Self::Success => OKGREEN,
            Self::Warning => WARNYEL,
            Self::Danger => ERRRED,
        }
    }
}

pub(crate) fn title(text: impl Into<String>, tone: SurfaceTone) -> Span<'static> {
    Span::styled(
        format!(" {} ", text.into()),
        Style::default().fg(tone.color()).bold(),
    )
}

pub(crate) fn hint(text: impl Into<String>) -> Span<'static> {
    Span::styled(format!(" {} ", text.into()), Style::default().fg(DIM))
}

pub(crate) fn panel(title: Span<'static>, tone: SurfaceTone) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title)
        .style(Style::default().bg(SURFACE_BG))
        .border_style(Style::default().fg(tone.color()))
}

pub(crate) fn panel_with_footer(
    title: Span<'static>,
    footer: Span<'static>,
    tone: SurfaceTone,
) -> Block<'static> {
    panel(title, tone).title_bottom(footer)
}

/// Clear and frame a surface, returning the usable content rectangle.
pub(crate) fn render_panel(
    frame: &mut Frame,
    area: Rect,
    title: Span<'static>,
    footer: Option<Span<'static>>,
    tone: SurfaceTone,
) -> Rect {
    // Styling a Block only changes cell style; it does not replace existing glyphs from the chat
    // underneath. Clear first so every framed surface is genuinely opaque.
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(SURFACE_BG)),
        area,
    );
    let block = match footer {
        Some(footer) => panel_with_footer(title, footer, tone),
        None => panel(title, tone),
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Paint an opaque full-frame backdrop before rendering a fullscreen view. This prevents the
/// inline chat or startup art from leaking through alternate-screen help and configuration views.
pub(crate) fn render_backdrop(frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(Style::default().bg(STATUS_BG)), area);
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
        assert_ne!(SurfaceTone::Success.color(), SurfaceTone::Danger.color());
        assert_ne!(SurfaceTone::Warning.color(), SurfaceTone::Accent.color());
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
