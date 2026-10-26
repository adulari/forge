//! Text helpers for the onboarding pages: wrapping, truncating, key chips.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::glyphs::g;
use crate::theme::styles;

/// Greedy word wrap to `width` columns. Never returns an empty vec for non-empty input.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let needed = if line.is_empty() {
                word.width()
            } else {
                line.width() + 1 + word.width()
            };
            if needed > width && !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

/// Cut `text` to `width` columns, ending in an ellipsis when it was shortened.
pub(crate) fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let ellipsis = g().ellipsis;
    let budget = width.saturating_sub(ellipsis.width());
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push_str(ellipsis);
    out
}

/// Pad `text` with spaces to `width` columns (no-op when already wider).
pub(crate) fn pad(text: &str, width: usize) -> String {
    let w = text.width();
    format!("{text}{}", " ".repeat(width.saturating_sub(w)))
}

/// A wrapped paragraph as styled lines with a left indent.
pub(crate) fn paragraph(
    text: &str,
    width: usize,
    indent: usize,
    style: Style,
) -> Vec<Line<'static>> {
    wrap(text, width.saturating_sub(indent))
        .into_iter()
        .map(|l| Line::from(Span::styled(format!("{}{l}", " ".repeat(indent)), style)))
        .collect()
}

pub(crate) fn blank() -> Line<'static> {
    Line::from("")
}

/// A bold section heading.
pub(crate) fn heading(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), styles::fg().bold()))
}

/// A key-chip row: `Enter Continue · b Back · q Leave`.
pub(crate) fn chips(pairs: &[(&str, &str)]) -> Line<'static> {
    Line::from(styles::hint_row(pairs))
}

/// The mask shown for typed secret text: one bullet per character, capped so a long paste does
/// not run off the row.
pub(crate) fn mask(len: usize, max: usize) -> String {
    let bullet = if g().unicode { "•" } else { "*" };
    if len <= max {
        bullet.repeat(len)
    } else {
        format!("{}{}", g().ellipsis, bullet.repeat(max.saturating_sub(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_breaks_on_words_and_respects_the_width() {
        let lines = wrap("one two three four five six", 11);
        assert!(lines.iter().all(|l| l.width() <= 11), "{lines:?}");
        assert_eq!(lines.join(" "), "one two three four five six");
    }

    #[test]
    fn wrapping_keeps_explicit_paragraph_breaks() {
        assert_eq!(wrap("a\nb", 20), ["a", "b"]);
    }

    #[test]
    fn truncating_ends_in_an_ellipsis_and_fits() {
        let cut = truncate("Claude Code 2.1.4 · signed in", 12);
        assert!(cut.width() <= 12, "{cut}");
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("short", 12), "short");
    }

    #[test]
    fn the_mask_never_reveals_the_text_and_never_overflows() {
        let m = mask(5, 20);
        assert_eq!(m.chars().count(), 5);
        assert!(m.chars().all(|c| c == '•'));
        assert!(mask(500, 20).chars().count() <= 21);
    }
}
