//! Fenced code blocks: a surface band across the row with a language/`/copy` header, line
//! numbers past five rows, and a faint `│` rule instead of the band on terminals that cannot
//! paint backgrounds.

use crate::copy_mark;
use crate::theme::{styles, t};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::highlight_code;

/// Blocks longer than this show line numbers.
const NUMBERED_FROM: usize = 6;

pub(super) fn code_block(lang: &str, code: &[String]) -> Vec<Line<'static>> {
    // Trim a trailing empty line that the fence parser commonly leaves.
    let mut lines: &[String] = code;
    while matches!(lines.last(), Some(l) if l.is_empty()) {
        lines = &lines[..lines.len() - 1];
    }
    let banded = t().paints_backgrounds();
    let band = if banded {
        styles::surface()
    } else {
        Style::default()
    };
    let rule = if t().glyphs().unicode { "│" } else { "|" };
    let label = if lang.is_empty() { "text" } else { lang };
    let numbered = lines.len() >= NUMBERED_FROM;
    let num_w = lines.len().to_string().len().max(2);

    let mut out = Vec::with_capacity(lines.len() + 1);
    let mut head = vec![
        Span::styled(
            if banded { " " } else { "" },
            copy_mark::frame_row(styles::muted()),
        ),
        Span::styled(label.to_string(), copy_mark::frame_row(styles::muted())),
        copy_mark::right_marker(),
        Span::styled("/copy", copy_mark::frame_row(styles::faint())),
        Span::styled(" ", copy_mark::frame_row(styles::faint())),
    ];
    if !banded {
        // The rule sits in the same column as on the code rows below.
        let lead = if numbered {
            format!("{:>num_w$} {rule} ", "")
        } else {
            format!("{rule} ")
        };
        head.insert(0, Span::styled(lead, copy_mark::frame_row(styles::line())));
    }
    out.push(Line::from(head).style(band));

    for (i, spans) in highlight_code(lang, lines).into_iter().enumerate() {
        let mut row: Vec<Span<'static>> = Vec::new();
        match (banded, numbered) {
            (true, true) => row.push(Span::styled(
                format!(" {:>num_w$}  ", i + 1),
                copy_mark::decor(styles::faint()),
            )),
            (true, false) => row.push(Span::styled(" ", copy_mark::decor(Style::default()))),
            (false, true) => row.push(Span::styled(
                format!("{:>num_w$} {rule} ", i + 1),
                copy_mark::decor(styles::faint()),
            )),
            (false, false) => row.push(Span::styled(
                format!("{rule} "),
                copy_mark::decor(styles::line()),
            )),
        }
        row.extend(spans);
        out.push(Line::from(row).style(band));
    }
    out
}
