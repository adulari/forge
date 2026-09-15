//! The full-output viewer: one tool call's complete output, on the whole screen.
//!
//! A tool card's inline preview is bounded on purpose. It lives in the scrollback, and a megabyte
//! of build log spliced into the transcript would bury the conversation around it. This view is
//! where the rest is read. It opens from a card's "⤢ view full output" row or from `/output`,
//! reads the output the core kept on disk (`forge_core::tool_output`), and pages it the way `less`
//! does: line numbers, search, a wrap toggle, copy-all, and a hand-off to `$PAGER`. It takes over
//! the chat's own terminal like the activity viewer, so a running turn keeps streaming underneath.

use std::cell::{Cell, RefCell};

use forge_types::ToolOutputRef;
use ratatui::style::Modifier;
use ratatui::widgets::Clear;
use ratatui::Frame;

use super::tool_cards::{self, CardStatus, ToolCard};
use super::*;
use crate::surface::VERY_DIM;

/// What a key in the viewer asks of the shell beyond a redraw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputAction {
    Redraw,
    Close,
    /// Put this text on the clipboard.
    Copy(String),
    /// Hand this file to `$PAGER`.
    OpenPager(String),
}

#[derive(Debug, Clone)]
struct Row {
    line: usize,
    first: bool,
    text: String,
}

#[derive(Debug, Clone)]
struct RowCache {
    key: (usize, bool, usize),
    rows: Vec<Row>,
}

#[derive(Debug, Clone)]
pub struct OutputView {
    title: String,
    outcome: String,
    ok: bool,
    lines: Vec<String>,
    bytes: usize,
    path: Option<String>,
    /// False when only a card's bounded preview could be shown.
    complete: bool,
    wrap: bool,
    hscroll: usize,
    /// The search being typed after `/`, until Enter or Esc.
    typing: Option<String>,
    query: Option<String>,
    matches: Vec<usize>,
    current: usize,
    notice: Option<String>,
    /// First visible row. Render owns the clamp, so these are cells like the activity viewer's.
    top: Cell<usize>,
    /// A source line to bring into view on the next render, and how many rows to leave above it:
    /// a search hit, or the line that was on top before a wrap toggle moved every row.
    anchor: Cell<Option<(usize, usize)>>,
    body_h: Cell<usize>,
    rows: RefCell<Option<RowCache>>,
}

