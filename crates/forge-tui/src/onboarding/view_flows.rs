//! The pages around a flow in progress: the browser sign-in page, the session-start card and the
//! "set up everything recommended" checklist.

use ratatui::text::{Line, Span};

use super::screen::Screen;
use super::text::{blank, heading, paragraph, truncate};
use super::view_models::entry_field;
use crate::theme::glyphs::g;
use crate::theme::styles;

// ---------------------------------------------------------------- welcome

/// A URL has no spaces to wrap at, so cut it into rows of exactly `width` columns.
fn hard_wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(10);
    text.chars()
        .collect::<Vec<_>>()
        .chunks(width)
        .map(|c| c.iter().collect())
        .collect()
}

pub(super) fn sign_in(s: &Screen, width: usize, height: usize) -> Vec<Line<'static>> {
    let Some(state) = &s.sign_in else {
        return Vec::new();
    };
    let mut out = vec![heading("Sign in")];
    match &state.url {
        Some(url) => {
            out.extend(paragraph(
                "Open this page and sign in. Forge carries on by itself when you are done.",
                width,
                0,
                styles::muted(),
            ));
            out.push(blank());
            for row in hard_wrap(url, width) {
                out.push(Line::from(Span::styled(row, styles::brand())));
            }
            out.push(blank());
            let qr = super::qr::lines(url);
            let room = height.saturating_sub(out.len() + 3);
            match qr {
                Some(rows) if state.qr || rows.len() + 2 <= room => {
                    for row in rows {
                        out.push(Line::from(Span::styled(row, styles::fg())));
                    }
                }
                Some(_) => out.push(Line::from(Span::styled(
                    "On another device? Press v for a QR code to scan.",
                    styles::faint(),
                ))),
                None => {}
            }
        }
        None => out.extend(paragraph(
            "Finish the sign-in in your browser.",
            width,
            0,
            styles::muted(),
        )),
    }
    if let Some(entry) = &s.entry {
        out.push(blank());
        out.push(heading(&entry.label));
        if let Some(hint) = &entry.hint {
            out.push(Line::from(Span::styled(
                truncate(hint, width),
                styles::muted(),
            )));
        }
        out.push(entry_field(s, width));
    }
    out
}

pub(super) fn reconnect_card(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![heading("Connect Claude Code"), blank()];
    out.extend(paragraph(
        "Forge now uses a token of its own for Claude Code, so it never touches your main Claude login. One click reconnects it: Forge runs the sign-in, keeps the token in your keyring, and checks it.",
        width,
        0,
        styles::muted(),
    ));
    out.push(blank());
    if s.running(&super::screen::Purpose::ConnectClaude) {
        out.push(Line::from(Span::styled(
            format!("{} Connecting...", g().spin(s.tick)),
            styles::spinner(s.tick),
        )));
    }
    out
}

pub(super) fn batch_checklist(s: &Screen, width: usize) -> Vec<Line<'static>> {
    use super::batch::BatchStatus;
    let mut out = vec![heading("Setting everything up"), blank()];
    let Some(batch) = &s.batch else {
        return out;
    };
    for item in &batch.items {
        let (glyph, style, tail) = match &item.status {
            BatchStatus::Done => (g().ok.to_string(), styles::ok(), String::new()),
            BatchStatus::Running => (
                g().spin(s.tick).to_string(),
                styles::spinner(s.tick),
                String::new(),
            ),
            BatchStatus::Pending if item.rec.action.is_none() => (
                g().task_open.to_string(),
                styles::faint(),
                "  you choose".to_string(),
            ),
            BatchStatus::Pending => (g().task_open.to_string(), styles::faint(), String::new()),
            BatchStatus::Failed(why) => (g().fail.to_string(), styles::err(), format!("  {why}")),
        };
        let line = truncate(
            &format!("{}{tail}", item.rec.title),
            width.saturating_sub(4),
        );
        out.push(Line::from(vec![
            Span::styled(format!("  {glyph} "), style),
            Span::styled(line, styles::fg()),
        ]));
    }
    out
}
