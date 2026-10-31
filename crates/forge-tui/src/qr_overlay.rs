//! The connect-link QR code as a dismissable overlay, instead of a block pushed into the
//! scrollback where it scrolls away and cannot be sized to the terminal.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, KeyKind};
use crate::onboarding::qr::{self, Qr};
use crate::surface::{self, SurfaceTone};
use crate::theme::styles;

#[derive(Debug, Clone)]
pub struct QrOverlay {
    pub title: String,
    pub url: String,
    /// The link only works on this machine, so a phone scanning it would get nowhere.
    pub loopback: bool,
}

impl App {
    /// Show `url` as a QR code until the person dismisses it.
    pub fn open_qr_overlay(&mut self, title: &str, url: &str, loopback: bool) {
        self.polish.qr = Some(QrOverlay {
            title: title.to_string(),
            url: url.to_string(),
            loopback,
        });
    }

    /// Keys while the overlay is up. It owns them all; Esc, Enter, `q` and Space close it.
    pub fn qr_overlay_key(&mut self, key: KeyKind) -> bool {
        if self.polish.qr.is_none() {
            return false;
        }
        if matches!(
            key,
            KeyKind::Esc
                | KeyKind::Enter
                | KeyKind::Interrupt
                | KeyKind::Char('q')
                | KeyKind::Char(' ')
        ) {
            self.polish.qr = None;
        }
        true
    }
}

const LOOPBACK_HINT: &str =
    "This link only works on this computer. To scan it from a phone, run /remote lan.";

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match rows.last_mut() {
            Some(row) if row.width() + 1 + word.width() <= width => {
                row.push(' ');
                row.push_str(word);
            }
            _ => rows.push(word.to_string()),
        }
    }
    rows
}