impl OutputView {
    fn new(
        title: String,
        outcome: String,
        ok: bool,
        text: &str,
        path: Option<String>,
        complete: bool,
    ) -> Self {
        let mut lines: Vec<String> = text.lines().map(|l| l.replace('\t', "    ")).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            title,
            outcome,
            ok,
            bytes: text.len(),
            lines,
            path,
            complete,
            wrap: true,
            hscroll: 0,
            typing: None,
            query: None,
            matches: Vec::new(),
            current: 0,
            notice: None,
            // A failed call's cause is printed last; a successful one reads from the top.
            top: Cell::new(if ok { 0 } else { usize::MAX }),
            anchor: Cell::new(None),
            body_h: Cell::new(1),
            rows: RefCell::new(None),
        }
    }

    /// A card's output: the kept file when there is one, else the preview the card already holds.
    pub(crate) fn for_card(card: &ToolCard) -> Self {
        let title = format!(
            "{}  {}",
            card.name,
            tool_cards::headline(&card.name, &card.args)
        );
        let outcome = tool_cards::outcome_text(card);
        let ok = card.status != CardStatus::Failed;
        let preview = card.detail.as_deref().unwrap_or("");
        if let Some(full) = &card.full {
            return match std::fs::read(&full.path) {
                Ok(bytes) => Self::new(
                    title,
                    outcome,
                    ok,
                    &String::from_utf8_lossy(&bytes),
                    Some(full.path.clone()),
                    true,
                ),
                Err(error) => {
                    let mut view = Self::new(title, outcome, ok, preview, None, false);
                    view.notice = Some(format!(
                        "the kept output could not be read ({error}); showing the preview"
                    ));
                    view
                }
            };
        }
        let complete = !preview.contains(forge_types::FULL_OUTPUT_HINT);
        let mut view = Self::new(title, outcome, ok, preview, None, complete);
        if !complete {
            view.notice =
                Some("only the preview exists: this call's full output was not kept".to_string());
        }
        view
    }

    pub fn key(&mut self, key: KeyKind) -> OutputAction {
        if let Some(typed) = self.typing.as_mut() {
            match key {
                KeyKind::Char(c) => typed.push(c),
                KeyKind::Backspace => {
                    typed.pop();
                }
                KeyKind::Enter => {
                    let query = self.typing.take().unwrap_or_default();
                    self.search(query);
                }
                KeyKind::Esc | KeyKind::Interrupt => self.typing = None,
                _ => {}
            }
            return OutputAction::Redraw;
        }
        self.notice = None;
        let page = self.body_h.get().max(1) as isize;
        match key {
            KeyKind::Esc | KeyKind::Interrupt | KeyKind::Char('q') => return OutputAction::Close,
            KeyKind::Up | KeyKind::Char('k') => self.scroll_by(-1),
            KeyKind::Down | KeyKind::Char('j') => self.scroll_by(1),
            KeyKind::PageUp | KeyKind::Char('u') | KeyKind::Char('b') => self.scroll_by(-page),
            KeyKind::PageDown | KeyKind::Char(' ') | KeyKind::Char('d') | KeyKind::Char('f') => {
                self.scroll_by(page)
            }
            KeyKind::Home | KeyKind::Char('g') => self.top.set(0),
            KeyKind::End | KeyKind::Char('G') => self.top.set(usize::MAX),
            KeyKind::Char('/') => self.typing = Some(String::new()),
            KeyKind::Char('n') => self.step_match(true),
            KeyKind::Char('N') => self.step_match(false),
            KeyKind::Char('w') => {
                self.anchor.set(Some((self.top_line(), 0)));
                self.wrap = !self.wrap;
                self.hscroll = 0;
                self.notice = Some(if self.wrap {
                    "wrap on".to_string()
                } else {
                    "wrap off: ←/→ scroll sideways".to_string()
                });
            }
            KeyKind::Left if !self.wrap => self.hscroll = self.hscroll.saturating_sub(8),
            KeyKind::Right if !self.wrap => self.hscroll += 8,
            KeyKind::Char('y') => {
                self.notice = Some(format!(
                    "copied {} to the clipboard",
                    tool_cards::size_label(self.lines.len(), self.bytes)
                ));
                return OutputAction::Copy(self.lines.join("\n"));
            }
            KeyKind::Char('o') => match &self.path {
                Some(path) => return OutputAction::OpenPager(path.clone()),
                None => {
                    self.notice =
                        Some("nothing on disk to open: only the preview was kept".to_string())
                }
            },
            _ => {}
        }
        OutputAction::Redraw
    }

    pub(crate) fn scroll_by(&self, delta: isize) {
        let top = self.top.get().min(self.max_top());
        self.top.set(top.saturating_add_signed(delta));
    }

    fn max_top(&self) -> usize {
        let total = self
            .rows
            .borrow()
            .as_ref()
            .map_or(self.lines.len(), |c| c.rows.len());
        total.saturating_sub(self.body_h.get())
    }

    fn top_line(&self) -> usize {
        let top = self.top.get();
        self.rows
            .borrow()
            .as_ref()
            .and_then(|c| c.rows.get(top.min(c.rows.len().saturating_sub(1))))
            .map_or(0, |row| row.line)
    }

    fn search(&mut self, query: String) {
        if query.is_empty() {
            self.query = None;
            self.matches.clear();
            return;
        }
        self.matches = find_matches(&self.lines, &query);
        self.current = 0;
        if self.matches.is_empty() {
            self.notice = Some(format!("no match for “{query}”"));
        } else {
            let from = self.top_line();
            self.current = self.matches.iter().position(|&l| l >= from).unwrap_or(0);
            self.bring_current_into_view();
        }
        self.query = Some(query);
    }

    fn step_match(&mut self, forward: bool) {
        if self.matches.is_empty() {
            self.notice = Some(match &self.query {
                Some(query) => format!("no match for “{query}”"),
                None => "press / to search".to_string(),
            });
            return;
        }
        let n = self.matches.len();
        self.current = if forward {
            (self.current + 1) % n
        } else {
            (self.current + n - 1) % n
        };
        self.bring_current_into_view();
    }

    fn bring_current_into_view(&self) {
        if let Some(&line) = self.matches.get(self.current) {
            self.anchor.set(Some((line, self.body_h.get() / 3)));
        }
    }
}

