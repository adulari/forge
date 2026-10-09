//! Turn one line of a user script's output (SGR colour codes included) into ratatui spans.
//! Only colour/weight attributes are honoured; every other escape sequence is dropped so a script
//! can never move the cursor or write outside its row.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub(crate) fn spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut style = base;
    let mut buf = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    let mut params = String::new();
                    let mut fin = None;
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            fin = Some(n);
                            break;
                        }
                        params.push(n);
                    }
                    if fin == Some('m') {
                        if !buf.is_empty() {
                            out.push(Span::styled(std::mem::take(&mut buf), style));
                        }
                        style = apply_sgr(style, base, &params);
                    }
                }
                Some(']') => {
                    chars.next();
                    // OSC runs to BEL or ST (ESC \).
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' || (n == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            c if c.is_control() => {}
            c => buf.push(c),
        }
    }
    if !buf.is_empty() {
        out.push(Span::styled(buf, style));
    }
    out
}

fn apply_sgr(mut style: Style, base: Style, params: &str) -> Style {
    let nums: Vec<u16> = if params.is_empty() {
        vec![0]
    } else {
        params
            .split([';', ':'])
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => style = base,
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            22 => style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => style = style.remove_modifier(Modifier::ITALIC),
            24 => style = style.remove_modifier(Modifier::UNDERLINED),
            n @ 30..=37 => style = style.fg(Color::Indexed((n - 30) as u8)),
            n @ 90..=97 => style = style.fg(Color::Indexed((n - 90 + 8) as u8)),
            n @ 40..=47 => style = style.bg(Color::Indexed((n - 40) as u8)),
            n @ 100..=107 => style = style.bg(Color::Indexed((n - 100 + 8) as u8)),
            39 => style.fg = base.fg,
            49 => style.bg = base.bg,
            n @ (38 | 48) => {
                let (color, used) = match nums.get(i + 1) {
                    Some(5) => (nums.get(i + 2).map(|v| Color::Indexed(*v as u8)), 2),
                    Some(2) => (
                        match (nums.get(i + 2), nums.get(i + 3), nums.get(i + 4)) {
                            (Some(r), Some(g), Some(b)) => {
                                Some(Color::Rgb(*r as u8, *g as u8, *b as u8))
                            }
                            _ => None,
                        },
                        4,
                    ),
                    _ => (None, 0),
                };
                if let Some(color) = color {
                    style = if n == 38 {
                        style.fg(color)
                    } else {
                        style.bg(color)
                    };
                }
                i += used;
            }
            _ => {}
        }
        i += 1;
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn plain_text_is_one_span() {
        let s = spans("hello", Style::default());
        assert_eq!(s.len(), 1);
        assert_eq!(text(&s), "hello");
    }

    #[test]
    fn sgr_colours_split_spans_and_reset_to_base() {
        let base = Style::default().fg(Color::Gray);
        let s = spans("\u{1b}[31mred\u{1b}[0m plain \u{1b}[1;38;2;1;2;3mrgb", base);
        assert_eq!(text(&s), "red plain rgb");
        assert_eq!(s[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(s[1].style, base);
        assert_eq!(s[2].style.fg, Some(Color::Rgb(1, 2, 3)));
        assert!(s[2].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn indexed_256_colour() {
        let s = spans("\u{1b}[38;5;208mx", Style::default());
        assert_eq!(s[0].style.fg, Some(Color::Indexed(208)));
    }

    #[test]
    fn non_sgr_escapes_and_controls_are_dropped() {
        let s = spans(
            "a\u{1b}[2Jb\u{1b}]0;title\u{7}c\u{7}\rd\u{1b}[H",
            Style::default(),
        );
        assert_eq!(text(&s), "abcd");
    }
}
