//! The rows of a call inside an open tool group, and of a failed call under a folded one.
//!
//! A card header elsewhere is one row cut to fit. Here the call's text (the command, the path,
//! the script) wraps onto continuation rows that hang under the text column, up to
//! [`MAX_TEXT_ROWS`], and the outcome stays flush right on the first row, never cut: a tight row
//! drops the duration before it touches the status. The calls a script made hang one level under
//! it, and the script row says what it called instead of showing its source.

use super::card_text::{cells, shorten_in};
use super::lines::rail;
use super::script_row;
use super::tool_cards::{self, CardStatus, ToolCard, VERB_W};
use crate::copy_mark;
use crate::theme::glyphs::g;
use crate::theme::styles;
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

/// Rows a call's text may take, the `… N more lines` row included.
pub(crate) const MAX_TEXT_ROWS: usize = 4;
/// The fewest cells of text the first row keeps before the right side gives up its duration.
const MIN_FIRST_TEXT: usize = 24;

/// One call to lay out.
pub(crate) struct CallRow<'a> {
    pub(crate) card: &'a ToolCard,
    /// Levels under the group header: 0 for a call of the group, 1 for a call a script made.
    pub(crate) depth: usize,
    /// The calls a `run_code` card made, oldest first.
    pub(crate) children: &'a [&'a ToolCard],
    /// The cells a row may use.
    pub(crate) width: usize,
    /// The spinner tick while the call runs, when the row is drawn live.
    pub(crate) tick: Option<usize>,
    pub(crate) awaiting: bool,
    /// Whether to show the card's own `▸` (not under a folded group, where a click opens the
    /// group).
    pub(crate) disclosure: bool,
}

fn spans_w(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// The duration a card shows: its own, or the sum over the reads folded into it.
fn duration(card: &ToolCard) -> Option<u128> {
    if card.group.is_empty() {
        card.ms
    } else {
        Some(card.group.iter().filter_map(|m| m.ms).sum::<u128>() + card.ms.unwrap_or(0))
    }
}

/// The right side, fullest first: outcome, kept-output size and duration; then without the
/// size; then the outcome alone.
fn meta_variants(r: &CallRow<'_>) -> Vec<Vec<Span<'static>>> {
    let card = r.card;
    let running = card.status == CardStatus::Running;
    if running && r.awaiting {
        return vec![vec![Span::styled("awaiting approval", styles::warn())]];
    }
    let style = if card.status == CardStatus::Failed {
        styles::err()
    } else {
        styles::muted()
    };
    let outcome = tool_cards::outcome_text(card);
    let full = card
        .full
        .as_ref()
        .filter(|_| !running)
        .map(|f| format!("{} lines", tool_cards::group(f.lines)));
    let ms = duration(card).filter(|_| !running).map(tool_cards::fmt_ms);
    let sep = format!(" {} ", g().sep);
    let join = |parts: &[Option<&String>]| -> String {
        std::iter::once(&outcome)
            .chain(parts.iter().flatten().copied())
            .cloned()
            .collect::<Vec<_>>()
            .join(&sep)
    };
    let mut texts = vec![join(&[full.as_ref(), ms.as_ref()]), join(&[ms.as_ref()])];
    texts.push(outcome.clone());
    texts.dedup();
    let tri = (r.disclosure && card.has_detail() && !running).then(|| {
        let t = if card.expanded {
            g().disclose_open
        } else {
            g().disclose_closed
        };
        Span::styled(format!(" {t}"), styles::faint())
    });
    texts
        .into_iter()
        .map(|t| {
            let mut v = vec![Span::styled(t, style)];
            v.extend(tri.clone());
            v
        })
        .collect()
}

/// What the row says, one entry per logical line.
fn text_lines(r: &CallRow<'_>) -> Vec<String> {
    let card = r.card;
    if card.name == "run_code" {
        if !r.children.is_empty() {
            return vec![script_label(r.children)];
        }
        let lines = script_row::meaningful_lines(&card.args);
        if !lines.is_empty() {
            return lines;
        }
    }
    if !card.group.is_empty() {
        // A card that folded several reads lists every file it read.
        return card
            .group
            .iter()
            .map(|m| tool_cards::headline(&m.name, &m.args))
            .chain(std::iter::once(tool_cards::headline(
                &card.name, &card.args,
            )))
            .collect();
    }
    tool_cards::headline(&card.name, &card.args)
        .split(" ⏎ ")
        .map(shorten_in)
        .collect()
}

/// `2 calls: run, read` for a script whose calls hang under it.
pub(crate) fn script_label(children: &[&ToolCard]) -> String {
    let mut verbs: Vec<String> = Vec::new();
    for c in children {
        let v = tool_cards::verb(&c.name).to_lowercase();
        if !verbs.contains(&v) {
            verbs.push(v);
        }
    }
    let n = children.len();
    format!(
        "{n} {}: {}",
        if n == 1 { "call" } else { "calls" },
        verbs.join(", ")
    )
}

/// Split `s` after at most `w` cells, at the last space past the middle when there is one.
fn take_cells(s: &str, w: usize) -> (&str, &str) {
    use unicode_width::UnicodeWidthChar;
    if cells(s) <= w {
        return (s, "");
    }
    let mut used = 0;
    let mut cut = 0;
    for (i, ch) in s.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + cw > w {
            break;
        }
        used += cw;
        cut = i + ch.len_utf8();
    }
    if cut == 0 {
        cut = s.chars().next().map_or(0, char::len_utf8);
    }
    match s[..cut].rfind(' ').filter(|&sp| sp > cut / 2) {
        Some(sp) => (&s[..sp], &s[sp + 1..]),
        None => (&s[..cut], &s[cut..]),
    }
}

/// Wrap the logical lines: the first row has `first` cells, the rest `rest`. Each row says
/// whether it continues the line above (a copy rejoins those).
fn wrap_rows(lines: &[String], first: usize, rest: usize) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for line in lines {
        let mut left: &str = line;
        let mut cont = false;
        loop {
            let w = if out.is_empty() { first } else { rest };
            if w == 0 {
                out.push((String::new(), false));
                continue;
            }
            let (chunk, tail) = take_cells(left, w);
            out.push((chunk.to_string(), cont));
            cont = true;
            if tail.is_empty() {
                break;
            }
            left = tail;
        }
    }
    out
}