impl App {
    /// A click on a card's "view full output" row. True when it opened the viewer, so the caller
    /// does not also treat the click as a card toggle.
    pub fn open_tool_output_at(&mut self, col: u16, row: u16) -> bool {
        let Some((wrapped_row, _)) = self.pointer_to_text(col, row) else {
            return false;
        };
        let width = self.transcript_geom.get().map(|g| g.width).unwrap_or(0);
        self.ensure_wrapped_main(width);
        let source = {
            let cache = self.wrap_cache.borrow();
            match cache.origins.get(wrapped_row) {
                Some(line) => *line,
                None => return false,
            }
        };
        let Some(card) = self.tool_cards.iter().find(|c| {
            tool_cards::full_output_row(c).is_some_and(|offset| source == c.start + offset)
        }) else {
            return false;
        };
        self.output_view = Some(OutputView::for_card(card));
        true
    }

    /// `/output`: the most recent call that has anything to show.
    pub fn open_latest_tool_output(&mut self) -> bool {
        let Some(card) = self
            .tool_cards
            .iter()
            .rev()
            .find(|c| c.full.is_some() || c.detail.is_some())
        else {
            return false;
        };
        self.output_view = Some(OutputView::for_card(card));
        true
    }

    /// The newest kept output file. Inline mode has no cards and no room for the viewer, so its
    /// `/output` points here instead.
    pub fn latest_kept_output(&self) -> Option<&str> {
        self.last_tool_output
            .as_ref()
            .map(|(_, output)| output.path.as_str())
    }

    /// Route a key to the viewer. `None` when it is not open, so the key belongs to someone else.
    pub fn output_view_key(&mut self, key: KeyKind) -> Option<OutputAction> {
        let action = self.output_view.as_mut()?.key(key);
        if action == OutputAction::Close {
            self.output_view = None;
        }
        Some(action)
    }

    /// Mouse wheel over the viewer. True when it was open and took the scroll.
    pub fn output_view_scroll(&self, up: bool, step: usize) -> bool {
        let Some(view) = &self.output_view else {
            return false;
        };
        let step = step as isize;
        view.scroll_by(if up { -step } else { step });
        true
    }

    pub(crate) fn attach_tool_output(&mut self, name: &str, output: ToolOutputRef) {
        self.last_tool_output = Some((name.to_string(), output.clone()));
        let card = self.last_closed_card.filter(|id| {
            self.tool_cards
                .iter()
                .any(|c| c.id == *id && c.name == name)
        });
        match card {
            Some(id) => {
                if let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == id) {
                    card.full = Some(output);
                }
                self.redraw_tool_card(id);
            }
            None if !self.fullscreen => {
                let hint = format!(
                    "    ⤢ full output kept ({}) · /output",
                    tool_cards::size_label(output.lines, output.bytes)
                );
                self.tag_flush(LineOrigin::tool_result(name, true), |s| {
                    s.flush.push(TextLine::from(Span::styled(
                        hint,
                        Style::default().fg(VERY_DIM),
                    )))
                });
            }
            None => {}
        }
    }
}

/// Hand `path` to `$PAGER` (default `less -R`). Call it inside `Tui::run_fullscreen`, which puts
/// the chat's terminal back when the pager exits; this only hands the terminal over.
pub fn page_file(path: &str) -> std::io::Result<()> {
    use crossterm::event::DisableMouseCapture;
    use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
    disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    let pager = std::env::var("PAGER")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "less -R".to_string());
    let mut words = pager.split_whitespace();
    let program = words.next().unwrap_or("less");
    std::process::Command::new(program)
        .args(words)
        .arg(path)
        .status()
        .map(|_| ())
}

