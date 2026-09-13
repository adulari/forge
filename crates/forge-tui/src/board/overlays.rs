//! The board's modal overlays: the one-line composer, the confirmation dialog and the key help.
//! Layout-only, like `widgets.rs`, which they share their primitives with.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::surface::{self, SurfaceTone, ACCENT, DIM, SELECT_BG, TEXT};

use super::keys::HELP;
use super::state::{BoardApp, ComposerMode};
use super::widgets::{clip_cells, wrap, Fit};

/// The one-line composer: label in the frame, the text with a real cursor, the send/cancel hint.
pub(super) fn draw_composer(app: &BoardApp, frame: &mut Frame, area: Rect) {
    let Some(c) = app.composer.as_ref() else {
        return;
    };
    if area.width < 12 || area.height < 5 {
        return;
    }
    let rect = surface::modal_area(area, area.width.saturating_sub(8).clamp(24, 96), 5);
    let hint = match &c.mode {
        ComposerMode::NewSession { .. } => "Enter send · Esc cancel · Tab: worktree on/off",
        _ => "Enter send · Esc cancel",
    };
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title(clip_cells(&c.label(), 60), SurfaceTone::Accent),
        Some(surface::hint(hint)),
        SurfaceTone::Accent,
    );
    if inner.width < 2 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let chars: Vec<char> = c.text.chars().collect();
    let cursor = c.cursor.min(chars.len());
    let start = (cursor + 1).saturating_sub(width);
    let before: String = chars[start..cursor].iter().collect();
    let (at, after) = match chars.get(cursor) {
        Some(ch) => (
            ch.to_string(),
            chars[(cursor + 1).min(chars.len())..].iter().collect(),
        ),
        None => (" ".to_string(), String::new()),
    };
    let mut fit = Fit::new(width);
    fit.add(before, Style::default().fg(TEXT));
    fit.add(at, Style::default().fg(TEXT).bg(SELECT_BG));
    fit.add(after, Style::default().fg(TEXT));
    frame.render_widget(Paragraph::new(fit.line()), inner);
}

/// A confirmation: what it will do, then yes/no. Irreversible actions get the danger tone.
pub(super) fn draw_confirm(app: &BoardApp, frame: &mut Frame, area: Rect) {
    let Some(c) = app.confirm.as_ref() else {
        return;
    };
    if area.width < 16 || area.height < 6 {
        return;
    }
    let tone = if c.kind.is_danger() {
        SurfaceTone::Danger
    } else {
        SurfaceTone::Warning
    };
    let width = area.width.saturating_sub(10).clamp(24, 68);
    let body = wrap(&c.body, width.saturating_sub(2) as usize);
    let height = (body.len() as u16 + 3)
        .min(area.height.saturating_sub(2))
        .max(4);
    let rect = surface::modal_area(area, width, height);
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title(clip_cells(&c.title, 60), tone),
        Some(surface::hint("Enter yes · Esc no")),
        tone,
    );
    let lines: Vec<Line> = body
        .into_iter()
        .map(|l| Line::from(Span::styled(l, Style::default().fg(TEXT))))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Every key the board answers to, in two columns, straight out of [`HELP`].
pub(super) fn draw_help(frame: &mut Frame, area: Rect) {
    if area.width < 20 || area.height < 6 {
        return;
    }
    // Two columns only when each gets room for the longest description; otherwise one column,
    // clipped to the height rather than clipping the words.
    let width = area.width.saturating_sub(6).clamp(24, 150);
    let two_columns = width >= 130;
    let rows = if two_columns {
        HELP.len().div_ceil(2)
    } else {
        HELP.len()
    };
    let height = (rows as u16 + 2).min(area.height.saturating_sub(2)).max(4);
    let rect = surface::modal_area(area, width, height);
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title("forge board — keys", SurfaceTone::Brand),
        Some(surface::hint("any key closes")),
        SurfaceTone::Brand,
    );
    let half = inner.width as usize / 2;
    let mut lines: Vec<Line> = Vec::new();
    for i in 0..rows {
        let mut fit = Fit::new(inner.width as usize);
        if two_columns {
            help_cell(&mut fit, HELP.get(i), half);
            help_cell(&mut fit, HELP.get(i + rows), inner.width as usize - half);
        } else {
            help_cell(&mut fit, HELP.get(i), inner.width as usize);
        }
        lines.push(fit.line());
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn help_cell(fit: &mut Fit, entry: Option<&(&'static str, &'static str)>, width: usize) {
    let Some((key, desc)) = entry else {
        return;
    };
    let before = fit.used();
    fit.add(format!("{key:>11}  "), Style::default().fg(ACCENT).bold());
    fit.add(*desc, Style::default().fg(DIM));
    let used = fit.used() - before;
    if used < width {
        fit.add(" ".repeat(width - used), Style::default());
    }
}
