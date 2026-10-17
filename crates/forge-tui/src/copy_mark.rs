//! What a mouse selection copies. Renderers tag the cells that are decoration rather than content
//! (frame bars, gutters, box padding) with private style bits, and [`selection_text`] drops exactly
//! those cells. The text is never inspected, so code that legitimately contains `│` survives, which
//! a glyph-stripping regex could not guarantee.
//!
//! The bits ride on [`Style`] because every wrap and clone in the transcript pipeline already
//! carries it. No terminal backend reads them: ratatui only diffs the modifiers it knows.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// The cell is decoration (border, gutter, padding) and is never copied.
const NO_COPY: Modifier = Modifier::from_bits_retain(1 << 15);
/// Set on the empty marker span `wrap_lines` puts at the head of a row that continues a wrapped
/// framed line, so the copy rejoins it instead of inserting a newline.
const SOFT_WRAP: Modifier = Modifier::from_bits_retain(1 << 14);
/// The whole row is a frame edge (`┌ rust ───`, `└───`): it yields neither text nor a newline.
const FRAME_ROW: Modifier = Modifier::from_bits_retain(1 << 13);

/// `style`, marked as decoration inside a content row.
pub(crate) fn decor(style: Style) -> Style {
    style.add_modifier(NO_COPY)
}

pub(crate) fn is_decor(modifier: Modifier) -> bool {
    modifier.intersects(NO_COPY)
}

/// `style`, marked as a frame-edge row that contributes nothing to a copy.
pub(crate) fn frame_row(style: Style) -> Style {
    style.add_modifier(NO_COPY | FRAME_ROW)
}

/// The span repeats verbatim at the head of every wrapped continuation row (the turn rail, a
/// blockquote bar). Always decoration.
const REPEAT: Modifier = Modifier::from_bits_retain(1 << 9);
/// Continuation rows of the line indent under the END of this span (list markers, card gutters).
const HANG: Modifier = Modifier::from_bits_retain(1 << 10);
/// The line is prose: it wraps at the reading measure instead of the full width.
const PROSE: Modifier = Modifier::from_bits_retain(1 << 11);
/// Marker span: everything after it is pushed flush against the right margin.
const RIGHT: Modifier = Modifier::from_bits_retain(1 << 12);

/// Widest a prose row may be, in content cells past the rail.
pub(crate) const MEASURE: usize = 100;

/// A decoration span that is repeated on every wrapped row of its line.
pub(crate) fn repeat(style: Style) -> Style {
    style.add_modifier(NO_COPY | REPEAT)
}

/// `style`, marking the end of the hanging-indent region (the text after it is the content).
pub(crate) fn hang(style: Style) -> Style {
    style.add_modifier(HANG)
}

/// `style` for the zero-width span that flags a line as prose.
pub(crate) fn prose(style: Style) -> Style {
    style.add_modifier(NO_COPY | PROSE)
}

/// The span that splits a row into a left part and a right-aligned part. Its text (two spaces)
/// is what a plain-text flatten of the row shows between the halves.
pub(crate) fn right_marker() -> Span<'static> {
    Span::styled("  ", Style::default().add_modifier(NO_COPY | RIGHT))
}

pub(crate) fn has_decor(line: &Line<'_>) -> bool {
    line.spans.iter().any(|s| {
        s.style
            .add_modifier
            .intersects(NO_COPY | REPEAT | HANG | PROSE)
    })
}

/// Whether a row shows nothing but its repeating decoration (a bare rail).
pub(crate) fn is_blank_row(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .all(|s| s.content.trim().is_empty() || s.style.add_modifier.intersects(REPEAT))
}

/// Whether a span is a repeating rail or bar (never part of what a reader copies or a phone shows).
pub(crate) fn is_rail(span: &Span<'_>) -> bool {
    span.style.add_modifier.intersects(REPEAT)
}

/// How a line wraps, read from the marks its spans carry.
pub(crate) struct RowPlan {
    /// Spans `0..prefix` are the head that continuation rows replace with `cont`.
    pub(crate) prefix: usize,
    pub(crate) cont: Vec<Span<'static>>,
    pub(crate) prefix_w: usize,
    /// Cells taken by the repeating decoration (the rail) alone.
    pub(crate) rail_w: usize,
    pub(crate) prose: bool,
    /// Index of the right-alignment marker, when the line has one.
    pub(crate) right: Option<usize>,
}

