//! The permanent session-delete confirmation (`/delete`, Ctrl+D in the sessions picker).

use ratatui::text::{Line, Span};
use ratatui::widgets::Wrap;

use super::*;
use crate::session_delete::SessionDeleteDialog;
use crate::theme::glyphs::g;

fn dialog_lines(d: &SessionDeleteDialog) -> Vec<Line<'static>> {
    let name = if d.title.trim().is_empty() {
        "(untitled)".to_string()
    } else {
        format!("\"{}\"", d.title.trim())
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            name,
            styles::fg().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(format!("  {}", d.short_id()), styles::faint()),
    ])];
    lines.push(Line::default());
    if let Some((why, next)) = &d.refusal {
        lines.push(Line::from(Span::styled(why.clone(), styles::warn())));
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("Instead: ", styles::muted()),
            Span::styled(next.clone(), styles::fg()),
        ]));
        return lines;
    }
    lines.push(Line::from(Span::styled(
        "This permanently removes:",
        styles::err(),
    )));
    for fact in &d.facts {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", g().sep), styles::faint()),
            Span::styled(fact.clone(), styles::fg()),
        ]));
    }
    for kept in &d.kept {
        lines.push(Line::from(Span::styled(
            format!("Kept: {kept}"),
            styles::muted(),
        )));
    }
    lines.push(Line::from(Span::styled(
        "It cannot be undone.",
        styles::err().add_modifier(ratatui::style::Modifier::BOLD),
    )));
    lines.push(Line::default());
    let short = d.short_id();
    lines.push(Line::from(vec![
        Span::styled(format!("Type {short} to confirm: "), styles::muted()),
        Span::styled(d.typed.clone(), styles::fg()),
        Span::styled("▏", styles::brand()),
    ]));
    lines
}

pub fn render_session_delete_overlay(f: &mut Frame, app: &App) {
    let Some(d) = &app.session_delete else {
        return;
    };
    let area = f.area();
    surface::dim_backdrop(f, area);
    let width = area.width.saturating_sub(2).clamp(10, 76).min(area.width);
    let text_w = width.saturating_sub(2 + crate::overlays::PAD_X * 2).max(1) as usize;
    let lines = dialog_lines(d);
    let wrapped: usize = lines
        .iter()
        .map(|l| (l.width().max(1)).div_ceil(text_w))
        .sum();
    let height = (wrapped as u16)
        .saturating_add(2 + crate::overlays::pad_y(area.height) * 2)
        .min(area.height);
    let card = crate::overlays::card_area(area, width, height);
    let refused = d.refusal.is_some();
    let tone = if refused {
        surface::SurfaceTone::Warning
    } else {
        surface::SurfaceTone::Danger
    };
    let title = if refused {
        "Cannot delete this session"
    } else {
        "Delete this session forever?"
    };
    let footer = if refused {
        " Esc close "
    } else {
        " Enter delete (after typing the id) · Esc cancel "
    };
    let body = crate::overlays::padded_panel(
        f,
        card,
        surface::title(title.to_string(), tone),
        Some(Span::styled(" /delete ", styles::faint())),
        Some(Span::styled(footer, styles::faint())),
        tone,
    );
    if body.width < 10 || body.height < 3 {
        return;
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}