pub(crate) fn render_output_view(frame: &mut Frame, view: &OutputView) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let width = area.width as usize;
    let height = area.height as usize;
    let body_h = height.saturating_sub(3).max(1);
    let gutter = view.lines.len().to_string().len();
    let text_w = width.saturating_sub(gutter + 4).max(8);
    let key = (text_w, view.wrap, view.hscroll);
    {
        let mut cache = view.rows.borrow_mut();
        if cache.as_ref().is_none_or(|c| c.key != key) {
            *cache = Some(RowCache {
                key,
                rows: build_rows(&view.lines, text_w, view.wrap, view.hscroll),
            });
        }
    }
    let cache = view.rows.borrow();
    let rows = &cache.as_ref().expect("rows built above").rows;
    view.body_h.set(body_h);
    if let Some((line, above)) = view.anchor.take() {
        let at = rows.iter().position(|r| r.line == line).unwrap_or(0);
        view.top.set(at.saturating_sub(above));
    }
    let top = view.top.get().min(rows.len().saturating_sub(body_h));
    view.top.set(top);

    let current = view.matches.get(view.current).copied();
    let mut out = Vec::with_capacity(height);
    out.push(header_line(view, width));
    out.push(meta_line(view, width));
    for row in rows.iter().skip(top).take(body_h) {
        out.push(body_line(view, row, gutter, current));
    }
    while out.len() + 1 < height {
        out.push(TextLine::default());
    }
    out.push(footer_line(view, rows, top, body_h, width));
    frame.render_widget(Paragraph::new(out), area);
}

fn header_line(view: &OutputView, width: usize) -> TextLine<'static> {
    let (mark, color) = if view.ok {
        ("✓", OKGREEN)
    } else {
        ("✖", ERRRED)
    };
    let right = if view.outcome.is_empty() {
        String::new()
    } else {
        format!("{mark} {}", view.outcome)
    };
    let title = tool_cards::truncate_cells(
        &view.title,
        width.saturating_sub(tool_cards::cells(&right) + 7),
    );
    let pad = width
        .saturating_sub(4 + tool_cards::cells(&title) + tool_cards::cells(&right) + 1)
        .max(1);
    TextLine::from(vec![
        Span::styled("  ⤢ ", Style::default().fg(TOOLCYAN)),
        Span::styled(
            title,
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, Style::default().fg(color)),
    ])
}

fn meta_line(view: &OutputView, width: usize) -> TextLine<'static> {
    let size = tool_cards::size_label(view.lines.len(), view.bytes);
    let mut spans = vec![Span::styled(format!("  {size}"), Style::default().fg(DIM))];
    if !view.complete {
        spans.push(Span::styled(
            "  · preview only",
            Style::default().fg(WARNYEL),
        ));
    } else if let Some(path) = &view.path {
        let shown = tool_cards::truncate_cells(
            &tool_cards::shorten(path),
            width.saturating_sub(tool_cards::cells(&size) + 8),
        );
        spans.push(Span::styled(
            format!("  · {shown}"),
            Style::default().fg(VERY_DIM),
        ));
    }
    TextLine::from(spans)
}

fn body_line(
    view: &OutputView,
    row: &Row,
    gutter: usize,
    current: Option<usize>,
) -> TextLine<'static> {
    let number = if row.first {
        format!("{:>gutter$}", row.line + 1)
    } else {
        " ".repeat(gutter)
    };
    let number_style = if row.first && current == Some(row.line) {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(VERY_DIM)
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(number, number_style),
        Span::styled(" │ ", Style::default().fg(VERY_DIM)),
    ];
    let base = Style::default().fg(TEXT);
    match view
        .query
        .as_deref()
        .filter(|_| view.matches.binary_search(&row.line).is_ok())
    {
        Some(query) => spans.extend(highlight(&row.text, query, base)),
        None => spans.push(Span::styled(row.text.clone(), base)),
    }
    TextLine::from(spans)
}

