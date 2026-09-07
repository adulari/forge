//! GFM table layout for the transcript renderer: collected styled cells → padded, aligned lines.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::surface::DIM;

/// Widest a single table column may render. Wider cells are cut with an ellipsis so one long
/// cell cannot push the whole table past the terminal edge and wrap every row.
pub(super) const TABLE_CELL_MAX: usize = 48;

/// A GFM table under construction: styled cells per row, header first.
#[derive(Default)]
pub(super) struct TableBuild {
    pub(super) rows: Vec<Vec<Vec<Span<'static>>>>,
    pub(super) cell: Vec<Span<'static>>,
    pub(super) header_rows: usize,
}

pub(super) fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Cut a cell's spans to `max` display columns, ending with `…` when anything was dropped.
pub(super) fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    if spans_width(&spans) <= max {
        return spans;
    }
    let mut out = Vec::new();
    let mut used = 0usize;
    let budget = max.saturating_sub(1);
    for span in spans {
        let mut kept = String::new();
        for ch in span.content.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + w > budget {
                break;
            }
            kept.push(ch);
            used += w;
        }
        let done = kept.chars().count() < span.content.chars().count();
        if !kept.is_empty() {
            out.push(Span::styled(kept, span.style));
        }
        if done {
            break;
        }
    }
    out.push(Span::styled("…", Style::default().fg(DIM)));
    out
}

/// Lay a collected table out as aligned columns: cells padded to the column's widest cell,
/// `│` between columns, and a `─┼─` rule under the header. Styling inside cells (code, bold)
/// is preserved span-for-span; only padding is added.
pub(super) fn layout(table: TableBuild, indent: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let cols = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return lines;
    }
    let mut widths = vec![0usize; cols];
    for row in &table.rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(spans_width(cell));
        }
    }
    let frame = Style::default().fg(DIM);
    for (r, row) in table.rows.into_iter().enumerate() {
        let mut line: Vec<Span<'static>> = vec![Span::raw(indent.to_string())];
        for (i, width) in widths.iter().enumerate() {
            let cell = row.get(i).cloned().unwrap_or_default();
            let pad = width.saturating_sub(spans_width(&cell));
            if i > 0 {
                line.push(Span::styled(" │ ", frame));
            }
            line.extend(cell);
            if pad > 0 {
                line.push(Span::raw(" ".repeat(pad)));
            }
        }
        lines.push(Line::from(line));
        if r + 1 == table.header_rows {
            let rule = widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┼─");
            lines.push(Line::from(vec![
                Span::raw(indent.to_string()),
                Span::styled(rule, frame),
            ]));
        }
    }
    lines
}
