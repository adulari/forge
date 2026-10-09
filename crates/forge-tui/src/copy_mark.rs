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

/// `style`, marked as a frame-edge row that contributes nothing to a copy.
pub(crate) fn frame_row(style: Style) -> Style {
    style.add_modifier(NO_COPY | FRAME_ROW)
}

pub(crate) fn has_decor(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .any(|s| s.style.add_modifier.intersects(NO_COPY))
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

    fn rows(md: &str, width: usize) -> Vec<Line<'static>> {
        wrap_lines(&markdown_to_lines(md), width)
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
        assert!(plain(&r[0]).contains('┌') && plain(r.last().unwrap()).contains('└'));
        assert_eq!(copy_all(&r), "fn main() {\n    println!(\"hi\");\n}");
    }

    #[test]
    fn partial_lines_and_multi_line_ranges_cut_at_the_cell() {
        let r = rows("```\nalpha\n  bravo\ncharlie\n```", 80);
        // Row 0 is the top edge. Cell 4 is the first code cell (indent 2 + "│ ").
        assert_eq!(copy(&r, (1, 6), (1, 9)), "pha");
        assert_eq!(copy(&r, (1, 6), (3, 4 + 3)), "pha\n  bravo\ncha");
        assert_eq!(copy(&r, (1, 0), (3, usize::MAX)), "alpha\n  bravo\ncharlie");
    }

    #[test]
    fn selection_starting_or_ending_on_the_frame_still_yields_only_code() {
        let r = rows("```\none\ntwo\n```", 80);
        assert_eq!(copy(&r, (0, 0), (r.len() - 1, usize::MAX)), "one\ntwo");
        assert_eq!(copy(&r, (1, 0), (2, 3)), "one\n");
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
        // gutter is 4 cells, wrap width 10 → a 6-cell line fills the first row exactly.
        let r = rows("```\nabcdef\nz\n```", 10);
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
    fn diff_keeps_markers_and_content_but_not_the_indent() {
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
        assert!(text.contains("\n-old\n+new"), "{text:?}");
        assert!(text.contains("\n keep\n"), "{text:?}");
    }

    #[test]
    fn wide_glyph_before_the_cut_does_not_shift_it() {
        let r = rows("```\n日本語abc\n```", 80);
        // gutter 4 + three 2-cell glyphs = cell 10 is the start of "abc".
        assert_eq!(copy(&r, (1, 10), (1, usize::MAX)), "abc");
    }
}
