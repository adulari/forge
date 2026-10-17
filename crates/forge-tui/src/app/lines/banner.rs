//! Scrollback line builders: the compact startup welcome printed once in inline mode. Fullscreen
//! draws its welcome as a render state of the empty transcript instead (`render/welcome.rs`).

use crate::theme::glyphs::g;
use crate::theme::styles;
use ratatui::text::{Line as TextLine, Span};
use unicode_width::UnicodeWidthStr;

pub(crate) const TAGLINE: &str = "Your coding partner. Describe what you want to do.";

/// The shortcut keys the welcome teaches, most useful first. Dropped from the end when narrow.
pub(crate) const WELCOME_KEYS: &[(&str, &str)] = &[
    ("/", "commands"),
    ("@", "files"),
    ("Ctrl+K", "actions"),
    ("?", "all keys"),
];

fn version_label() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// `project ⑂ branch` for the working directory, or just the directory name outside a checkout.
fn project_label() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    match crate::git_location::read_git_location(&cwd) {
        Some(location) => Some(format!(
            "{} {} {}",
            location.repo,
            g().branch,
            location.branch_label()
        )),
        None => cwd.file_name().map(|n| n.to_string_lossy().into_owned()),
    }
}

/// Clip `text` to `max` cells with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push_str(g().ellipsis);
    out
}

#[derive(Clone, Copy)]
enum Role {
    Brand,
    Chip,
    Muted,
    Faint,
}

impl Role {
    fn style(self) -> ratatui::style::Style {
        match self {
            Role::Brand => styles::brand().bold(),
            Role::Chip => styles::key_chip(),
            Role::Muted => styles::muted(),
            Role::Faint => styles::faint(),
        }
    }

    fn token(self) -> crate::theme::Token {
        let p = crate::theme::t().palette;
        match self {
            Role::Brand => crate::theme::Token {
                mods: p.brand.mods | ratatui::style::Modifier::BOLD,
                ..p.brand
            },
            Role::Chip => crate::theme::Token {
                mods: p.fg.mods | ratatui::style::Modifier::BOLD,
                ..p.fg
            },
            Role::Muted => p.fg_muted,
            Role::Faint => p.fg_faint,
        }
    }
}

/// The welcome rows as `(text, role)` runs: the identity row, then the keys row.
fn compact_rows(width: u16) -> Vec<Vec<(String, Role)>> {
    let glyphs = g();
    let room = (width as usize).saturating_sub(2);
    let mut head = vec![
        (format!(" {} forge", glyphs.brand), Role::Brand),
        (format!("  {}", version_label()), Role::Faint),
    ];
    if let Some(project) = project_label() {
        let used: usize = head.iter().map(|(t, _)| t.width()).sum();
        let lead = format!(" {} ", glyphs.sep);
        let room_for_project = room.saturating_sub(used + lead.width());
        if room_for_project >= 8 {
            head.push((
                format!("{lead}{}", clip(&project, room_for_project)),
                Role::Faint,
            ));
        }
    }
    let used: usize = head.iter().map(|(t, _)| t.width()).sum();
    let sep = format!(" {} ", glyphs.sep);
    let tail = TAGLINE.to_string();
    let budget = room.saturating_sub(used + sep.width());
    if budget >= 16 {
        head.push((sep, Role::Faint));
        head.push((clip(&tail, budget), Role::Muted));
    }
    let mut keys = vec![(" Keys".to_string(), Role::Faint)];
    let mut used = keys[0].0.width();
    for (key, label) in WELCOME_KEYS {
        let cost = 2 + key.width() + 1 + label.width();
        if used + cost > room {
            break;
        }
        keys.push(("  ".to_string(), Role::Faint));
        keys.push((key.to_string(), Role::Chip));
        keys.push((format!(" {label}"), Role::Faint));
        used += cost;
    }
    vec![head, keys]
}

/// The compact welcome (no block art): identity, version, project and a keys row.
pub fn banner_lines(width: u16) -> Vec<TextLine<'static>> {
    let mut lines = vec![TextLine::default()];
    for row in compact_rows(width) {
        lines.push(TextLine::from(
            row.into_iter()
                .map(|(text, role)| Span::styled(text, role.style()))
                .collect::<Vec<_>>(),
        ));
    }
    lines.push(TextLine::default());
    lines
}

/// Print the startup welcome directly to stdout using raw ANSI escape codes, bypassing ratatui's
/// `insert_before` path.
///
/// ratatui's `draw_lines_over_cleared` diffs against an empty buffer, so default-style cells to
/// the right of short rows are never written and prior terminal content (for example hook output
/// from the same session) bleeds through. Printing before ratatui creates the inline viewport
/// lets the terminal clear each row itself. Every colour goes through the theme tier, so
/// `NO_COLOR` and 16-colour terminals get plain or palette-relative output.
pub fn print_banner_direct() {
    use std::io::Write;
    let width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
    let mut out = std::io::stdout();
    let _ = writeln!(out);
    for row in compact_rows(width) {
        let line: String = row
            .iter()
            .map(|(text, role)| crate::theme::paint(role.token(), text))
            .collect();
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out);
    let _ = out.flush();
}
