//! Find in the full-screen transcript (Ctrl+F).
//!
//! The scrollback is the wrapped `main_log`, so the search runs over its wrapped rows exactly as
//! drawn: a hit is a row, the current hit scrolls into view, and every occurrence on a visible row
//! is painted. Smart case, like `less`: an upper-case letter in the query makes it case-sensitive.
//! While the bar is open it owns the keys that edit the query; scrolling keys pass through so the
//! transcript can still be paged. It is not available inline, where the scrollback belongs to the
//! terminal and its own search.

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

use super::*;
use crate::theme::styles;

/// The open Find bar: what was typed and which hit is current (an index into the hits).
#[derive(Debug, Clone, Default)]
pub struct TranscriptFind {
    query: String,
    current: usize,
}

/// One searchable cell of a wrapped row: its character and the columns it covers.
#[derive(Debug, Clone, Copy)]
struct Cell {
    ch: char,
    col: u16,
    width: u16,
}

/// The cells of `line` that are text, not decoration (rails, borders, gutters), with the column
/// each starts at. Decoration still advances the column.
fn text_cells(line: &TextLine<'_>) -> Vec<Cell> {
    let mut cells = Vec::new();
    let mut col = 0u16;
    for span in &line.spans {
        let decor = crate::copy_mark::is_decor(span.style.add_modifier);
        for ch in span.content.chars() {
            let width = ch.width().unwrap_or(0) as u16;
            if !decor {
                cells.push(Cell { ch, col, width });
            }
            col = col.saturating_add(width);
        }
    }
    cells
}

fn fold(c: char, insensitive: bool) -> char {
    if insensitive {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c
    }
}

