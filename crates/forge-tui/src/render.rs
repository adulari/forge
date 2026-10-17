//! Pure rendering helpers: turn model output into styled ratatui [`Line`]s. Free of terminal
//! I/O so it's unit-testable on strings: `markdown_to_lines` (markdown → styled lines),
//! `highlight_code` (syntect syntax highlighting), and the diff, report and table renderers in
//! the submodules.
//!
//! `pulldown-cmark` is *total* (it degrades malformed markdown to literal text and never
//! panics), so this renderer never drops content or crashes on bad input.
//!
//! Rows carry wrap marks (`copy_mark`): prose rows wrap at the reading measure and hang under
//! their list marker, and the turn rail the transcript puts in front repeats on every wrapped
//! row. Nothing here knows the terminal width.

use crate::copy_mark;
use crate::theme::{glyphs::g, styles, t};
use std::sync::OnceLock;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
mod code;
mod diff;
mod reports;
mod stream;
mod table;
pub use diff::{
    diff_file_snapshot, diff_rows, diff_stat, diff_to_lines, diff_to_plain,
    REMOTE_DIFF_MAX_HUNK_LINES,
};
pub use reports::{assay_report_lines, assay_report_plain, mcp_status_lines};
pub use stream::prewarm_highlighter;
pub(crate) use stream::StreamCache;
use table::{clip_spans, TableBuild, TABLE_CELL_MAX};

/// Base indent of the report renderers (the markdown renderer has none: the rail is the margin).
const INDENT: &str = "  ";

/// Bundled syntaxes + theme, loaded once. The cost is paid at most once per process and
/// only when a code block is actually highlighted — never on `forge --help`/`forge run`.
fn highlighter() -> &'static (SyntaxSet, Theme) {
    static DARK: OnceLock<(SyntaxSet, Theme)> = OnceLock::new();
    static LIGHT: OnceLock<(SyntaxSet, Theme)> = OnceLock::new();
    let (cell, name) = match t().mode {
        crate::theme::Mode::Dark => (&DARK, "base16-ocean.dark"),
        crate::theme::Mode::Light => (&LIGHT, "InspiredGitHub"),
    };
    cell.get_or_init(|| {
        let ss = SyntaxSet::load_defaults_newlines();
        let ts = ThemeSet::load_defaults();
        let theme = ts
            .themes
            .get(name)
            .or_else(|| ts.themes.values().next())
            .cloned()
            .unwrap_or_default();
        (ss, theme)
    })
}

fn highlighted_spans(
    highlighter: &mut HighlightLines<'_>,
    ss: &SyntaxSet,
    line: &str,
) -> Vec<Span<'static>> {
    match highlighter.highlight_line(line, ss) {
        Ok(ranges) => ranges
            .into_iter()
            .map(|(st, text)| {
                let fg = st.foreground;
                let mut style = if t().tier == crate::theme::ColorTier::Mono {
                    Style::default()
                } else {
                    Style::default().fg(crate::theme::rgb(fg.r, fg.g, fg.b))
                };
                if st.font_style.contains(FontStyle::BOLD) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if st.font_style.contains(FontStyle::ITALIC) {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                Span::styled(text.to_string(), style)
            })
            .collect(),
        Err(_) => vec![Span::styled(line.to_string(), styles::fg())],
    }
}

/// Syntax-highlight `lines` of source in `lang` into per-line span vectors. Unknown/empty
/// language falls back to plain (single dim span per line); never panics.
pub fn highlight_code(lang: &str, lines: &[String]) -> Vec<Vec<Span<'static>>> {
    let (ss, theme) = highlighter();
    let syntax = (!lang.is_empty())
        .then(|| ss.find_syntax_by_token(lang))
        .flatten()
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    let mut h = HighlightLines::new(syntax, theme);
    lines
        .iter()
        .map(|line| highlighted_spans(&mut h, ss, line))
        .collect()
}

