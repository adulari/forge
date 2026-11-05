//! The why-trace detail card: which agent turn wrote the lines under the cursor, what it was
//! asked, and the tool call, with a way to jump to that turn in the timeline.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::board::model_short;
use crate::ide::state::IdeApp;
use crate::surface::{self, SurfaceTone};
use crate::theme::glyphs::g;
use crate::theme::styles;

fn when(now: i64, created: i64) -> String {
    let created = if created > 1_000_000_000_000 {
        created / 1000
    } else {
        created
    };
    let secs = (now - created).max(0);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

pub(super) fn draw(app: &IdeApp, frame: &mut Frame, area: Rect) {
    let Some(popup) = &app.tools.why_popup else {
        return;
    };
    let Some(entry) = popup.entries.get(popup.index) else {
        return;
    };
    let width = area.width.saturating_sub(4).min(76);
    let height = 12u16.min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let title = format!(
        "Why-trace {} {}/{}",
        g().sep,
        popup.index + 1,
        popup.entries.len()
    );
    let block = surface::panel(
        surface::title(&title, SurfaceTone::Brand),
        SurfaceTone::Brand,
    );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let agent = app
        .agents
        .iter()
        .find(|a| a.id == entry.session_id)
        .map_or_else(
            || entry.session_id.chars().take(8).collect(),
            |a| a.title.clone(),
        );
    let model = entry.model.as_deref().map(model_short).unwrap_or_default();
    let label = |text: &str| Span::styled(format!(" {text:<9}"), styles::faint());
    let mut lines = vec![
        Line::from(vec![
            label("agent"),
            Span::styled(agent, styles::fg().bold()),
        ]),
        Line::from(vec![
            label("turn"),
            Span::styled(
                format!(
                    "{}{}{}",
                    entry.turn,
                    if model.is_empty() {
                        String::new()
                    } else {
                        format!(" {} {model}", g().sep)
                    },
                    entry
                        .created_at
                        .map(|t| format!(" {} {}", g().sep, when(app.now, t)))
                        .unwrap_or_default()
                ),
                styles::fg(),
            ),
        ]),
        Line::from(vec![
            label("prompt"),
            Span::styled(
                entry
                    .prompt_summary
                    .clone()
                    .filter(|p| !p.trim().is_empty())
                    .unwrap_or_else(|| "(not recorded)".to_string()),
                styles::muted(),
            ),
        ]),
    ];
    if let Some(call) = &entry.tool_call_id {
        lines.push(Line::from(vec![
            label("tool call"),
            Span::styled(call.clone(), styles::faint()),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    let hint = Rect {
        y: inner.bottom().saturating_sub(1),
        height: 1,
        ..inner
    };
    let mut spans = vec![Span::raw(" ")];
    spans.extend(styles::hint_row(&[
        ("j/k", "entry"),
        ("Enter", "jump to turn"),
        ("Esc", "close"),
    ]));
    frame.render_widget(Paragraph::new(Line::from(spans)), hint);
}
