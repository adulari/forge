//! The confirmation shown before key sync is switched on.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::render::{card_rows, width};
use crate::surface::{self, SurfaceTone};
use crate::theme::styles;

pub(super) fn render_keys_confirm(f: &mut Frame, card: Rect, on_confirm: bool) {
    let w = card.width.saturating_sub(4).min(76);
    let paragraphs: [(&str, &str); 5] = [
        ("", "Your provider and search API keys and static MCP secrets will be uploaded and delivered to every device on this account that also turns key sync on."),
        ("- ", "Sealed with a second vault key, on top of end-to-end encryption."),
        ("- ", "OAuth sign-ins (Codex, xAI, MCP) never sync; other devices get a sign-in prompt instead."),
        ("- ", "A revoked device keeps what it already downloaded, so rotate keys at the provider after revoking one."),
        ("- ", "Secret values are never shown on screen."),
    ];
    let inner_w = (w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let mut lines: Vec<Line> = Vec::new();
    for (bullet, text) in paragraphs {
        let indent = bullet.len();
        for (i, part) in wrap(text, inner_w.saturating_sub(indent).max(10))
            .into_iter()
            .enumerate()
        {
            let lead = if i == 0 {
                bullet.to_string()
            } else {
                " ".repeat(indent)
            };
            lines.push(Line::from(vec![
                Span::styled(lead, styles::faint()),
                Span::styled(part, styles::muted()),
            ]));
        }
        if bullet.is_empty() {
            lines.push(Line::default());
        }
    }
    lines.push(Line::default());
    let button = |label: &str, active: bool, style: Style| {
        let s = if active {
            style.add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            styles::muted()
        };
        Span::styled(format!(" {label} "), s)
    };
    lines.push(Line::from(vec![
        button("Turn on key sync", on_confirm, styles::warn()),
        Span::raw("   "),
        button("Cancel", !on_confirm, styles::fg()),
    ]));
    let h = card_rows(lines.len() as u16);
    let area = surface::modal_area(card, w, h);
    let inner = crate::overlays::padded_panel(
        f,
        area,
        surface::title("Sync API keys across your machines?", SurfaceTone::Warning),
        None,
        Some(Span::styled(
            " y confirm · Esc cancel ".to_string(),
            styles::faint(),
        )),
        SurfaceTone::Warning,
    );
    f.render_widget(Paragraph::new(lines), inner);
}

fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && width(&line) + 1 + width(word) > max {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}
