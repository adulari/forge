//! The history search (`/history <words>`): ranked word search over this session's whole history,
//! the messages a compaction folded out of the model's context included. Enter on a hit reads the
//! messages around it, so a person can see what the model can still `recall` but no longer holds.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::theme::glyphs::g;
use crate::theme::styles;
use crate::KeyKind;

/// One search hit, as the host found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryHit {
    pub seq: i64,
    pub role: String,
    /// The match in context; matched words are wrapped in `[` `]`.
    pub snippet: String,
    pub folded: bool,
}

/// What the host does after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAction {
    None,
    Search(String),
    /// Read the messages around this seq.
    View(i64),
}

#[derive(Debug, Clone, Default)]
pub struct HistoryOverlay {
    pub open: bool,
    query: String,
    /// The query text is being typed (otherwise the hit list takes the navigation keys).
    editing: bool,
    searched: bool,
    hits: Vec<HistoryHit>,
    selected: usize,
    /// The messages around a hit: `(seq, lines)`.
    viewing: Option<(i64, Vec<String>)>,
    scroll: usize,
    pub notice: Option<String>,
    /// A search `/history <words>` asked for before the host first looked.
    start: Option<String>,
}

impl HistoryOverlay {
    pub fn open(&mut self, query: &str) {
        let query = query.trim().to_string();
        *self = Self {
            open: true,
            editing: query.is_empty(),
            start: (!query.is_empty()).then(|| query.clone()),
            query,
            ..Self::default()
        };
    }

    /// The search `/history <words>` opened with, once.
    pub fn take_start(&mut self) -> Option<String> {
        self.start.take()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn hits(&self) -> &[HistoryHit] {
        &self.hits
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn viewing(&self) -> Option<i64> {
        self.viewing.as_ref().map(|(seq, _)| *seq)
    }

    pub fn set_hits(&mut self, hits: Vec<HistoryHit>) {
        self.searched = true;
        self.selected = 0;
        self.editing = hits.is_empty();
        self.hits = hits;
    }

    pub fn set_view(&mut self, seq: i64, text: &str) {
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        self.scroll = 0;
        self.viewing = Some((seq, lines));
    }

    pub fn key(&mut self, key: KeyKind) -> HistoryAction {
        if let Some((_, lines)) = &self.viewing {
            let last = lines.len().saturating_sub(1);
            match key {
                KeyKind::Esc | KeyKind::Char('q') | KeyKind::Left => self.viewing = None,
                KeyKind::Down | KeyKind::Char('j') => self.scroll = (self.scroll + 1).min(last),
                KeyKind::Up | KeyKind::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                KeyKind::PageDown => self.scroll = (self.scroll + 10).min(last),
                KeyKind::PageUp => self.scroll = self.scroll.saturating_sub(10),
                KeyKind::Home | KeyKind::Char('g') => self.scroll = 0,
                KeyKind::End | KeyKind::Char('G') => self.scroll = last,
                _ => {}
            }
            return HistoryAction::None;
        }
        if self.editing {
            match key {
                KeyKind::Esc => {
                    if self.hits.is_empty() {
                        self.open = false;
                    } else {
                        self.editing = false;
                    }
                }
                KeyKind::Enter => {
                    let query = self.query.trim().to_string();
                    if query.chars().count() >= 2 {
                        self.notice = None;
                        return HistoryAction::Search(query);
                    }
                    self.notice = Some("type at least two letters".to_string());
                }
                KeyKind::Backspace => {
                    self.query.pop();
                }
                KeyKind::KillLineBack | KeyKind::DeleteWordBack => self.query.clear(),
                KeyKind::Char(c) => self.query.push(c),
                _ => {}
            }
            return HistoryAction::None;
        }
        match key {
            KeyKind::Esc | KeyKind::Char('q') => self.open = false,
            KeyKind::Down | KeyKind::Char('j') => {
                self.selected = (self.selected + 1).min(self.hits.len().saturating_sub(1));
            }
            KeyKind::Up | KeyKind::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyKind::Char('/') => self.editing = true,
            KeyKind::Enter | KeyKind::Right => {
                if let Some(hit) = self.hits.get(self.selected) {
                    return HistoryAction::View(hit.seq);
                }
            }
            _ => {}
        }
        HistoryAction::None
    }
}

fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push_str(g().ellipsis);
    out
}

/// A message line cut into pieces no wider than `width` characters.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// A snippet as spans, with the host's `[` `]` marks turned into emphasis.
fn snippet_spans(snippet: &str, room: usize) -> Vec<Span<'static>> {
    let flat = fit(&snippet.replace('\n', " "), room + 2);
    let mut spans = Vec::new();
    let mut rest = flat.as_str();
    while !rest.is_empty() {
        let open = rest.find('[');
        let close = open.and_then(|o| rest[o..].find(']').map(|c| o + c));
        match (open, close) {
            (Some(o), Some(c)) => {
                if o > 0 {
                    spans.push(Span::styled(rest[..o].to_string(), styles::fg()));
                }
                spans.push(Span::styled(
                    rest[o + 1..c].to_string(),
                    styles::brand().bold(),
                ));
                rest = &rest[c + 1..];
            }
            _ => {
                spans.push(Span::styled(rest.to_string(), styles::fg()));
                break;
            }
        }
    }
    spans
}