fn span_w(s: &Span<'_>) -> usize {
    s.content
        .chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

pub(crate) fn plan(line: &Line<'_>) -> RowPlan {
    let right = line
        .spans
        .iter()
        .position(|s| s.style.add_modifier.intersects(RIGHT));
    let end = right.unwrap_or(line.spans.len());
    let head = line.spans[..end]
        .iter()
        .rposition(|s| s.style.add_modifier.intersects(REPEAT | HANG));
    let prefix = head.map_or(0, |h| h + 1);
    let mut cont = Vec::new();
    let mut prefix_w = 0;
    let mut rail_w = 0;
    for s in &line.spans[..prefix] {
        let w = span_w(s);
        prefix_w += w;
        if s.style.add_modifier.intersects(REPEAT) {
            rail_w += w;
            cont.push(Span::styled(s.content.to_string(), s.style));
        } else if w > 0 {
            cont.push(Span::styled(
                " ".repeat(w),
                Style::default().add_modifier(NO_COPY),
            ));
        }
    }
    RowPlan {
        prefix,
        cont,
        prefix_w,
        rail_w,
        prose: line
            .spans
            .iter()
            .any(|s| s.style.add_modifier.intersects(PROSE)),
        right,
    }
}

/// Lay a right-aligned line out as ONE row of exactly `width` cells: the left part, padding, the
/// right part flush at the margin. A left part too long to leave room is cut with an ellipsis;
/// when even that cannot fit the right part, the right part is dropped.
pub(crate) fn fit_right(line: &Line<'_>, at: usize, width: usize) -> Line<'static> {
    let left: Vec<Span<'static>> = line.spans[..at]
        .iter()
        .map(|s| Span::styled(s.content.to_string(), s.style))
        .collect();
    let right: Vec<Span<'static>> = line.spans[at + 1..]
        .iter()
        .map(|s| Span::styled(s.content.to_string(), s.style))
        .collect();
    let lw: usize = left.iter().map(span_w).sum();
    let rw: usize = right.iter().map(span_w).sum();
    let mut spans = left;
    if lw + 2 + rw <= width {
        spans.push(Span::styled(
            " ".repeat(width - lw - rw),
            Style::default().add_modifier(NO_COPY),
        ));
        spans.extend(right);
    } else if rw + 6 <= width {
        spans = cut_spans(spans, width - rw - 2);
        let used: usize = spans.iter().map(span_w).sum();
        spans.push(Span::styled(
            " ".repeat(width - used - rw),
            Style::default().add_modifier(NO_COPY),
        ));
        spans.extend(right);
    } else {
        spans = cut_spans(spans, width);
    }
    Line::from(spans).style(line.style)
}

/// Cut `spans` to `max` cells, ending in `…` when anything was dropped. Decoration spans are kept
/// whole (the head of a row is never what gets cut).
fn cut_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(span_w).sum();
    if total <= max {
        return spans;
    }
    let ellipsis = crate::theme::glyphs::g().ellipsis;
    let budget = max.saturating_sub(ellipsis.chars().count());
    let mut used = 0usize;
    let mut out = Vec::new();
    for span in spans {
        let w = span_w(&span);
        if span.style.add_modifier.intersects(NO_COPY) || used + w <= budget {
            used += w;
            out.push(span);
            continue;
        }
        let mut kept = String::new();
        for ch in span.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
            if used + cw > budget {
                break;
            }
            kept.push(ch);
            used += cw;
        }
        out.push(Span::styled(kept, span.style));
        break;
    }
    out.push(Span::raw(ellipsis));
    out
}

/// The zero-width span that opens a wrapped continuation row of a framed line.
pub(crate) fn soft_wrap_marker() -> Span<'static> {
    Span::styled(String::new(), Style::default().add_modifier(SOFT_WRAP))
}

fn is_soft_wrap(line: &Line<'_>) -> bool {
    line.spans
        .first()
        .is_some_and(|s| s.content.is_empty() && s.style.add_modifier.intersects(SOFT_WRAP))
}

fn is_frame_row(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .any(|s| s.style.add_modifier.intersects(FRAME_ROW))
}

/// One row's copyable text between two CELL columns (`from` inclusive, `to` exclusive). A wide
/// glyph belongs to the range its first cell falls in, matching how the highlight is painted.
fn row_text(line: &Line<'_>, from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut cell = 0usize;
    for span in &line.spans {
        let copy = !span.style.add_modifier.intersects(NO_COPY);
        for ch in span.content.chars() {
            if copy && cell >= from && cell < to {
                out.push(ch);
            }
            cell += UnicodeWidthChar::width(ch).unwrap_or(1);
        }
    }
    out
}