fn footer_line(
    view: &OutputView,
    rows: &[Row],
    top: usize,
    body_h: usize,
    width: usize,
) -> TextLine<'static> {
    if let Some(typed) = &view.typing {
        return TextLine::from(vec![
            Span::styled("  /", Style::default().fg(ACCENT)),
            Span::styled(typed.clone(), Style::default().fg(TEXT)),
            Span::styled("▏", Style::default().fg(ACCENT)),
            Span::styled(
                "   Enter search · Esc cancel",
                Style::default().fg(VERY_DIM),
            ),
        ]);
    }
    let first = rows.get(top).map_or(0, |r| r.line + 1);
    let last = rows
        .get((top + body_h).min(rows.len()).saturating_sub(1))
        .map_or(0, |r| r.line + 1);
    let mut position = format!("{first}–{last} of {}", tool_cards::group(view.lines.len()));
    if let Some(query) = &view.query {
        position = if view.matches.is_empty() {
            format!("{position} · no “{query}”")
        } else {
            format!(
                "{position} · match {}/{}",
                view.current + 1,
                view.matches.len()
            )
        };
    }
    let (left, left_style) = match &view.notice {
        Some(notice) => (notice.clone(), Style::default().fg(WARNYEL)),
        None => (hints(view).to_string(), Style::default().fg(VERY_DIM)),
    };
    let left = tool_cards::truncate_cells(
        &left,
        width.saturating_sub(tool_cards::cells(&position) + 6),
    );
    let pad = width
        .saturating_sub(4 + tool_cards::cells(&left) + tool_cards::cells(&position))
        .max(1);
    TextLine::from(vec![
        Span::raw("  "),
        Span::styled(left, left_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(position, Style::default().fg(DIM)),
    ])
}

fn hints(view: &OutputView) -> &'static str {
    match (view.path.is_some(), view.wrap) {
        (true, true) => "↑↓ PgUp PgDn · g/G top/end · / search · n/N next · w wrap · y copy all · o $PAGER · Esc close",
        (true, false) => "↑↓ ←→ · g/G top/end · / search · n/N next · w wrap · y copy all · o $PAGER · Esc close",
        (false, true) => "↑↓ PgUp PgDn · g/G top/end · / search · n/N next · w wrap · y copy all · Esc close",
        (false, false) => "↑↓ ←→ · g/G top/end · / search · n/N next · w wrap · y copy all · Esc close",
    }
}

fn build_rows(lines: &[String], width: usize, wrap: bool, hscroll: usize) -> Vec<Row> {
    let mut rows = Vec::with_capacity(lines.len());
    for (line, text) in lines.iter().enumerate() {
        if wrap {
            for (k, chunk) in tool_cards::wrap_plain(text, width).into_iter().enumerate() {
                rows.push(Row {
                    line,
                    first: k == 0,
                    text: chunk,
                });
            }
        } else {
            rows.push(Row {
                line,
                first: true,
                text: slice_cells(text, hscroll, width),
            });
        }
    }
    rows
}

