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
    use super::qr::{fitted, Qr};
    let Some(state) = &s.sign_in else {
        return Vec::new();
    };
    let Some(url) = &state.url else {
        let mut out = vec![heading("Sign in")];
        out.extend(paragraph(
            "Finish the sign-in in your browser.",
            width,
            0,
            styles::muted(),
        ));
        push_entry(s, width, &mut out);
        return out;
    };
    let code = state.code.is_some();
    if state.qr {
        // The QR view gives the code the whole page: no link, no log.
        let mut out = vec![heading("Scan to sign in")];
        let how = if code {
            "Scan with your phone's camera, sign in there, then paste the code it shows."
        } else {
            "Scan with your phone's camera and sign in there. Press v for the link."
        };
        out.push(Line::from(Span::styled(
            truncate(how, width),
            styles::muted(),
        )));
        push_entry(s, width, &mut out);
        out.push(blank());
        let room = height.saturating_sub(out.len());
        match fitted(url, room, width, s.rich_glyphs) {
            Some(Qr::Fits(rows)) => out.extend(
                rows.into_iter()
                    .map(|row| Line::from(Span::styled(row, styles::fg()))),
            ),
            Some(Qr::TooBig { rows, cols }) => {
                let taller = rows.saturating_sub(room);
                let wider = cols.saturating_sub(width);
                let grow = match (taller, wider) {
                    (0, w) => format!("{w} columns wider"),
                    (t, 0) => format!("{t} rows taller"),
                    (t, w) => format!("{t} rows taller and {w} columns wider"),
                };
                out.extend(paragraph(
                    &format!("The QR code needs a bigger window: make it {grow}, or press v and use the link."),
                    width,
                    0,
                    styles::faint(),
                ));
            }
            None => out.push(Line::from(Span::styled(
                "This link is too long for a QR code. Press v and use the link.",
                styles::faint(),
            ))),
        }
        return out;
    }
    let mut out = vec![heading("Sign in")];
    let how = match (s.can_open_browser, code) {
        (true, true) => "Open this page and sign in, then paste the code it shows.",
        (true, false) => "Open this page and sign in. Forge carries on by itself when you are done.",
        (false, true) => "Click the link or press c to copy it, sign in on any device, then paste the code it shows.",
        (false, false) => "Click the link or press c to copy it, then sign in on any device.",
    };
    out.extend(paragraph(how, width, 0, styles::muted()));
    out.push(blank());
    for row in hard_wrap(url, width) {
        out.push(Line::from(Span::styled(row, styles::brand())));
    }
    push_entry(s, width, &mut out);
    out.push(blank());
    // Shown inline only when it fits with a row to spare; otherwise v opens the QR view.
    match fitted(
        url,
        height.saturating_sub(out.len() + 2),
        width,
        s.rich_glyphs,
    ) {
        Some(Qr::Fits(rows)) => out.extend(
            rows.into_iter()
                .map(|row| Line::from(Span::styled(row, styles::fg()))),
        ),
        Some(Qr::TooBig { .. }) => out.push(Line::from(Span::styled(
            "On another device? Press v for a QR code to scan.",
            styles::faint(),
        ))),
        None => {}
    }
    out
}

/// The code field sits right under the link, so the QR code can never push it off the screen.
fn push_entry(s: &Screen, width: usize, out: &mut Vec<Line<'static>>) {
    let Some(entry) = &s.entry else {
        return;
    };
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
