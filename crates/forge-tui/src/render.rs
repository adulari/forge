//! Pure rendering helpers: turn model output into styled ratatui [`Line`]s. Free of terminal
//! I/O so it's unit-testable on strings. Currently: markdown → lines (syntax highlighting of
//! fenced code and diff rendering land in follow-up increments).
//!
//! `pulldown-cmark` is *total* (it degrades malformed markdown to literal text and never
//! panics), so this renderer never drops content or crashes on bad input.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// Palette — mirrors crate::app so rendered markdown belongs to the same TUI.
const ORANGE: Color = Color::Rgb(255, 145, 60);
const DIM: Color = Color::Rgb(110, 110, 120);
const CODEFG: Color = Color::Rgb(205, 205, 215);
const CODEBG: Color = Color::Rgb(40, 40, 48);

/// The base indent every rendered line carries (matches the plain `body_line` convention).
const INDENT: &str = "  ";

/// Render a markdown document to styled lines, indented to match the conversation body.
pub fn markdown_to_lines(md: &str) -> Vec<Line<'static>> {
    let mut r = Renderer::default();
    for ev in Parser::new_ext(md, Options::empty()) {
        r.event(ev);
    }
    r.finish()
}

#[derive(Default)]
struct Renderer {
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    bold: u32,
    italic: u32,
    heading: bool,
    /// list markers stack: None = bullet, Some(n) = next ordered number
    lists: Vec<Option<u64>>,
    quote: u32,
    /// when Some, we're inside a fenced code block accumulating raw lines
    code: Option<Vec<String>>,
}

impl Renderer {
    fn style(&self) -> Style {
        let mut s = Style::default().fg(CODEFG);
        if self.heading {
            s = s.fg(ORANGE).add_modifier(Modifier::BOLD);
        }
        if self.bold > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        s
    }

    fn indent_prefix(&self) -> String {
        // base indent + 2 spaces per nested list level + a quote bar.
        let mut p = String::from(INDENT);
        for _ in 0..self.lists.len().saturating_sub(1) {
            p.push_str("  ");
        }
        p
    }

    fn push_text(&mut self, text: &str) {
        if self.cur.is_empty() {
            // start the line with its indent (+ quote bar if quoted).
            self.cur.push(Span::raw(self.indent_prefix()));
            if self.quote > 0 {
                self.cur.push(Span::styled("▏ ", Style::default().fg(DIM)));
            }
        }
        self.cur.push(Span::styled(text.to_string(), self.style()));
    }

    fn flush_line(&mut self) {
        if !self.cur.is_empty() {
            self.lines.push(Line::from(std::mem::take(&mut self.cur)));
        }
    }