/// Break a link that has no spaces at `width` cells.
fn chop(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for c in text.chars() {
        if row.width() + 1 > width {
            rows.push(std::mem::take(&mut row));
        }
        row.push(c);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// Terminals known to draw Unicode block sextants themselves, which packs a code into a third
/// fewer rows and half the columns. SSH carries `TERM` across, so this also holds remotely.
fn draws_sextants() -> bool {
    let term = std::env::var("TERM").unwrap_or_default();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    ["kitty", "ghostty", "foot", "wezterm"]
        .iter()
        .any(|t| term.contains(t))
        || matches!(program.as_str(), "WezTerm" | "ghostty")
}

/// Card height for `content` rows: borders, plus the blank rows top and bottom that
/// `padded_panel` only draws once a card is tall enough.
fn card_height(content: usize) -> usize {
    let padded = content + 2 + 2 * usize::from(crate::overlays::PAD_Y);
    if padded >= usize::from(crate::overlays::SHORT_REGION) {
        padded
    } else {
        content + 2
    }
}

/// `modal` is the room between the header and the composer; `full` is everything, used when the
/// code does not fit in `modal` (a short window) because a code that cannot be scanned is no use.
pub(crate) fn render_qr_overlay(frame: &mut Frame, app: &App, modal: Rect, full: Rect) {
    let Some(overlay) = &app.polish.qr else {
        return;
    };
    let card_w = modal.width.saturating_sub(2).clamp(1, 76);
    let text_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let url_rows = chop(&overlay.url, text_w);
    let sextants = draws_sextants();
    let layout = |height: u16, with_hint: bool| {
        let hint_rows = if overlay.loopback && with_hint {
            wrap(LOOPBACK_HINT, text_w)
        } else {
            Vec::new()
        };
        let extra = url_rows.len() + hint_rows.len() + usize::from(!hint_rows.is_empty()) + 1;
        // Each card_height step is one row, so ask for the most code that still fits.
        let mut rows = 0;
        while card_height(rows + 1 + extra) <= height as usize {
            rows += 1;
        }
        (
            hint_rows,
            extra,
            qr::fitted(&overlay.url, rows, text_w, sextants),
        )
    };
    // Above the status line the composer has nothing to say while the code is up, so a short
    // window may use its rows too.
    let tall = Rect {
        height: full.height.saturating_sub(1).max(1),
        ..full
    };
    let mut region = modal;
    let (mut hint_rows, mut extra, mut fitted) = layout(modal.height, true);
    for (room, with_hint) in [(modal, false), (tall, true), (tall, false)] {
        if matches!(fitted, Some(Qr::Fits(_))) {
            break;
        }
        region = room;
        (hint_rows, extra, fitted) = layout(room.height, with_hint);
    }
    surface::dim_backdrop(frame, region);
    let qr_lines = match &fitted {
        Some(Qr::Fits(lines)) => lines.clone(),
        _ => Vec::new(),
    };
    let content = qr_lines.len().max(2) + extra;
    let card_h = (card_height(content) as u16).min(region.height).max(6);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let inner = crate::overlays::padded_panel(
        frame,
        card,
        surface::title(overlay.title.clone(), SurfaceTone::Brand),
        None,
        Some(Span::styled("Esc close", styles::faint())),
        SurfaceTone::Brand,
    );
    let mut lines: Vec<Line<'static>> = Vec::new();
    match fitted {
        Some(Qr::Fits(_)) => {
            for row in qr_lines {
                let pad = text_w.saturating_sub(row.width()) / 2;
                lines.push(Line::from(Span::styled(
                    format!("{}{row}", " ".repeat(pad)),
                    styles::fg(),
                )));
            }
        }
        Some(Qr::TooBig { rows, cols }) => lines.push(Line::from(Span::styled(
            format!(
                "The QR code needs {cols}x{rows} cells. Enlarge the window, or copy the link below."
            ),
            styles::warn(),
        ))),
        None => lines.push(Line::from(Span::styled(
            "This link is too long for a QR code.",
            styles::warn(),
        ))),
    }
    lines.push(Line::default());
    for row in url_rows {
        lines.push(Line::from(Span::styled(row, styles::muted())));
    }
    if !hint_rows.is_empty() {
        lines.push(Line::default());
        for row in hint_rows {
            lines.push(Line::from(Span::styled(row, styles::warn())));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                let modal = Rect {
                    y: 1,
                    height: area.height.saturating_sub(5),
                    ..area
                };
                render_qr_overlay(f, app, modal, area)
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(w as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_overlay_draws_a_code_the_link_and_the_loopback_hint() {
        let mut app = App::default();
        app.open_qr_overlay("Remote control", "https://127.0.0.1:7421/t/abc", true);
        let screen = draw(&app, 100, 40);
        assert!(screen.contains("Remote control"), "{screen}");
        assert!(screen.contains("https://127.0.0.1:7421/t/abc"), "{screen}");
        assert!(screen.contains("only works on this computer"), "{screen}");
        assert!(screen.contains('█') || screen.contains('▀'), "{screen}");
    }

    #[test]
    fn a_reachable_link_has_no_loopback_hint() {
        let mut app = App::default();
        app.open_qr_overlay("Remote control", "https://192.168.1.5:7421/t/abc", false);
        assert!(!draw(&app, 100, 40).contains("only works on this computer"));
    }

    #[test]
    fn a_small_terminal_says_how_much_room_the_code_needs() {
        let mut app = App::default();
        app.open_qr_overlay("Remote control", "https://192.168.1.5:7421/t/abc", false);
        let screen = draw(&app, 40, 12);
        assert!(screen.contains("QR code needs"), "{screen}");
    }

    #[test]
    fn keys_close_it_and_everything_else_is_swallowed() {
        let mut app = App::default();
        assert!(!app.qr_overlay_key(KeyKind::Esc), "no overlay, no claim");
        app.open_qr_overlay("t", "https://x.test/a", false);
        assert!(app.qr_overlay_key(KeyKind::Char('a')));
        assert!(app.polish.qr.is_some());
        assert!(app.qr_overlay_key(KeyKind::Esc));
        assert!(app.polish.qr.is_none());
    }
}
