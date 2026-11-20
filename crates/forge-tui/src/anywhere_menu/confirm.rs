//! The confirmations shown before key sync is switched on, before a device is revoked and before
//! ownership moves to another device.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::render::{card_rows, width};
use crate::surface::{self, SurfaceTone};
use crate::theme::styles;

pub(super) fn render_keys_confirm(f: &mut Frame, card: Rect, on_confirm: bool) {
    let paragraphs: [(&str, &str); 5] = [
        ("", "Your provider and search API keys and static MCP secrets will be uploaded and delivered to every device on this account that also turns key sync on."),
        ("- ", "Sealed with a second vault key, on top of end-to-end encryption."),
        ("- ", "OAuth sign-ins (Codex, xAI, MCP) never sync; other devices get a sign-in prompt instead."),
        ("- ", "A revoked device keeps what it already downloaded, so rotate keys at the provider after revoking one."),
        ("- ", "Secret values are never shown on screen."),
    ];
    render_confirm(
        f,
        card,
        &paragraphs,
        ("Turn on key sync", on_confirm),
        "Sync API keys across your machines?".into(),
    );
}

pub(super) fn render_device_confirm(f: &mut Frame, card: Rect, pending: &super::DeviceConfirm) {
    match pending.kind {
        super::DeviceConfirmKind::Revoke => {
            render_revoke_confirm(f, card, &pending.name, pending.on_confirm)
        }
        super::DeviceConfirmKind::MakeOwner => {
            render_make_owner_confirm(f, card, &pending.name, pending.on_confirm)
        }
    }
}

fn render_revoke_confirm(f: &mut Frame, card: Rect, name: &str, on_confirm: bool) {
    let paragraphs: [(&str, &str); 4] = [
        ("", "The device loses access at once: its sign-in, its host grants and its push subscription are revoked."),
        ("- ", "Forge rotates the account key and re-wraps it to your remaining devices and your Recovery Kit, so the device cannot read anything new."),
        ("- ", "Forge asks for your Recovery Kit next, in this terminal. It is never shown on screen."),
        ("- ", "A revoked device cannot be restored; pair it again to use it."),
    ];
    render_confirm(
        f,
        card,
        &paragraphs,
        ("Revoke device", on_confirm),
        format!("Revoke {name}?"),
    );
}

fn render_make_owner_confirm(f: &mut Frame, card: Rect, name: &str, on_confirm: bool) {
    let paragraphs: [(&str, &str); 3] = [
        ("", "Only the owner device can revoke the owner, or hand ownership to another device."),
        ("- ", "This device stops being the owner. It can still revoke other devices, but no longer the new owner."),
        ("- ", "Only the new owner can make this device the owner again."),
    ];
    render_confirm(
        f,
        card,
        &paragraphs,
        ("Make owner", on_confirm),
        format!("Make {name} the owner?"),
    );
}

fn render_confirm(
    f: &mut Frame,
    card: Rect,
    paragraphs: &[(&str, &str)],
    (primary, on_confirm): (&str, bool),
    title: String,
) {
    let w = card.width.saturating_sub(4).min(76);
    let inner_w = (w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let mut lines: Vec<Line> = Vec::new();
    for &(bullet, text) in paragraphs {
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
        button(primary, on_confirm, styles::warn()),
        Span::raw("   "),
        button("Cancel", !on_confirm, styles::fg()),
    ]));
    let h = card_rows(lines.len() as u16);
    let area = surface::modal_area(card, w, h);
    let inner = crate::overlays::padded_panel(
        f,
        area,
        surface::title(title, SurfaceTone::Warning),
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
