//! GFM table layout for the transcript renderer: collected styled cells → padded, aligned lines.

use crate::theme::styles;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

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
    let ellipsis = crate::theme::glyphs::g().ellipsis;
    let budget = max.saturating_sub(ellipsis.chars().count());
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
    out.push(Span::styled(ellipsis, styles::muted()));
    out
}

/// Whether a cell reads as a number (`86`, `1,204`, `$0.12`, `93%`, `86 us`): those columns are
/// right-aligned so the digits line up.
fn is_numeric(cell: &[Span<'_>]) -> bool {
    let text: String = cell.iter().map(|s| s.content.as_ref()).collect();
    let text = text.trim();
    let mut parts = text.split_whitespace();
    let Some(head) = parts.next() else {
        return false;
    };
    let number = head
        .trim_start_matches(['$', '+', '-', '~'])
        .trim_end_matches(['%', 'x']);
    if number.is_empty()
        || !number
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ','))
    {
        return false;
    }
    match (parts.next(), parts.next()) {
        (None, _) => true,
        (Some(unit), None) => {
            unit.len() <= 4 && unit.chars().all(|c| c.is_alphabetic() || c == '/')
        }
        _ => false,
    }
}

/// Lay a collected table out as aligned columns: a bold header, a `─` rule per column under it,
/// two spaces between columns and no grid or outer border. Numeric columns are right-aligned.
/// Styling inside cells (code, bold) is preserved span-for-span; only padding is added.
pub(super) fn layout(table: TableBuild) -> Vec<Line<'static>> {
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
    let right: Vec<bool> = (0..cols)
        .map(|i| {
            let body: Vec<_> = table
                .rows
                .iter()
                .skip(table.header_rows)
                .filter_map(|r| r.get(i))
                .collect();
            !body.is_empty() && body.iter().all(|c| is_numeric(c))
        })
        .collect();
    let rule_style = styles::line();
    for (r, row) in table.rows.into_iter().enumerate() {
        let header = r < table.header_rows;
        let mut line: Vec<Span<'static>> = Vec::new();
        for (i, width) in widths.iter().enumerate() {
            let cell = row.get(i).cloned().unwrap_or_default();
            let pad = width.saturating_sub(spans_width(&cell));
            if i > 0 {
                line.push(Span::raw("  "));
            }
            if right[i] && pad > 0 {
                line.push(Span::raw(" ".repeat(pad)));
            }
            if header {
                line.extend(cell.into_iter().map(|s| {
                    let style = s.style.add_modifier(ratatui::style::Modifier::BOLD);
                    Span::styled(s.content, style)
                }));
            } else {
                line.extend(cell);
            }
            if !right[i] && pad > 0 && i + 1 < cols {
                line.push(Span::raw(" ".repeat(pad)));
            }
        }
        lines.push(Line::from(line));
        if r + 1 == table.header_rows {
            let dash = if crate::theme::t().glyphs().unicode {
                "─"
            } else {
                "-"
            };
            let mut rule: Vec<Span<'static>> = Vec::new();
            for (i, w) in widths.iter().enumerate() {
                if i > 0 {
                    rule.push(Span::raw("  "));
                }
                rule.push(Span::styled(dash.repeat(*w), rule_style));
            }
            lines.push(Line::from(rule));
        }
    }
    lines
}