/// Render a markdown document to styled lines, indented to match the conversation body.
pub fn markdown_to_lines(md: &str) -> Vec<Line<'static>> {
    let mut r = Renderer::default();
    for ev in Parser::new_ext(md, markdown_options()) {
        r.event(ev);
    }
    r.finish()
}
// Without ENABLE_TABLES a GFM table is just a paragraph: its rows are joined by soft breaks into
// one long line of pipes — the "broken table" every model-written comparison used to render as.
// Strikethrough is enabled so `~~x~~` does not leak tildes either.
fn markdown_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH
}

#[derive(Default)]
struct Renderer {
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    bold: u32,
    italic: u32,
    heading: Option<HeadingLevel>,
    /// list markers stack: None = bullet, Some(n) = next ordered number
    lists: Vec<Option<u64>>,
    quote: u32,
    /// when Some, we're inside a fenced code block: (language tag, accumulated raw lines)
    code: Option<(String, Vec<String>)>,
    /// when Some, we're inside a table: cells are collected and laid out on `TagEnd::Table`.
    table: Option<TableBuild>,
    strike: u32,
    /// Cells a wrapped row of the current list item indents by (nested indent plus marker).
    item_hang: usize,
    /// True between an item's marker and its first text, so a loose item's paragraph start does
    /// not flush the bare marker as a row of its own.
    marker_pending: bool,
    /// The open link: its target and the text shown for it.
    link: Option<(String, String)>,
}

impl Renderer {
    fn style(&self) -> Style {
        let mut s = styles::fg();
        match self.heading {
            Some(HeadingLevel::H1 | HeadingLevel::H2 | HeadingLevel::H3) => {
                s = s.add_modifier(Modifier::BOLD)
            }
            Some(_) => s = styles::muted().add_modifier(Modifier::BOLD),
            None => {}
        }
        if self.quote > 0 {
            s = styles::muted().add_modifier(Modifier::ITALIC);
        }
        if self.bold > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link.is_some() {
            s = s.patch(styles::user()).add_modifier(Modifier::UNDERLINED);
        }
        s
    }

    /// The nested-list indent of a line.
    fn indent_prefix(&self) -> String {
        "  ".repeat(self.lists.len().saturating_sub(1))
    }

    /// Open a prose row: the (possibly empty) indent flags it as prose so it wraps at the
    /// measure, the item's hang keeps continuation rows under the text, and a quote bar repeats.
    fn open_row(&mut self) {
        let indent = self.indent_prefix();
        self.cur
            .push(Span::styled(indent, copy_mark::prose(Style::default())));
        if self.item_hang > 0 && !self.lists.is_empty() {
            self.cur.push(Span::styled(
                " ".repeat(self.item_hang.saturating_sub(self.cur[0].content.len())),
                copy_mark::hang(copy_mark::prose(Style::default())),
            ));
        }
        if self.quote > 0 {
            let bar = if g().unicode { "▏ " } else { "| " };
            self.cur
                .push(Span::styled(bar, copy_mark::repeat(styles::line())));
        }
    }

    fn push_text(&mut self, text: &str) {
        let style = self.style();
        if let Some(table) = self.table.as_mut() {
            table.cell.push(Span::styled(text.to_string(), style));
            return;
        }
        if self.cur.is_empty() {
            self.open_row();
        }
        self.marker_pending = false;
        let text = if self.heading == Some(HeadingLevel::H1) {
            text.to_uppercase()
        } else {
            text.to_string()
        };
        if let Some((_, shown)) = self.link.as_mut() {
            shown.push_str(&text);
        }
        self.cur.push(Span::styled(text, style));
    }

    fn flush_line(&mut self) {
        if !self.cur.is_empty() {
            self.lines.push(Line::from(std::mem::take(&mut self.cur)));
        }
    }

    fn blank(&mut self) {
        // No blank at the very start (the row under a turn label is text), and consecutive
        // blanks collapse.
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }

    /// Cells of the text on the open row, past the indent.
    fn cur_text_width(&self) -> usize {
        self.cur
            .iter()
            .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
            .sum()
    }