/// The rows of one call: the header with its outcome flush right, then the continuation rows.
/// Every row fits `width` exactly or less, so none of them wraps again.
pub(crate) fn call_rows(r: &CallRow<'_>) -> Vec<TextLine<'static>> {
    let card = r.card;
    let (mark, mark_style) = tool_cards::status_mark(card, r.tick, r.awaiting);
    let mut head = rail::rail_spans(rail::BAKED);
    head.push(Span::styled(
        " ".repeat(2 * (r.depth + 1)),
        copy_mark::decor(Style::default()),
    ));
    head.push(Span::styled(mark, mark_style));
    head.push(Span::raw(" "));
    head.push(Span::styled(
        format!("{:<VERB_W$} ", tool_cards::verb(&card.name)),
        styles::muted().bold(),
    ));
    let col = spans_w(&head);
    let variants = meta_variants(r);
    let fits = |v: &Vec<Span<'static>>| r.width.saturating_sub(col + 2 + spans_w(v));
    let mut right = variants
        .iter()
        .find(|v| fits(v) >= MIN_FIRST_TEXT)
        .or(variants.last())
        .cloned()
        .unwrap_or_default();
    // A status is a few words and is never cut; a long free-text outcome (an error message) is,
    // down to a dozen cells, before the call's own text loses its row.
    if fits(&right) < MIN_FIRST_TEXT {
        let tri_w = right.get(1..).map_or(0, spans_w);
        if let Some(first) = right.first_mut() {
            let keep = r
                .width
                .saturating_sub(col + 2 + MIN_FIRST_TEXT + tri_w)
                .max(12);
            if first.width() > keep {
                first.content = tool_cards::truncate_cells(&first.content, keep).into();
            }
        }
    }
    let first = fits(&right);
    let rest = r.width.saturating_sub(col + 1).max(8);
    let mut rows = wrap_rows(&text_lines(r), first, rest);
    let total = rows.len();
    let hidden = if total > MAX_TEXT_ROWS {
        rows.truncate(MAX_TEXT_ROWS - 1);
        total - (MAX_TEXT_ROWS - 1)
    } else {
        0
    };
    let text_style = styles::fg();
    let mut out = Vec::with_capacity(rows.len() + 1);
    let (lead, _) = rows.first().cloned().unwrap_or_default();
    let mut line = head;
    let lead_w = cells(&lead);
    line.push(Span::styled(lead, text_style));
    let pad = r
        .width
        .saturating_sub(col + lead_w + spans_w(&right))
        .max(2);
    line.push(Span::styled(
        " ".repeat(pad),
        copy_mark::decor(Style::default()),
    ));
    line.extend(right);
    out.push(TextLine::from(line));
    let hang = |cont: bool| {
        let mut spans = Vec::new();
        if cont {
            spans.push(copy_mark::soft_wrap_marker());
        }
        spans.extend(rail::rail_spans(rail::BAKED));
        spans.push(Span::styled(
            " ".repeat(col.saturating_sub(3)),
            copy_mark::decor(Style::default()),
        ));
        spans
    };
    for (text, cont) in rows.into_iter().skip(1) {
        let mut spans = hang(cont);
        spans.push(Span::styled(text, text_style));
        out.push(TextLine::from(spans));
    }
    if hidden > 0 {
        let mut spans = hang(false);
        spans.push(Span::styled(
            format!("{} {hidden} more lines", g().ellipsis),
            copy_mark::decor(styles::faint()),
        ));
        out.push(TextLine::from(spans));
    }
    out
}

#[cfg(test)]
#[path = "group_rows_tests.rs"]
mod tests;