    fn blank(&mut self) {
        // collapse consecutive blanks
        if self.lines.last().map(|l| l.spans.is_empty()) != Some(true) {
            self.lines.push(Line::default());
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        // Inside a fenced code block, capture raw text verbatim until it closes.
        if let Some(buf) = self.code.as_mut() {
            match ev {
                Event::Text(t) => {
                    for (i, part) in t.split('\n').enumerate() {
                        if i == 0 {
                            if let Some(last) = buf.last_mut() {
                                last.push_str(part);
                                continue;
                            }
                        }
                        buf.push(part.to_string());
                    }
                    return;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let code = self.code.take().unwrap_or_default();
                    self.render_code_block(&code);
                    return;
                }
                _ => return,
            }
        }

        match ev {
            Event::Start(Tag::Heading { .. }) => {
                self.flush_line();
                self.blank();
                self.heading = true;
            }
            Event::End(TagEnd::Heading(level)) => {
                self.flush_line();
                if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                    self.lines.push(Line::from(Span::styled(
                        format!("{INDENT}{}", "─".repeat(44)),
                        Style::default().fg(DIM),
                    )));
                }
                self.heading = false;
                self.blank();
            }
            Event::Start(Tag::Paragraph) => self.flush_line(),
            Event::End(TagEnd::Paragraph) => {
                self.flush_line();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::BlockQuote(_)) => {
                self.flush_line();
                self.quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.flush_line();
                self.quote = self.quote.saturating_sub(1);
                self.blank();
            }
            Event::Start(Tag::List(first)) => self.lists.push(first),
            Event::End(TagEnd::List(_)) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            Event::Start(Tag::Item) => {
                self.flush_line();
                // emit the marker as the start of this line.
                self.cur.push(Span::raw(self.indent_prefix()));
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.cur
                    .push(Span::styled(marker, Style::default().fg(ORANGE)));
            }
            Event::End(TagEnd::Item) => self.flush_line(),
            Event::Start(Tag::CodeBlock(_)) => {
                self.flush_line();
                self.code = Some(vec![String::new()]);
            }
            Event::Start(Tag::Link { .. }) => {} // text rendered inline; URL appended on end
            Event::End(TagEnd::Link) => {}
            Event::Text(t) => self.push_text(&t),
            Event::Code(c) => {
                if self.cur.is_empty() {
                    self.cur.push(Span::raw(self.indent_prefix()));
                }
                self.cur.push(Span::styled(
                    c.to_string(),
                    Style::default().fg(ORANGE).bg(CODEBG),
                ));
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.flush_line(),
            Event::Rule => {
                self.flush_line();
                self.blank();
                self.lines.push(Line::from(Span::styled(
                    format!("{INDENT}{}", "─".repeat(48)),
                    Style::default().fg(DIM),
                )));
                self.blank();
            }
            _ => {}
        }
    }

    fn render_code_block(&mut self, code: &[String]) {
        // Trim a trailing empty line that the fence parser commonly leaves.
        let mut lines: &[String] = code;
        while matches!(lines.last(), Some(l) if l.is_empty()) {
            lines = &lines[..lines.len() - 1];
        }
        let frame = Style::default().fg(DIM);
        let body = Style::default().fg(CODEFG);
        self.lines.push(Line::from(Span::styled(
            format!("{INDENT}┌{}", "─".repeat(48)),
            frame,
        )));
        for l in lines {
            self.lines.push(Line::from(vec![
                Span::styled(format!("{INDENT}│ "), frame),
                Span::styled(l.clone(), body),
            ]));
        }
        self.lines.push(Line::from(Span::styled(
            format!("{INDENT}└{}", "─".repeat(48)),
            frame,
        )));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        // drop a trailing blank
        while matches!(self.lines.last(), Some(l) if l.spans.is_empty()) {
            self.lines.pop();
        }
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn has_modifier(lines: &[Line], needle: &str, m: Modifier) -> bool {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.content.contains(needle) && s.style.add_modifier.contains(m))
    }

    #[test]
    fn heading_renders_without_hashes_and_is_bold() {
        let out = markdown_to_lines("## Plan\n");
        let t = text_of(&out);
        assert!(t.contains("Plan"), "heading text shown");
        assert!(!t.contains("##"), "no literal hashes: {t:?}");
        assert!(
            has_modifier(&out, "Plan", Modifier::BOLD),
            "heading is bold"
        );
    }

    #[test]
    fn bold_and_inline_code_strip_markup() {
        let out = markdown_to_lines("step **one** and `two`\n");
        let t = text_of(&out);
        assert!(t.contains("one") && t.contains("two"));
        assert!(
            !t.contains("**") && !t.contains('`'),
            "markup stripped: {t:?}"
        );
        assert!(has_modifier(&out, "one", Modifier::BOLD), "bold applied");
    }

    #[test]
    fn bullet_list_gets_markers() {
        let out = markdown_to_lines("- alpha\n- beta\n");
        let t = text_of(&out);
        assert!(t.contains("• alpha"), "bullet marker: {t:?}");
        assert!(t.contains("• beta"));
        assert!(!t.contains("- alpha"));
    }

    #[test]
    fn ordered_list_numbers() {
        let out = markdown_to_lines("1. first\n2. second\n");
        let t = text_of(&out);
        assert!(t.contains("1. first") && t.contains("2. second"), "{t:?}");
    }

    #[test]
    fn fenced_code_block_is_framed_and_verbatim() {
        let out = markdown_to_lines("```rust\nlet x = 1;\n```\n");
        let t = text_of(&out);
        assert!(t.contains("let x = 1;"), "code preserved verbatim: {t:?}");
        assert!(t.contains('┌') && t.contains('└'), "code block framed");
        assert!(!t.contains("```"), "fence markers not shown");
    }

    #[test]
    fn malformed_markdown_does_not_panic_and_keeps_text() {
        let out = markdown_to_lines("**unbalanced and `weird ## not a heading mid-line");
        let t = text_of(&out);
        assert!(t.contains("unbalanced"));
        assert!(t.contains("weird"));
    }

    #[test]
    fn plain_text_passes_through() {
        let out = markdown_to_lines("the workspace looks healthy");
        assert_eq!(text_of(&out), "  the workspace looks healthy");
    }
}