pub fn render_history_overlay(frame: &mut Frame, overlay: &HistoryOverlay) {
    if !overlay.open {
        return;
    }
    let full = frame.area();
    frame.render_widget(Clear, full);
    if full.height < 4 || full.width < 20 {
        return;
    }
    let width = usize::from(full.width);
    let folded = overlay.hits.iter().filter(|h| h.folded).count();
    let title = match overlay.viewing() {
        Some(seq) => format!(" History  around message #{seq}"),
        None if overlay.searched => format!(
            " History  {} hit(s), {folded} folded out of the model's context",
            overlay.hits.len()
        ),
        None => " History  search this session, folded messages included".to_string(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(title, styles::brand().bold()))),
        Rect { height: 1, ..full },
    );
    let body = Rect {
        y: full.y + 1,
        height: full.height - 2,
        ..full
    };
    let h = usize::from(body.height);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some((_, text)) = &overlay.viewing {
        for raw in text.iter().skip(overlay.scroll) {
            for piece in wrap(raw, width.saturating_sub(2)) {
                lines.push(Line::from(Span::styled(format!(" {piece}"), styles::fg())));
            }
            if lines.len() >= h {
                break;
            }
        }
        lines.truncate(h);
    } else {
        let caret = if overlay.editing { g().prompt } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!("{caret} "), styles::brand()),
            Span::styled(overlay.query.clone(), styles::fg()),
            Span::styled(if overlay.editing { "▏" } else { "" }, styles::brand()),
        ]));
        if overlay.hits.is_empty() {
            let text = if overlay.searched {
                " nothing in this session matches"
            } else {
                " type words from the conversation, then Enter"
            };
            lines.push(Line::from(Span::styled(text, styles::muted())));
        }
        let rows = h.saturating_sub(lines.len());
        let start = (overlay.selected + 1).saturating_sub(rows);
        for (i, hit) in overlay.hits.iter().enumerate().skip(start).take(rows) {
            let selected = !overlay.editing && i == overlay.selected;
            let fill = if selected {
                styles::selected_row()
            } else {
                ratatui::style::Style::default()
            };
            let caret = if selected { g().prompt } else { " " };
            let tag = if hit.folded { "folded" } else { "      " };
            let head = format!(" #{:<6} {:<9} {tag} ", hit.seq, hit.role);
            let room = width.saturating_sub(head.width() + 3);
            let mut spans = vec![
                Span::styled(format!("{caret} "), styles::brand().patch(fill)),
                Span::styled(head, styles::muted().patch(fill)),
            ];
            spans.extend(
                snippet_spans(&hit.snippet, room)
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style.patch(fill))),
            );
            lines.push(Line::from(spans));
        }
    }
    frame.render_widget(Paragraph::new(lines), body);
    let hints: Vec<(&str, &str)> = if overlay.viewing.is_some() {
        vec![("j/k", "scroll"), ("Esc", "back")]
    } else if overlay.editing {
        vec![("Enter", "search"), ("Esc", "close")]
    } else {
        vec![
            ("j/k", "move"),
            ("Enter", "read around"),
            ("/", "new search"),
            ("Esc", "close"),
        ]
    };
    let mut spans = styles::hint_row(&hints);
    if let Some(note) = &overlay.notice {
        spans.push(Span::styled(format!("   {note}"), styles::warn()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect {
            y: full.y + full.height - 1,
            height: 1,
            ..full
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn hit(seq: i64, folded: bool) -> HistoryHit {
        HistoryHit {
            seq,
            role: "assistant".into(),
            snippet: "we settled on [exponential] backoff".into(),
            folded,
        }
    }

    fn draw(o: &HistoryOverlay, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render_history_overlay(f, o)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn type_text(o: &mut HistoryOverlay, text: &str) {
        for c in text.chars() {
            o.key(KeyKind::Char(c));
        }
    }

    #[test]
    fn typing_then_enter_asks_the_host_to_search() {
        let mut o = HistoryOverlay::default();
        o.open("");
        type_text(&mut o, "backoff");
        assert_eq!(
            o.key(KeyKind::Enter),
            HistoryAction::Search("backoff".into())
        );
        o.key(KeyKind::Backspace);
        assert_eq!(o.query(), "backof");
    }

    #[test]
    fn a_one_letter_query_is_not_sent() {
        let mut o = HistoryOverlay::default();
        o.open("");
        type_text(&mut o, "a");
        assert_eq!(o.key(KeyKind::Enter), HistoryAction::None);
        assert!(o.notice.is_some());
    }

    #[test]
    fn the_command_argument_starts_a_search_once() {
        let mut o = HistoryOverlay::default();
        o.open(" retry policy ");
        assert_eq!(o.take_start().as_deref(), Some("retry policy"));
        assert_eq!(o.take_start(), None);
    }

    #[test]
    fn hits_show_which_are_folded_and_enter_reads_around_one() {
        let mut o = HistoryOverlay::default();
        o.open("backoff");
        o.set_hits(vec![hit(12, true), hit(90, false)]);
        let text = draw(&o, 100, 8);
        assert!(text.contains("2 hit(s), 1 folded"), "{text}");
        assert!(
            text.contains("#12") && text.contains("folded") && text.contains("exponential"),
            "{text}"
        );
        o.key(KeyKind::Down);
        assert_eq!(o.key(KeyKind::Enter), HistoryAction::View(90));
        o.set_view(
            90,
            "--- #89 user ---\nwhat now?\n--- #90 assistant ---\nbackoff it is",
        );
        assert!(draw(&o, 100, 8).contains("backoff it is"));
        o.key(KeyKind::Esc);
        assert!(
            o.viewing().is_none() && o.open,
            "Esc steps back before closing"
        );
        o.key(KeyKind::Esc);
        assert!(!o.open);
    }

    #[test]
    fn no_hits_returns_to_editing_and_says_so() {
        let mut o = HistoryOverlay::default();
        o.open("zzz");
        o.set_hits(Vec::new());
        assert!(draw(&o, 80, 6).contains("nothing in this session matches"));
        o.key(KeyKind::Esc);
        assert!(!o.open);
    }
}
