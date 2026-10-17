//! The empty-transcript welcome: a render state of the empty transcript (not scrollback), so it
//! vanishes on the first message and never pollutes history.

use unicode_width::UnicodeWidthStr;

use super::*;
use crate::app::lines::banner::{TAGLINE, WELCOME_KEYS};
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Height from which the two-row half-block wordmark replaces the one-row `✦ forge`.
const WORDMARK_MIN_HEIGHT: u16 = 30;

/// Rows the welcome may use on a short terminal, before the optional `More` row.
const MAX_COMPACT_ROWS: usize = 12;
/// Rows the welcome may use on a tall terminal.
const TALL_ROWS: usize = 18;

/// The half-block `FORGE`, two rows, one entry per letter.
const WORDMARK: [[&str; 5]; 2] = [
    ["█▀▀", "█▀█", "█▀█", "█▀▀", "█▀▀"],
    ["█▀ ", "█▄█", "█▀▄", "█▄█", "██▄"],
];

/// Draw the welcome into `area` when the transcript is empty. Returns whether it drew anything,
/// so the caller falls back to the transcript renderer otherwise.
pub(crate) fn render_empty_state(frame: &mut Frame, area: Rect, app: &App) -> bool {
    if !transcript_is_empty(app, area.width) {
        return false;
    }
    let lines = welcome_lines(app, area.width, area.height);
    frame.render_widget(Paragraph::new(lines), area);
    true
}

pub(crate) fn transcript_is_empty(app: &App, width: u16) -> bool {
    !app.busy
        && app.main_log.is_empty()
        && app.flush.is_empty()
        && app.streaming_edge_len(width) == 0
}

/// The wordmark rows, each letter a step along the ember heat ramp. ASCII terminals get plain
/// `FORGE` text.
fn wordmark_rows() -> Vec<TextLine<'static>> {
    if !g().unicode {
        return vec![TextLine::from(Span::styled(
            "   FORGE",
            styles::brand().bold(),
        ))];
    }
    WORDMARK
        .iter()
        .map(|row| {
            let mut spans = vec![Span::raw("   ")];
            for (i, letter) in row.iter().enumerate() {
                spans.push(Span::styled(
                    format!("{letter} "),
                    styles::heat(i).add_modifier(Modifier::BOLD),
                ));
            }
            TextLine::from(spans)
        })
        .collect()
}

/// A labelled row: a faint label padded to the content column, then the runs.
fn labelled(label: &str, runs: Vec<Span<'static>>) -> TextLine<'static> {
    let mut spans = vec![Span::styled(format!("   {label:<7}"), styles::faint())];
    spans.extend(runs);
    TextLine::from(spans)
}

/// The key hints, dropping the least useful from the end until the row fits.
fn keys_row(width: usize) -> TextLine<'static> {
    let mut keys: Vec<(&str, &str)> = WELCOME_KEYS.to_vec();
    keys.insert(3, ("Shift+Tab", "mode"));
    let cost = |keys: &[(&str, &str)]| -> usize {
        keys.iter()
            .map(|(k, l)| k.width() + 2 + l.width() + 4)
            .sum()
    };
    while keys.len() > 1 && 10 + cost(&keys) > width {
        // Shift+Tab goes first, then the tail.
        match keys.iter().position(|(k, _)| *k == "Shift+Tab") {
            Some(i) => keys.remove(i),
            None => keys.pop().expect("non-empty"),
        };
    }
    let mut runs = Vec::new();
    for (key, label) in keys {
        runs.push(Span::styled(key.to_string(), styles::key_chip()));
        runs.push(Span::styled(format!("  {label}    "), styles::faint()));
    }
    labelled("Keys", runs)
}

/// The example prompts on the welcome; `↓` from an empty composer focuses them and Enter copies
/// the focused one into the composer.
pub(crate) const TRY_EXAMPLES: [&str; 3] = [
    "fix the failing test in this project",
    "explain how this codebase is organised",
    "/plan split the largest file into modules",
];