/// The text a selection covers. `rows` yields each selected wrapped row, in order, with the
/// `(start, end)` cell columns that apply to it (`end` of `usize::MAX` means "to end of row").
/// Wrapped continuations of a framed line rejoin without a newline and frame-edge rows vanish.
pub(crate) fn selection_text<'a>(
    rows: impl Iterator<Item = (&'a Line<'a>, usize, usize)>,
) -> String {
    let mut out = String::new();
    let mut any = false;
    for (line, from, to) in rows {
        if is_frame_row(line) {
            continue;
        }
        if any && !is_soft_wrap(line) {
            out.push('\n');
        }
        any = true;
        out.push_str(&row_text(line, from, to));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::markdown_to_lines;
    use crate::transcript::wrap_lines;

    const ALL: (usize, usize) = (0, usize::MAX);

    /// Wrapped rows under a truecolor theme, where a code block is a band whose gutter is one
    /// cell wide (no line numbers below six rows).
    fn rows(md: &str, width: usize) -> Vec<Line<'static>> {
        crate::theme::scoped(
            crate::theme::Theme::new(
                crate::theme::ColorTier::TrueColor,
                crate::theme::Mode::Dark,
                crate::theme::GlyphTier::Unicode,
                crate::theme::Motion::Full,
            ),
            || wrap_lines(&markdown_to_lines(md), width),
        )
    }

    fn plain(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Copy rows `r0..=r1`, the first from cell `c0` and the last up to cell `c1`.
    fn copy(rows: &[Line<'static>], (r0, c0): (usize, usize), (r1, c1): (usize, usize)) -> String {
        selection_text((r0..=r1).map(|r| {
            let from = if r == r0 { c0 } else { 0 };
            let to = if r == r1 { c1 } else { usize::MAX };
            (&rows[r], from, to)
        }))
    }

    fn copy_all(rows: &[Line<'static>]) -> String {
        copy(rows, (0, ALL.0), (rows.len() - 1, ALL.1))
    }

    #[test]
    fn whole_block_copies_exactly_the_code() {
        let r = rows("```rust\nfn main() {\n    println!(\"hi\");\n}\n```", 80);
        assert!(
            plain(&r[0]).contains("rust") && plain(&r[0]).contains("/copy"),
            "header row"
        );
        assert_eq!(copy_all(&r), "fn main() {\n    println!(\"hi\");\n}");
    }

    #[test]
    fn partial_lines_and_multi_line_ranges_cut_at_the_cell() {
        let r = rows("```\nalpha\n  bravo\ncharlie\n```", 80);
        // Row 0 is the header. Cell 1 is the first code cell (one cell of band padding).
        assert_eq!(copy(&r, (1, 3), (1, 6)), "pha");
        assert_eq!(copy(&r, (1, 3), (3, 1 + 3)), "pha\n  bravo\ncha");
        assert_eq!(copy(&r, (1, 0), (3, usize::MAX)), "alpha\n  bravo\ncharlie");
    }

    #[test]
    fn selection_starting_or_ending_on_the_frame_still_yields_only_code() {
        let r = rows("```\none\ntwo\n```", 80);
        assert_eq!(copy(&r, (0, 0), (r.len() - 1, usize::MAX)), "one\ntwo");
        assert_eq!(copy(&r, (1, 0), (2, 3)), "one\ntw");
    }

    #[test]
    fn code_that_contains_box_glyphs_survives() {
        let src = "┌─┬─┐\n│ a │ b │\n└─┴─┘\n  ╭─ x";
        let r = rows(&format!("```\n{src}\n```"), 80);
        assert_eq!(copy_all(&r), src);
    }

    #[test]
    fn leading_spaces_tabs_and_blank_lines_are_preserved() {
        let src = "if x:\n\treturn 1\n\n        deep";
        let r = rows(&format!("```py\n{src}\n```"), 80);
        assert_eq!(copy_all(&r), src);
    }

    #[test]
    fn soft_wrapped_code_line_is_rejoined() {
        let long = format!("let s = \"{}\";", "x".repeat(60));
        let r = rows(&format!("```\n{long}\nnext\n```"), 30);
        assert!(r.len() > 4, "the long line wrapped: {}", r.len());
        assert_eq!(copy_all(&r), format!("{long}\nnext"));
    }

    #[test]
    fn exact_width_wrap_does_not_invent_a_newline_or_lose_text() {
        // gutter is 1 cell, wrap width 7 → a 6-cell line fills the first row exactly.
        let r = rows("```\nabcdef\nz\n```", 7);
        assert_eq!(copy_all(&r), "abcdef\nz");
    }

    #[test]
    fn prose_around_a_block_copies_unchanged_and_the_quote_bar_is_dropped() {
        let r = rows("before\n\n```\ncode\n```\n\n> quoted", 80);
        let text = copy_all(&r);
        assert!(text.contains("before"), "{text:?}");
        assert!(text.contains("code") && !text.contains('│') && !text.contains('┌'));
        assert!(text.ends_with("quoted") && !text.contains('▏'), "{text:?}");
    }

    #[test]
    fn diff_copies_the_code_not_the_gutter_sigil_or_separator() {
        let diff = forge_types::FileDiff {
            path: "a.rs".into(),
            kind: forge_types::DiffKind::Modified,
            old: Some("keep\nold\n".into()),
            new: Some("keep\nnew\n".into()),
            lang: None,
            binary: false,
        };
        let r = wrap_lines(&crate::render::diff_to_lines(&diff), 80);
        let text = copy_all(&r);
        assert_eq!(text, "keep\nold\nnew", "{text:?}");
    }

    #[test]
    fn wide_glyph_before_the_cut_does_not_shift_it() {
        let r = rows("```\n日本語abc\n```", 80);
        // gutter 1 + three 2-cell glyphs = cell 7 is the start of "abc".
        assert_eq!(copy(&r, (1, 7), (1, usize::MAX)), "abc");
    }
}
