//! Terminal hyperlinks (OSC 8) for the sign-in link. ratatui cells cannot carry a hyperlink, so
//! after each draw the host finds the rows the link was wrapped onto and rewrites them with the
//! escape around them. Every row carries the full address, so clicking any row opens all of it.

use ratatui::buffer::Buffer;
use ratatui::style::Style;

#[derive(Debug, Clone, PartialEq)]
pub struct LinkRow {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub style: Style,
}

/// The rows of `buf` that hold `url`, wrapped from one column. Stops at the first row that does
/// not continue it, so other text is never linked.
pub fn link_rows(buf: &Buffer, url: &str) -> Vec<LinkRow> {
    let area = buf.area;
    let mut out = Vec::new();
    let mut rest = url;
    let mut column: Option<usize> = None;
    for y in area.top()..area.bottom() {
        if rest.is_empty() {
            break;
        }
        let row: Vec<&str> = (area.left()..area.right())
            .map(|x| buf[(x, y)].symbol())
            .collect();
        let start = match column.or_else(|| find_start(&row, rest)) {
            Some(start) => start,
            None => continue,
        };
        let run: String = row[start..]
            .iter()
            .take_while(|s| s.len() == 1 && !s.trim().is_empty())
            .copied()
            .collect();
        if run.is_empty() || !rest.starts_with(&run) {
            if column.is_some() {
                break;
            }
            continue;
        }
        let x = area.left() + start as u16;
        out.push(LinkRow {
            x,
            y,
            style: buf[(x, y)].style(),
            text: run.clone(),
        });
        rest = &rest[run.len()..];
        column = Some(start);
    }
    out
}

fn find_start(row: &[&str], url: &str) -> Option<usize> {
    let probe: Vec<char> = url.chars().take(12).collect();
    (0..row.len().saturating_sub(probe.len() - 1)).find(|&i| {
        probe
            .iter()
            .zip(&row[i..])
            .all(|(c, s)| s.chars().eq(std::iter::once(*c)))
    })
}

/// `text` wrapped in an OSC 8 hyperlink to `url`. The shared `id` makes a terminal treat the
/// wrapped rows as one link.
pub fn osc8(url: &str, text: &str) -> String {
    format!("\x1b]8;id=forge-sign-in;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn a_wrapped_link_is_found_row_by_row_and_nothing_else() {
        let url = "https://claude.com/cai/oauth/authorize?code=true&state=abc";
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 6));
        buf.set_string(2, 0, "Open this page", Style::default());
        buf.set_string(2, 1, &url[..30], Style::default());
        buf.set_string(2, 2, &url[30..], Style::default());
        buf.set_string(2, 3, "Paste the code", Style::default());
        let rows = link_rows(&buf, url);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].x, rows[0].y), (2, 1));
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<String>(),
            url
        );
    }

    #[test]
    fn no_link_on_screen_means_no_rows() {
        let buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        assert!(link_rows(&buf, "https://example.com/x").is_empty());
    }
}
