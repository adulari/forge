//! Scrollback line builders: banner.

use crate::theme::styles;
use ratatui::text::{Line as TextLine, Span};

/// ANSI-Shadow block wordmark retained for terminal startup, with a quieter in-chat welcome.
pub(crate) const FORGE_WORDMARK: &[&str] = &[
    "███████╗ ██████╗ ██████╗  ██████╗ ███████╗",
    "██╔════╝██╔═══██╗██╔══██╗██╔════╝ ██╔════╝",
    "█████╗  ██║   ██║██████╔╝██║  ███╗█████╗  ",
    "██╔══╝  ██║   ██║██╔══██╗██║   ██║██╔══╝  ",
    "██║     ╚██████╔╝██║  ██║╚██████╔╝███████╗",
    "╚═╝      ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝",
];

pub(crate) const WORDMARK_WIDTH: u16 = 42;

pub(crate) const TAGLINE: &str = "Your coding partner. Describe what you want to do.";

/// The welcome banner, printed once into scrollback. Left-aligned.
pub fn banner_lines(width: u16) -> Vec<TextLine<'static>> {
    let mut lines = vec![TextLine::default()];
    if width < WORDMARK_WIDTH {
        lines.push(TextLine::from(Span::styled(
            "FORGE",
            styles::brand().bold(),
        )));
        lines.push(TextLine::from(Span::styled(
            "model-mesh coding agent",
            styles::muted(),
        )));
    } else {
        for row in FORGE_WORDMARK {
            lines.push(TextLine::from(Span::styled(
                row.to_string(),
                styles::brand().bold(),
            )));
        }
        lines.push(TextLine::default());
        lines.push(TextLine::from(Span::styled(
            TAGLINE.to_string(),
            styles::muted(),
        )));
    }
    lines.push(TextLine::default());
    lines
}

/// Print the startup banner directly to stdout using raw ANSI escape codes, bypassing ratatui's
/// `insert_before` / `draw_lines_over_cleared` path.
///
/// ratatui's `draw_lines_over_cleared` uses `old.diff(&new)` where old = `Buffer::empty` (all
/// `Cell::EMPTY` = space + Reset style). The banner logo is 42 chars wide; the remaining columns
/// on each logo row are also `Cell::EMPTY` in the new buffer, so the diff skips them and never
/// writes to those terminal cells. If the terminal had prior content at those positions (e.g.
/// Claude Code hook output from the same session), it bleeds through to the right of the logo.
///
/// By printing before ratatui creates the inline viewport, the terminal handles each `\n`
/// naturally, clearing the rest of each row implicitly.
pub fn print_banner_direct() {
    use std::io::Write;
    let width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
    let mut out = std::io::stdout();
    let p = crate::theme::t().palette;
    let brand = crate::theme::Token {
        mods: p.brand.mods | ratatui::style::Modifier::BOLD,
        ..p.brand
    };
    let paint_brand = |text: &str| crate::theme::paint(brand, text);
    let paint_dim = |text: &str| crate::theme::paint(p.fg_muted, text);
    if width < WORDMARK_WIDTH {
        let _ = writeln!(out, "\n{}", paint_brand("FORGE"));
        let _ = writeln!(out, "{}\n", paint_dim("model-mesh coding agent"));
    } else {
        let _ = writeln!(out);
        for row in FORGE_WORDMARK {
            let _ = writeln!(out, "{}", paint_brand(row));
        }
        let _ = writeln!(out, "\n{}\n", paint_dim(TAGLINE));
    }
    let _ = out.flush();
}
