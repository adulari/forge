//! Scrollback line builders: turn.

use crate::theme::{color, styles};
use ratatui::style::{Color, Style};
use ratatui::text::{Line as TextLine, Span};

use crate::app::{sanitize_terminal_text, truncate, width_cap};

pub(crate) fn header_line(label: &str, color: Color) -> TextLine<'static> {
    TextLine::from(Span::styled(
        format!("  {label}"),
        Style::default().fg(color).bold(),
    ))
}

pub(crate) fn body_line(text: &str) -> TextLine<'static> {
    TextLine::from(format!("  {text}"))
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

pub(crate) fn warning_line(msg: &str) -> TextLine<'static> {
    let msg = sanitize_terminal_text(msg);
    TextLine::from(Span::styled(
        format!("  {} {msg}", crate::theme::glyphs::g().warn),
        styles::warn(),
    ))
}

pub(crate) fn error_line(msg: &str) -> TextLine<'static> {
    let msg = sanitize_terminal_text(msg);
    TextLine::from(Span::styled(
        format!("  {} {msg}", crate::theme::glyphs::g().fail),
        styles::err().bold(),
    ))
}

pub(crate) fn tool_start_line(name: &str, args: &str, width: u16) -> TextLine<'static> {
    let cap = width_cap(width, 6 + name.chars().count(), 48);
    TextLine::from(vec![
        Span::styled("  ↳ ", styles::muted()),
        Span::styled(name.to_string(), styles::muted().bold()),
        Span::styled(format!("  {}", truncate(args, cap)), styles::muted()),
    ])
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

pub(crate) fn lattice_line(symbols: usize, files: usize, tokens: usize) -> TextLine<'static> {
    TextLine::from(vec![
        Span::styled("  ⌬ lattice ", styles::muted().bold()),
        Span::styled(
            format!("→ injected {symbols} symbols · {files} files (~{tokens} tok)"),
            styles::muted(),
        ),
    ])
}

/// A failed shell command + the model's likely-cause/fix, rendered as a header line plus one
/// dimmed line per diagnosis line (shell-error-interceptor.md).
pub(crate) fn shell_diagnosis_lines(
    command: &str,
    diagnosis: &str,
    fix: Option<&str>,
    width: u16,
) -> Vec<TextLine<'static>> {
    let mut lines = vec![TextLine::from(vec![
        Span::styled(
            format!("  {} shell failed ", crate::theme::glyphs::g().fail),
            styles::err().bold(),
        ),
        Span::styled(truncate(command, width_cap(width, 18, 56)), styles::muted()),
    ])];
    for line in diagnosis.lines() {
        lines.push(TextLine::from(Span::styled(
            format!("    {line}"),
            styles::muted(),
        )));
    }
    if let Some(fix) = fix {
        lines.push(TextLine::from(Span::styled(
            format!("    fix: {fix}"),
            styles::muted(),
        )));
    }
    lines
}

/// A finished tool call as one scrollback row: mark, name, and a truncated summary.
pub(crate) fn tool_result_line(
    name: &str,
    ok: bool,
    summary: &str,
    width: u16,
) -> TextLine<'static> {
    let (mark, color) = if ok {
        ("  ✓ ", color::ok())
    } else {
        ("  ✗ ", color::err())
    };
    let cap = width_cap(width, 6 + name.chars().count(), 56);
    TextLine::from(vec![
        Span::styled(mark, Style::default().fg(color)),
        Span::styled(format!("{name}  "), Style::default().fg(color)),
        Span::styled(truncate(summary, cap), styles::muted()),
    ])
}
