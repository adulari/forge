//! Scrollback line builders: turn.

use crate::theme::styles::{self, NoticeKind, RailKind};
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

use super::{notice, rail};
use crate::app::{truncate, width_cap};

/// The `you` label row of a user turn (or `you · follow-up` for a message that joined a running turn).
pub(crate) fn user_label(extra: Option<&str>) -> TextLine<'static> {
    let label = match extra {
        Some(e) => format!("you {} {e}", crate::theme::glyphs::g().sep),
        None => "you".to_string(),
    };
    rail::label_row(
        RailKind::User,
        vec![Span::styled(label, styles::user().bold())],
        Vec::new(),
    )
}

/// The label of a message another session sent.
pub(crate) fn peer_label(who: &str) -> TextLine<'static> {
    rail::label_row(
        RailKind::User,
        vec![Span::styled(
            format!("peer message from {who}"),
            styles::muted().bold(),
        )],
        Vec::new(),
    )
}

/// One row of what the user typed.
pub(crate) fn user_text(text: &str) -> TextLine<'static> {
    rail::text_row(RailKind::User, text, styles::fg())
}

/// A routing tier as the footer and the turn label name it. The tier names are the router's own
/// vocabulary (`/mesh` and the status inspector keep them); beside a model name they read as
/// noise, and "pinned" says nothing about who pinned what.
pub(crate) fn plain_tier(tier: &str) -> &str {
    match tier {
        "trivial" => "simple",
        "pinned" => "your pick",
        other => other,
    }
}

/// The assistant's label: identity on the left, the routed model and tier right-aligned.
pub(crate) fn assistant_label(model: Option<&str>, tier: &str) -> TextLine<'static> {
    let g = crate::theme::glyphs::g();
    // A pinned model is the user's own choice: its name says everything the tier would.
    let tier = if tier == "pinned" {
        ""
    } else {
        plain_tier(tier)
    };
    let label = vec![
        Span::styled(g.brand, styles::brand()),
        Span::styled(" forge", styles::fg().bold()),
    ];
    let right = match (model, tier.is_empty()) {
        (Some(m), false) => format!("{m} {} {tier}", g.sep),
        (Some(m), true) => m.to_string(),
        (None, _) => String::new(),
    };
    let right = if right.is_empty() {
        Vec::new()
    } else {
        vec![Span::styled(right, styles::faint())]
    };
    rail::label_row(rail::BAKED, label, right)
}

/// The faint footer closing a turn: elapsed, tokens, cost.
pub(crate) fn turn_footer(facts: &str) -> TextLine<'static> {
    rail::text_row(rail::BAKED, facts, styles::faint())
}