/// The `width` cells of `text` that start `skip` cells in: a line seen through a sideways scroll.
fn slice_cells(text: &str, skip: usize, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut out = String::new();
    let (mut at, mut used) = (0usize, 0usize);
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);
        if at < skip {
            at += w;
            continue;
        }
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// Smartcase, as in `less -i` and most editors: a query with a capital letter matches case exactly.
fn is_case_sensitive(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

fn fold(c: char, sensitive: bool) -> char {
    if sensitive {
        c
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Char offsets where `needle` starts in `text`, non-overlapping. Compared char by char rather
/// than on lowercased strings, whose byte lengths can differ from the original's.
fn occurrences(text: &[char], needle: &[char], sensitive: bool) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > text.len() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= text.len() {
        let hit = text[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| fold(*a, sensitive) == fold(*b, sensitive));
        if hit {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

fn find_matches(lines: &[String], query: &str) -> Vec<usize> {
    let sensitive = is_case_sensitive(query);
    let needle: Vec<char> = query.chars().collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let chars: Vec<char> = line.chars().collect();
            !occurrences(&chars, &needle, sensitive).is_empty()
        })
        .map(|(i, _)| i)
        .collect()
}

fn highlight(text: &str, query: &str, base: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = query.chars().collect();
    let mut spans = Vec::new();
    let mut at = 0;
    for start in occurrences(&chars, &needle, is_case_sensitive(query)) {
        if start > at {
            spans.push(Span::styled(
                chars[at..start].iter().collect::<String>(),
                base,
            ));
        }
        let end = start + needle.len();
        spans.push(Span::styled(
            chars[start..end].iter().collect::<String>(),
            base.add_modifier(Modifier::REVERSED),
        ));
        at = end;
    }
    if at < chars.len() {
        spans.push(Span::styled(chars[at..].iter().collect::<String>(), base));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn numbered(n: usize) -> String {
        (1..=n)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn view(text: &str, ok: bool) -> OutputView {
        OutputView::new(
            "shell  cargo test".into(),
            "exit 0 in 1s".into(),
            ok,
            text,
            Some("/tmp/out.log".into()),
            true,
        )
    }

    fn draw(view: &OutputView) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| render_output_view(f, view)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_successful_call_opens_at_the_top_and_a_failed_one_at_the_end() {
        let ok = view(&numbered(500), true);
        let screen = draw(&ok);
        assert!(screen[2].contains("line 1"), "{screen:?}");

        let failed = view(&numbered(500), false);
        let screen = draw(&failed);
        assert!(
            screen[screen.len() - 2].contains("line 500"),
            "a failure's cause is printed last, so that is where the view opens: {screen:?}"
        );
    }

    #[test]
    fn the_whole_output_is_reachable_not_just_a_preview() {
        let mut v = view(&numbered(5_000), true);
        draw(&v);
        v.key(KeyKind::Char('G'));
        let screen = draw(&v);
        assert!(
            screen.iter().any(|row| row.contains("line 5000")),
            "{screen:?}"
        );
        assert!(screen[1].contains("5,000 lines"), "{screen:?}");
    }

    #[test]
    fn search_jumps_between_matches_and_says_where_it_is() {
        let text = format!(
            "{}\nerror: first\n{}\nerror: second",
            numbered(100),
            numbered(100)
        );
        let mut v = view(&text, true);
        draw(&v);
        for key in "/ERROR".chars() {
            v.key(KeyKind::Char(key));
        }
        v.key(KeyKind::Enter);
        assert!(
            v.matches.is_empty(),
            "a capital letter makes the search case-sensitive"
        );
        for key in "/error".chars() {
            v.key(KeyKind::Char(key));
        }
        v.key(KeyKind::Enter);
        assert_eq!(v.matches, vec![100, 201]);
        let screen = draw(&v);
        assert!(
            screen.iter().any(|row| row.contains("error: first")),
            "{screen:?}"
        );
        assert!(screen.last().unwrap().contains("match 1/2"), "{screen:?}");
        v.key(KeyKind::Char('n'));
        let screen = draw(&v);
        assert!(
            screen.iter().any(|row| row.contains("error: second")),
            "{screen:?}"
        );
        v.key(KeyKind::Char('n'));
        assert_eq!(v.current, 0, "n wraps back to the first match");
    }

    #[test]
    fn esc_while_typing_a_search_cancels_the_search_not_the_viewer() {
        let mut v = view("a\nb", true);
        v.key(KeyKind::Char('/'));
        assert_eq!(v.key(KeyKind::Esc), OutputAction::Redraw);
        assert!(v.typing.is_none());
        assert_eq!(v.key(KeyKind::Esc), OutputAction::Close);
    }

    #[test]
    fn copy_hands_over_everything_and_the_pager_gets_the_kept_file() {
        let mut v = view("one\ntwo\nthree", true);
        assert_eq!(
            v.key(KeyKind::Char('y')),
            OutputAction::Copy("one\ntwo\nthree".into())
        );
        assert_eq!(
            v.key(KeyKind::Char('o')),
            OutputAction::OpenPager("/tmp/out.log".into())
        );
    }

    #[test]
    fn without_wrap_long_lines_scroll_sideways() {
        let mut v = view(&format!("{}END", "x".repeat(200)), true);
        v.key(KeyKind::Char('w'));
        for _ in 0..25 {
            v.key(KeyKind::Right);
        }
        assert!(draw(&v).iter().any(|row| row.contains("END")));
    }

    #[test]
    fn a_preview_that_stands_in_for_more_output_says_so() {
        let mut card = ToolCard::new(1, "shell".into(), r#"{"command":"make"}"#.into());
        card.status = CardStatus::Ok;
        card.detail = Some(format!(
            "start\n… 900 lines hidden · {}\nend",
            forge_types::FULL_OUTPUT_HINT
        ));
        let v = OutputView::for_card(&card);
        assert!(!v.complete);
        assert!(draw(&v)[1].contains("preview only"));
    }

    #[test]
    fn highlighting_survives_characters_whose_lowercase_is_longer() {
        let spans = highlight("İstanbul error", "error", Style::default());
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "İstanbul error");
        assert_eq!(spans.last().unwrap().content, "error");
    }
}