/// Start indices into `cells` of every non-overlapping occurrence of `needle`.
fn occurrences(cells: &[Cell], needle: &[char], insensitive: bool) -> Vec<usize> {
    let mut found = Vec::new();
    if needle.is_empty() || cells.len() < needle.len() {
        return found;
    }
    let mut at = 0;
    while at + needle.len() <= cells.len() {
        let hit = needle
            .iter()
            .enumerate()
            .all(|(i, n)| fold(cells[at + i].ch, insensitive) == *n);
        if hit {
            found.push(at);
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

/// The query as folded characters, and whether the search ignores case.
fn needle(query: &str) -> (Vec<char>, bool) {
    let insensitive = !query.chars().any(char::is_uppercase);
    (
        query.chars().map(|c| fold(c, insensitive)).collect(),
        insensitive,
    )
}

impl App {
    /// Whether the Find bar is open.
    pub fn transcript_find_open(&self) -> bool {
        self.transcript_find.is_some()
    }

    /// Wrapped-row indices holding the query, ascending, at the last rendered width.
    fn find_hits(&self) -> Vec<usize> {
        let (Some(find), Some(geom)) = (&self.transcript_find, self.transcript_geom.get()) else {
            return Vec::new();
        };
        let (needle, insensitive) = needle(&find.query);
        if needle.is_empty() {
            return Vec::new();
        }
        self.ensure_wrapped_main(geom.width);
        let cache = self.wrap_cache.borrow();
        cache
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !occurrences(&text_cells(row), &needle, insensitive).is_empty())
            .map(|(i, _)| i)
            .collect()
    }

    /// Feed a key to Find: opens it on Ctrl+F, and edits/steps/closes it while open. True when the
    /// key was consumed.
    pub fn transcript_find_key(&mut self, key: KeyKind) -> bool {
        if self.transcript_find.is_none() {
            if key != KeyKind::FindTranscript {
                return false;
            }
            if !self.fullscreen {
                self.note("find works in full-screen mode; inline, use your terminal's own search");
            } else if !self.overlay_open() {
                self.transcript_find = Some(TranscriptFind::default());
            }
            return true;
        }
        match key {
            KeyKind::Esc | KeyKind::Interrupt | KeyKind::FindTranscript => {
                self.transcript_find = None
            }
            KeyKind::Char(c) => {
                self.find_edit(|query| query.push(c));
            }
            KeyKind::Backspace => {
                self.find_edit(|query| {
                    query.pop();
                });
            }
            KeyKind::Enter | KeyKind::Down => self.find_step(1),
            KeyKind::Up => self.find_step(-1),
            // The bar owns typing, not paging: scroll keys keep their usual meaning.
            KeyKind::PageUp | KeyKind::PageDown | KeyKind::JumpBottom | KeyKind::JumpPrevUser => {
                return false;
            }
            _ => {}
        }
        true
    }

    /// Change the query, then land on the first hit at or below the top of the view (else the last).
    fn find_edit(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(find) = self.transcript_find.as_mut() {
            edit(&mut find.query);
        }
        let hits = self.find_hits();
        let top = self.transcript_geom.get().map_or(0, |g| g.scroll);
        let current = hits
            .iter()
            .position(|&row| row >= top)
            .unwrap_or(hits.len().saturating_sub(1));
        if let Some(find) = self.transcript_find.as_mut() {
            find.current = current;
        }
        self.find_reveal(&hits, current);
    }

    fn find_step(&mut self, direction: isize) {
        let hits = self.find_hits();
        if hits.is_empty() {
            return;
        }
        let Some(find) = self.transcript_find.as_mut() else {
            return;
        };
        let n = hits.len() as isize;
        find.current = (find.current as isize + direction).rem_euclid(n) as usize;
        let current = find.current;
        self.find_reveal(&hits, current);
    }

    /// Scroll so the current hit sits a third of the way down the view.
    fn find_reveal(&mut self, hits: &[usize], current: usize) {
        let (Some(&row), Some(geom)) = (hits.get(current), self.transcript_geom.get()) else {
            return;
        };
        self.transcript_follow = false;
        self.transcript_scroll = row.saturating_sub(geom.height as usize / 3);
    }

    /// Paint every occurrence on the visible rows: the current hit's row filled, the rest bold and
    /// underlined. `scroll` is the row at the top of `area`.
    pub(crate) fn paint_find_matches(&self, frame: &mut Frame, area: Rect, scroll: usize) {
        let Some(find) = &self.transcript_find else {
            return;
        };
        let (needle, insensitive) = needle(&find.query);
        if needle.is_empty() {
            return;
        }
        let current_row = self.find_hits().get(find.current).copied();
        let cache = self.wrap_cache.borrow();
        let buf = frame.buffer_mut();
        for offset in 0..area.height as usize {
            let Some(row) = cache.rows.get(scroll + offset) else {
                break;
            };
            let cells = text_cells(row);
            let y = area.y + offset as u16;
            let on_current = current_row == Some(scroll + offset);
            for start in occurrences(&cells, &needle, insensitive) {
                for cell in &cells[start..start + needle.len()] {
                    for dx in 0..cell.width.max(1) {
                        let x = area.x + cell.col + dx;
                        if x >= area.x + area.width {
                            break;
                        }
                        if let Some(target) = buf.cell_mut(ratatui::layout::Position::new(x, y)) {
                            if on_current {
                                target.set_style(styles::selection().add_modifier(Modifier::BOLD));
                            } else {
                                target.set_style(
                                    target
                                        .style()
                                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// The bar on the last row of the transcript area.
    pub(crate) fn render_find_bar(&self, frame: &mut Frame, area: Rect) {
        let Some(find) = &self.transcript_find else {
            return;
        };
        if area.height == 0 {
            return;
        }
        let hits = self.find_hits().len();
        let counter = if find.query.is_empty() {
            String::new()
        } else if hits == 0 {
            "no matches".to_string()
        } else {
            format!("{}/{}", find.current.min(hits - 1) + 1, hits)
        };
        let glyphs = crate::theme::glyphs::g();
        let hint = format!(
            "{} {} Enter next {} Esc close",
            glyphs.up, "prev", glyphs.sep
        );
        let row = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        let line = TextLine::from(vec![
            Span::styled(" Find ", styles::pill(styles::brand())),
            Span::styled(format!(" {}", find.query), styles::fg()),
            Span::styled("▏", styles::brand()),
            Span::styled(format!("  {counter}"), styles::muted()),
            Span::styled(format!("   {hint}"), styles::faint()),
        ]);
        frame.render_widget(Clear, row);
        frame.render_widget(Paragraph::new(line), row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(text: &str) -> Vec<Cell> {
        text_cells(&TextLine::from(text.to_string()))
    }

    #[test]
    fn smart_case_ignores_case_until_the_query_has_a_capital() {
        let (lower, insensitive) = needle("parser");
        assert!(insensitive);
        assert_eq!(
            occurrences(&cells("The Parser parser"), &lower, insensitive),
            vec![4, 11]
        );
        let (capital, insensitive) = needle("Parser");
        assert!(!insensitive);
        assert_eq!(
            occurrences(&cells("The Parser parser"), &capital, insensitive),
            vec![4]
        );
    }

    #[test]
    fn columns_follow_wide_characters() {
        let row = cells("日本 go");
        let (n, i) = needle("go");
        let at = occurrences(&row, &n, i)[0];
        assert_eq!(row[at].col, 5);
    }

    #[test]
    fn decoration_advances_columns_but_is_not_searched() {
        let decor = crate::copy_mark::frame_row(ratatui::style::Style::default());
        let line = TextLine::from(vec![Span::styled("│ ", decor), Span::raw("needle")]);
        let row = text_cells(&line);
        assert_eq!(row.first().map(|c| c.col), Some(2));
        let (n, i) = needle("│");
        assert!(occurrences(&row, &n, i).is_empty());
    }

    fn app_with_log(lines: &[&str]) -> App {
        let mut app = App {
            fullscreen: true,
            ..App::default()
        };
        for line in lines {
            app.main_log.push(TextLine::from((*line).to_string()));
        }
        app.main_log_rev += 1;
        app.transcript_geom.set(Some(TranscriptGeom {
            col0: 0,
            row0: 0,
            width: 40,
            height: 6,
            scroll: 0,
        }));
        app
    }

    #[test]
    fn ctrl_f_opens_typing_finds_and_esc_closes() {
        let mut app = app_with_log(&["alpha", "beta needle", "gamma", "needle again"]);
        assert!(app.transcript_find_key(KeyKind::FindTranscript));
        assert!(app.transcript_find_open());
        for c in "needle".chars() {
            assert!(app.transcript_find_key(KeyKind::Char(c)));
        }
        assert_eq!(app.find_hits(), vec![1, 3]);
        assert!(!app.transcript_follow);
        app.transcript_find_key(KeyKind::Enter);
        assert_eq!(app.transcript_find.as_ref().map(|f| f.current), Some(1));
        app.transcript_find_key(KeyKind::Enter);
        assert_eq!(app.transcript_find.as_ref().map(|f| f.current), Some(0));
        app.transcript_find_key(KeyKind::Up);
        assert_eq!(app.transcript_find.as_ref().map(|f| f.current), Some(1));
        assert!(app.transcript_find_key(KeyKind::Esc));
        assert!(!app.transcript_find_open());
    }

    #[test]
    fn scroll_keys_pass_through_an_open_bar() {
        let mut app = app_with_log(&["x"]);
        app.transcript_find_key(KeyKind::FindTranscript);
        assert!(!app.transcript_find_key(KeyKind::PageUp));
        assert!(app.transcript_find_open());
    }

    #[test]
    fn other_keys_are_not_taken_while_closed() {
        let mut app = app_with_log(&["x"]);
        assert!(!app.transcript_find_key(KeyKind::Char('a')));
        assert!(!app.transcript_find_key(KeyKind::Esc));
    }

    #[test]
    fn the_bar_and_the_hits_are_drawn() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut app = app_with_log(&["alpha", "beta needle", "gamma"]);
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let draw = |terminal: &mut Terminal<TestBackend>, app: &App| {
            terminal
                .draw(|f| crate::app::render::render_transcript_area(f, f.area(), app))
                .unwrap();
        };
        draw(&mut terminal, &app);
        app.transcript_find_key(KeyKind::FindTranscript);
        for c in "needle".chars() {
            app.transcript_find_key(KeyKind::Char(c));
        }
        draw(&mut terminal, &app);
        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..40).map(|x| buffer[(x, y)].symbol()).collect() };
        let bar = row(5);
        assert!(
            bar.contains("Find") && bar.contains("needle") && bar.contains("1/1"),
            "{bar:?}"
        );
        let y = (0..5)
            .find(|&y| row(y).contains("needle"))
            .expect("the hit row is on screen");
        let x = row(y).find("needle").unwrap() as u16;
        let hit = buffer[(x, y)].style();
        let plain = buffer[(0, y)].style();
        assert_ne!(hit, plain, "the occurrence is painted");
    }

    #[test]
    fn inline_mode_explains_instead_of_opening() {
        let mut app = App::default();
        assert!(app.transcript_find_key(KeyKind::FindTranscript));
        assert!(!app.transcript_find_open());
    }
}