/// Greedy word-wrap to `width` display columns (approximated by char count, matching the rest of
/// this module). Words longer than `width` are hard-split so a single long token can't overflow.
/// Always returns at least one (possibly empty) line.
pub(crate) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for word in text.split_whitespace() {
        // Measure in terminal CELLS (CJK/emoji = 2) so a row never overflows its column width.
        let ww = UnicodeWidthStr::width(word);
        if ww > width {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            let mut chunk = String::new();
            let mut chunk_w = 0usize;
            for ch in word.chars() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
                if chunk_w + cw > width && chunk_w > 0 {
                    lines.push(std::mem::take(&mut chunk));
                    chunk_w = 0;
                }
                chunk.push(ch);
                chunk_w += cw;
            }
            cur = chunk;
            cur_w = chunk_w;
            continue;
        }
        let add = if cur.is_empty() { ww } else { ww + 1 };
        if cur_w + add > width {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
            cur_w = ww;
        } else {
            if !cur.is_empty() {
                cur.push(' ');
                cur_w += 1;
            }
            cur.push_str(word);
            cur_w += ww;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Render a symbol's scoped subgraph (the `/lattice` view) as styled scrollback lines: the
/// matching definitions, their reverse-dependents (blast radius), and a provenance line. Takes
/// plain tuples `(kind, name, rel_path, line)` so this crate needn't depend on forge-index.
#[allow(clippy::type_complexity)]
pub fn lattice_view_lines(
    query: &str,
    roots: &[(String, String, String, i64)],
    dependents: &[(String, String, String, i64)],
    why: Option<(String, String, String, String)>,
) -> Vec<TextLine<'static>> {
    let mut out = Vec::new();
    out.push(TextLine::from(vec![
        Span::styled("  ⌬ LATTICE ", styles::muted().bold()),
        Span::styled(format!(" {query}"), styles::muted()),
    ]));
    if roots.is_empty() {
        out.push(TextLine::from(Span::styled(
            format!("    no symbols match '{query}' — run `forge lattice update`?"),
            styles::muted(),
        )));
        return out;
    }
    for (kind, name, path, line) in roots {
        out.push(TextLine::from(vec![
            Span::styled("    ● ", styles::muted()),
            Span::styled(format!("{kind} "), styles::muted()),
            Span::styled(name.clone(), styles::user().bold()),
            Span::styled(format!("  {path}:{line}"), styles::muted()),
        ]));
    }
    if !dependents.is_empty() {
        const MAX: usize = 20;
        out.push(TextLine::from(Span::styled(
            format!("    blast radius: {} reference(s)", dependents.len()),
            styles::muted(),
        )));
        for (kind, name, path, line) in dependents.iter().take(MAX) {
            out.push(TextLine::from(vec![
                Span::styled("    ← ", styles::muted()),
                Span::styled(format!("{kind} "), styles::muted()),
                Span::styled(name.clone(), Style::default()),
                Span::styled(format!("  {path}:{line}"), styles::muted()),
            ]));
        }
        if dependents.len() > MAX {
            out.push(TextLine::from(Span::styled(
                format!(
                    "    … +{} more (forge lattice impact)",
                    dependents.len() - MAX
                ),
                styles::muted(),
            )));
        }
    }
    if let Some((author, date, commit, subject)) = why {
        out.push(TextLine::from(Span::styled(
            format!("    why: {author} · {date} · {commit} · {subject}"),
            styles::muted(),
        )));
    }
    out
}

pub(crate) fn lattice_line(
    symbols: usize,
    files: usize,
    tokens: usize,
    in_turn: bool,
) -> Vec<TextLine<'static>> {
    notice::notice_with(
        NoticeKind::Info,
        in_turn,
        vec![
            Span::styled("lattice ", styles::muted().bold()),
            Span::styled(
                format!("injected {symbols} symbols · {files} files (~{tokens} tok)"),
                styles::muted(),
            ),
        ],
        &[],
    )
}

/// A failed shell command + the model's likely-cause/fix: an error notice naming the command,
/// then one muted row per diagnosis line (shell-error-interceptor.md).
pub(crate) fn shell_diagnosis_lines(
    command: &str,
    diagnosis: &str,
    fix: Option<&str>,
    width: u16,
    in_turn: bool,
) -> Vec<TextLine<'static>> {
    let mut details: Vec<String> = diagnosis.lines().map(str::to_string).collect();
    if let Some(fix) = fix {
        details.push(format!("fix: {fix}"));
    }
    notice::notice_with(
        NoticeKind::Err,
        in_turn,
        vec![
            Span::styled("shell failed ", styles::err().bold()),
            Span::styled(truncate(command, width_cap(width, 18, 56)), styles::muted()),
        ],
        &details,
    )
}

/// The thinking block of a turn: one faint italic line when hidden, the text behind an inner
/// rail when open.
pub(crate) fn thinking_rows(text: &str, secs: u64, open: bool) -> Vec<TextLine<'static>> {
    let g = crate::theme::glyphs::g();
    let tri = if open {
        g.disclose_open
    } else {
        g.disclose_closed
    };
    let head = TextLine::from(Span::styled(
        format!("{tri} Thought for {secs}s"),
        styles::faint().add_modifier(ratatui::style::Modifier::ITALIC),
    ));
    let mut rows = vec![head];
    if open {
        let bar = if g.unicode { "▏ " } else { "| " };
        for l in text.lines() {
            rows.push(TextLine::from(vec![
                Span::styled(String::new(), crate::copy_mark::prose(Style::default())),
                Span::styled(bar, crate::copy_mark::repeat(styles::faint())),
                Span::styled(
                    l.to_string(),
                    styles::muted().add_modifier(ratatui::style::Modifier::ITALIC),
                ),
            ]));
        }
    }
    rows
}