    fn inline_code(&self, c: &str) -> Span<'static> {
        // Where no background is painted the code needs its backticks to read as code.
        let text = if t().paints_backgrounds() {
            c.to_string()
        } else {
            format!("`{c}`")
        };
        Span::styled(text, styles::user().patch(styles::surface()))
    }

    fn event(&mut self, ev: Event<'_>) {
        // Inside a fenced code block, capture raw text verbatim until it closes.
        if let Some((_, buf)) = self.code.as_mut() {
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
                    let (lang, code) = self.code.take().unwrap_or_default();
                    self.render_code_block(&lang, &code);
                    return;
                }
                _ => return,
            }
        }

        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                self.flush_line();
                self.blank();
                self.heading = Some(level);
            }
            Event::End(TagEnd::Heading(level)) => {
                let width = self.cur_text_width();
                self.flush_line();
                let dash = |heavy: bool| match (g().unicode, heavy) {
                    (true, true) => "━",
                    (true, false) => "─",
                    (false, _) => "-",
                };
                match level {
                    HeadingLevel::H1 => {
                        self.lines.push(Line::from(vec![
                            Span::styled(String::new(), copy_mark::prose(Style::default())),
                            Span::styled(dash(true).repeat(width.clamp(1, 40)), styles::brand()),
                        ]));
                        self.blank();
                    }
                    HeadingLevel::H2 => {
                        self.lines.push(Line::from(vec![
                            Span::styled(String::new(), copy_mark::prose(Style::default())),
                            Span::styled(dash(false).repeat(width.max(1)), styles::line()),
                        ]));
                    }
                    _ => {}
                }
                self.heading = None;
            }
            Event::Start(Tag::Paragraph) => {
                if !self.marker_pending {
                    self.flush_line();
                }
            }
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
            Event::Start(Tag::List(first)) => {
                self.flush_line();
                self.lists.push(first);
            }
            Event::End(TagEnd::List(_)) => {
                self.lists.pop();
                self.item_hang = 0;
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            Event::Start(Tag::Item) => {
                self.flush_line();
                let indent = self.indent_prefix();
                let depth = self.lists.len();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => {
                        let bullet = match (g().unicode, depth) {
                            (true, 0 | 1) => "•",
                            (true, 2) => "◦",
                            (true, _) => "▪",
                            (false, 0 | 1) => "*",
                            (false, 2) => "-",
                            (false, _) => "+",
                        };
                        format!("{bullet} ")
                    }
                };
                self.item_hang =
                    indent.len() + unicode_width::UnicodeWidthStr::width(marker.as_str());
                self.cur
                    .push(Span::styled(indent, copy_mark::prose(Style::default())));
                self.cur.push(Span::styled(
                    marker,
                    copy_mark::hang(copy_mark::prose(styles::user())),
                ));
                self.marker_pending = true;
            }
            Event::End(TagEnd::Item) => {
                self.flush_line();
                self.marker_pending = false;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                self.flush_line();
                let lang = match kind {
                    CodeBlockKind::Fenced(s) => {
                        s.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, vec![String::new()]));
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.link = Some((dest_url.to_string(), String::new()));
            }
            Event::End(TagEnd::Link) => {
                if let Some((url, shown)) = self.link.take() {
                    let same = url.trim_end_matches('/') == shown.trim_end_matches('/')
                        || url.strip_prefix("mailto:") == Some(shown.as_str());
                    if !url.is_empty() && !same && self.table.is_none() {
                        self.cur
                            .push(Span::styled(format!(" ({url})"), styles::faint()));
                    }
                }
            }
            Event::Start(Tag::Table(_)) => {
                self.flush_line();
                self.blank();
                self.table = Some(TableBuild::default());
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let Some(t) = self.table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(t) = self.table.as_mut() {
                    t.header_rows = t.rows.len();
                }
            }
            Event::End(TagEnd::TableRow) => {}
            Event::Start(Tag::TableCell) => {
                if let Some(t) = self.table.as_mut() {
                    t.cell.clear();
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = self.table.as_mut() {
                    let cell = clip_spans(std::mem::take(&mut t.cell), TABLE_CELL_MAX);
                    if let Some(row) = t.rows.last_mut() {
                        row.push(cell);
                    }
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = self.table.take() {
                    self.render_table(t);
                }
                self.blank();
            }
            Event::Start(Tag::Strikethrough) => self.strike += 1,
            Event::End(TagEnd::Strikethrough) => self.strike = self.strike.saturating_sub(1),
            Event::Text(t) => self.push_text(&t),
            Event::Code(c) if self.table.is_some() => {
                let span = self.inline_code(&c);
                if let Some(table) = self.table.as_mut() {
                    table.cell.push(span);
                }
            }
            Event::Code(c) => {
                if self.cur.is_empty() {
                    self.open_row();
                }
                self.marker_pending = false;
                let span = self.inline_code(&c);
                self.cur.push(span);
            }
            Event::SoftBreak if self.table.is_some() => self.push_text(" "),
            Event::SoftBreak => self.flush_line(),
            Event::HardBreak if self.table.is_some() => self.push_text(" "),
            Event::HardBreak => self.flush_line(),
            Event::Rule => {
                self.flush_line();
                self.blank();
                let dash = if g().unicode { "─" } else { "-" };
                self.lines.push(Line::from(vec![
                    Span::styled(String::new(), copy_mark::prose(Style::default())),
                    Span::styled(dash.repeat(40), styles::line()),
                ]));
                self.blank();
            }
            _ => {}
        }
    }

    fn render_table(&mut self, table: TableBuild) {
        self.lines.extend(table::layout(table));
    }

    fn render_code_block(&mut self, lang: &str, code: &[String]) {
        self.blank();
        self.lines.extend(code::code_block(lang, code));
        self.blank();
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
    use unicode_width::UnicodeWidthStr;

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

    fn scoped_tier<R>(tier: crate::theme::ColorTier, f: impl FnOnce() -> R) -> R {
        crate::theme::scoped(
            crate::theme::Theme::new(
                tier,
                crate::theme::Mode::Dark,
                crate::theme::GlyphTier::Unicode,
                crate::theme::Motion::Full,
            ),
            f,
        )
    }

    #[test]
    fn soft_breaks_keep_their_line_breaks() {
        let out = markdown_to_lines("one\ntwo\nthree\n\n1\n2\n3\n");
        let t = text_of(&out);
        for l in ["one", "two", "three", "1", "2", "3"] {
            assert!(t.lines().any(|x| x.trim() == l), "{l} on its own line: {t}");
        }
    }

    #[test]
    fn gfm_table_has_a_bold_header_a_rule_and_no_grid() {
        let md = "Intro:\n\n| Field | App | Status |\n|---|---|---|\n| f1 wall-clock | `ctx` | ✓ |\n| f20 queue | 80 entries | **fixed** |\n\nAfter.\n";
        let out = markdown_to_lines(md);
        let t = text_of(&out);
        assert!(!t.contains('│') && !t.contains('┼'), "no grid: {t}");
        assert!(!t.contains("|---"), "separator row is not literal: {t}");
        let header = t.lines().find(|l| l.contains("Field")).unwrap();
        let rule = t.lines().find(|l| l.starts_with('─')).expect("header rule");
        assert!(rule.contains("─  ─"), "a rule per column: {rule:?}");
        assert!(has_modifier(&out, "Field", Modifier::BOLD), "bold header");
        // Columns line up: every row puts the second column at the same offset.
        let col = header.find("App").unwrap();
        for row in ["ctx", "80 entries"] {
            let r = t.lines().find(|l| l.contains(row)).unwrap();
            assert_eq!(r.find(row), Some(col), "aligned: {t}");
        }
        assert!(
            has_modifier(&out, "fixed", Modifier::BOLD),
            "styling survives inside cells"
        );
        assert!(
            t.contains("Intro:") && t.contains("After."),
            "surrounding text intact"
        );
    }

    #[test]
    fn numeric_columns_are_right_aligned() {
        let out = markdown_to_lines(
            "| stage | cost |\n|---|---|\n| parse | 86 us |\n| wrap | 1,258 us |\n",
        );
        let t = text_of(&out);
        let parse = t.lines().find(|l| l.contains("parse")).unwrap();
        let wrap = t.lines().find(|l| l.contains("wrap")).unwrap();
        assert_eq!(parse.width(), wrap.width(), "right edges meet: {t}");
        assert!(
            parse.ends_with("86 us") && wrap.ends_with("1,258 us"),
            "{t}"
        );
        assert!(parse.contains("   86 us"), "padded on the left: {parse:?}");
    }

    #[test]
    fn overlong_table_cell_is_clipped_with_an_ellipsis() {
        let long = "x".repeat(200);
        let md = format!("| a | b |\n|---|---|\n| {long} | y |\n");
        let out = markdown_to_lines(&md);
        let t = text_of(&out);
        let row = t.lines().find(|l| l.contains("xxx")).unwrap();
        assert!(row.contains('…'), "clipped: {row}");
        assert!(row.width() < 200, "row bounded: {}", row.width());
    }

    #[test]
    fn strikethrough_strips_tildes() {
        let out = markdown_to_lines("was ~~wrong~~ now right\n");
        let t = text_of(&out);
        assert!(!t.contains('~'), "{t}");
        assert!(has_modifier(&out, "wrong", Modifier::CROSSED_OUT));
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
    fn heading_levels_get_distinct_treatments() {
        let t = text_of(&markdown_to_lines(
            "# Big title\n\n## Mid title\n\n### Small\n\n#### Tiny\n",
        ));
        assert!(t.contains("BIG TITLE"), "H1 is uppercase: {t}");
        assert!(t.contains("━━"), "H1 has the heavy rule: {t}");
        let mid = t.lines().position(|l| l.contains("Mid title")).unwrap();
        assert_eq!(
            t.lines().nth(mid + 1).unwrap().trim(),
            "─".repeat("Mid title".len()),
            "H2 rule is as wide as the title: {t}"
        );
        let small = t.lines().position(|l| l.contains("Small")).unwrap();
        assert!(
            !t.lines().nth(small + 1).unwrap().contains('─'),
            "H3 has no rule"
        );
        let out = markdown_to_lines("#### Tiny\n");
        assert!(
            out[0]
                .spans
                .iter()
                .any(|s| s.content == "Tiny" && s.style.fg == styles::muted().fg),
            "H4 is muted: {out:?}"
        );
    }

    #[test]
    fn bold_and_inline_code_strip_markup_where_backgrounds_paint() {
        scoped_tier(crate::theme::ColorTier::TrueColor, || {
            let out = markdown_to_lines("step **one** and `two`\n");
            let t = text_of(&out);
            assert!(t.contains("one") && t.contains("two"));
            assert!(
                !t.contains("**") && !t.contains('`'),
                "markup stripped: {t:?}"
            );
            assert!(has_modifier(&out, "one", Modifier::BOLD), "bold applied");
        });
    }

    #[test]
    fn inline_code_keeps_its_backticks_without_backgrounds() {
        scoped_tier(crate::theme::ColorTier::Ansi16, || {
            let t = text_of(&markdown_to_lines("run `cargo test` now\n"));
            assert!(t.contains("`cargo test`"), "{t}");
        });
    }

    #[test]
    fn bullet_list_gets_markers_by_depth() {
        let out = markdown_to_lines("- alpha\n  - inner\n    - deep\n- beta\n");
        let t = text_of(&out);
        assert!(t.contains("• alpha"), "bullet marker: {t:?}");
        assert!(t.contains("◦ inner") && t.contains("▪ deep"), "{t:?}");
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
    fn a_wrapped_list_item_hangs_under_its_text() {
        let rows = crate::transcript::wrap_lines(
            &markdown_to_lines("- alpha beta gamma delta epsilon zeta eta theta\n"),
            24,
        );
        let t = text_of(&rows);
        let second = t.lines().nth(1).expect("wrapped");
        assert!(
            second.starts_with("  ") && !second.starts_with("   "),
            "hangs at the text: {t:?}"
        );
        assert!(second.trim_start().starts_with(|c: char| c.is_alphabetic()));
    }

    #[test]
    fn prose_wraps_at_the_reading_measure_not_the_terminal_width() {
        let words = "word ".repeat(60);
        let rows = crate::transcript::wrap_lines(&markdown_to_lines(&words), 200);
        let widest = rows
            .iter()
            .map(|r| r.spans.iter().map(|s| s.content.width()).sum::<usize>())
            .max()
            .unwrap();
        assert!(widest <= 100, "prose stays readable: {widest}");
        assert!(
            rows.len() >= 3,
            "wrapped well before 200 cells: {}",
            rows.len()
        );
    }

    #[test]
    fn fenced_code_block_is_a_band_with_a_header_and_verbatim_code() {
        scoped_tier(crate::theme::ColorTier::TrueColor, || {
            let out = markdown_to_lines("```rust\nlet x = 1;\n```\n");
            let t = text_of(&out);
            assert!(t.contains("let x = 1;"), "code preserved verbatim: {t:?}");
            assert!(
                t.contains("rust") && t.contains("/copy"),
                "header row: {t:?}"
            );
            assert!(!t.contains('┌') && !t.contains('└'), "no open frame");
            assert!(!t.contains("```"), "fence markers not shown");
            assert!(
                out.iter().any(|l| l.style.bg.is_some()),
                "rows carry the surface band"
            );
        });
    }

    #[test]
    fn code_blocks_get_line_numbers_only_past_five_rows() {
        scoped_tier(crate::theme::ColorTier::TrueColor, || {
            let short = text_of(&markdown_to_lines("```\na\nb\nc\n```\n"));
            assert!(
                !short.contains(" 1  a"),
                "no numbers on a short block: {short:?}"
            );
            let long = text_of(&markdown_to_lines("```\na\nb\nc\nd\ne\nf\ng\n```\n"));
            assert!(long.contains(" 1  a") && long.contains(" 7  g"), "{long:?}");
        });
    }

    #[test]
    fn code_blocks_use_a_rule_instead_of_a_band_on_sixteen_colours() {
        scoped_tier(crate::theme::ColorTier::Ansi16, || {
            let out = markdown_to_lines("```\nx\n```\n");
            assert!(out.iter().all(|l| l.style.bg.is_none()), "no background");
            assert!(text_of(&out).contains("│ x"), "rule beside the code");
        });
    }

    #[test]
    fn blockquote_is_italic_muted_behind_an_inner_rule() {
        let out = markdown_to_lines("> a quoted line\n");
        assert!(text_of(&out).contains("▏ a quoted line"));
        assert!(has_modifier(&out, "a quoted line", Modifier::ITALIC));
    }

    #[test]
    fn a_link_is_underlined_and_shows_its_target_after_when_different() {
        let out = markdown_to_lines("see [the docs](https://example.com/docs) now\n");
        assert!(has_modifier(&out, "the docs", Modifier::UNDERLINED));
        assert!(text_of(&out).contains("(https://example.com/docs)"));
        let bare = text_of(&markdown_to_lines("<https://example.com>\n"));
        assert_eq!(bare.matches("example.com").count(), 1, "{bare}");
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
        assert_eq!(text_of(&out), "the workspace looks healthy");
    }

    #[test]
    fn highlight_known_language_colors_tokens() {
        scoped_tier(crate::theme::ColorTier::TrueColor, || {
            let lines = vec!["fn main() { let x = 1; }".to_string()];
            let spans = highlight_code("rust", &lines);
            assert_eq!(spans.len(), 1);
            let joined: String = spans[0].iter().map(|s| s.content.as_ref()).collect();
            assert!(joined.contains("fn main()"), "source preserved: {joined:?}");
            // highlighting splits the line into more than one styled span.
            assert!(spans[0].len() > 1, "tokens are colored into multiple spans");
        });
    }

    #[test]
    fn highlight_unknown_language_falls_back_to_plain() {
        let lines = vec!["some arbitrary text".to_string()];
        let spans = highlight_code("nonsense-lang-xyz", &lines);
        let joined: String = spans[0].iter().map(|s| s.content.as_ref()).collect();
        assert!(
            joined.contains("some arbitrary text"),
            "no crash, text kept"
        );
    }

    #[test]
    fn fenced_block_shows_language_label() {
        let out = markdown_to_lines("```python\nprint('hi')\n```\n");
        let t = text_of(&out);
        assert!(t.contains("python"), "language label in the header: {t:?}");
        assert!(t.contains("print('hi')"), "code preserved");
    }
}