fn try_row(first: bool, text: &str, focused: bool, width: usize) -> TextLine<'static> {
    let style = if text.starts_with('/') {
        styles::brand()
    } else {
        styles::muted()
    };
    let label = if first { "Try" } else { "" };
    if !focused {
        return labelled(label, vec![Span::styled(text.to_string(), style)]);
    }
    let fill = styles::selected_row();
    let mut spans = vec![
        Span::styled(format!("   {label:<5}"), styles::faint().patch(fill)),
        Span::styled(format!("{} ", g().prompt), styles::brand().patch(fill)),
        Span::styled(text.to_string(), style.patch(fill).bold()),
    ];
    let hint = "   Enter use · Esc back";
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    if used + hint.width() < width {
        spans.push(Span::styled(hint, styles::faint().patch(fill)));
    }
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), fill));
    TextLine::from(spans)
}

fn onboarding_row(width: usize) -> TextLine<'static> {
    let glyphs = g();
    let long = width >= 100;
    let text = if long {
        "This project isn't set up for Forge yet.   "
    } else {
        "Not set up for Forge yet · "
    };
    let tail = if long {
        "  writes project guidance + agents"
    } else {
        " writes guidance + agents"
    };
    TextLine::from(vec![
        Span::styled(format!("   {} ", glyphs.warn), styles::warn()),
        Span::styled(text.to_string(), styles::muted()),
        Span::styled("/init", styles::key_chip()),
        Span::styled(tail.to_string(), styles::muted()),
    ])
}

fn recent_row(app: &App, width: usize) -> Option<TextLine<'static>> {
    let recent = &app.panels_ui.recent_sessions;
    if recent.is_empty() {
        return None;
    }
    let mut runs = Vec::new();
    let mut used = 10 + "/sessions  all".width();
    for (title, age) in recent.iter().take(2) {
        let item = format!("{title} {} {age}", g().sep);
        if used + item.width() + 5 > width {
            break;
        }
        used += item.width() + 5;
        runs.push(Span::styled(item, styles::fg()));
        runs.push(Span::raw("     "));
    }
    runs.push(Span::styled("/sessions", styles::key_chip()));
    runs.push(Span::styled("  all", styles::faint()));
    Some(labelled("Recent", runs))
}

/// The welcome as lines: wordmark (tall terminals) or `✦ forge`, tagline, the `/init` nudge while
/// the project is uninitialised, three examples, the keys row and recent sessions.
pub(crate) fn welcome_lines(app: &App, width: u16, height: u16) -> Vec<TextLine<'static>> {
    let width = width as usize;
    let mut lines = vec![TextLine::default()];
    if height >= WORDMARK_MIN_HEIGHT {
        lines.extend(wordmark_rows());
        lines.push(TextLine::default());
    } else {
        lines.push(TextLine::from(vec![
            Span::styled(format!("   {} ", g().brand), styles::brand().bold()),
            Span::styled("forge", styles::brand().bold()),
        ]));
    }
    lines.push(TextLine::from(Span::styled(
        format!("   {TAGLINE}"),
        styles::fg(),
    )));
    lines.push(TextLine::default());
    if app
        .panels_ui
        .onboarding_pending(app.git_workspace.as_deref())
    {
        lines.push(onboarding_row(width));
        lines.push(TextLine::default());
    }
    for (i, example) in TRY_EXAMPLES.iter().enumerate() {
        let focused = app.panels_ui.try_focus == Some(i);
        lines.push(try_row(i == 0, example, focused, width));
    }
    lines.push(TextLine::default());
    lines.push(keys_row(width));
    if let Some(recent) = recent_row(app, width) {
        lines.push(TextLine::default());
        lines.push(recent);
    }
    // The setup and routing escape hatches, when the welcome has room left.
    let row_budget = if height >= WORDMARK_MIN_HEIGHT {
        TALL_ROWS
    } else {
        MAX_COMPACT_ROWS
    };
    if lines.len() < row_budget && height >= 20 {
        lines.push(TextLine::default());
        lines.push(labelled(
            "More",
            vec![
                Span::styled("forge doctor", styles::key_chip()),
                Span::styled("  check readiness    ", styles::faint()),
                Span::styled("/mesh", styles::key_chip()),
                Span::styled("  preview routing", styles::faint()),
            ],
        ));
    }
    lines
}
