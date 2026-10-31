//! The background-shells list (`/jobs`): every `shell{background:true}` job of this workspace,
//! with its log tail and a way to stop it. In inline mode the live region has no room for a
//! panel, so this is the one place a person sees what is still running without asking the model.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::theme::glyphs::g;
use crate::theme::styles;
use crate::KeyKind;

/// Log lines kept for the viewer.
pub const TAIL_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub id: u32,
    pub command: String,
    pub running: bool,
    pub status: String,
    pub age_secs: u64,
}

/// What the host does after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobsAction {
    None,
    Refresh,
    View(u32),
    Stop(u32),
}

#[derive(Debug, Clone, Default)]
pub struct JobsOverlay {
    pub open: bool,
    rows: Vec<JobRow>,
    selected: usize,
    /// The job whose log is on screen, and the lines.
    viewing: Option<(u32, Vec<String>)>,
    scroll: usize,
    /// The last thing a stop or a read said.
    pub notice: Option<String>,
}

impl JobsOverlay {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            ..Self::default()
        };
    }

    pub fn rows(&self) -> &[JobRow] {
        &self.rows
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn viewing(&self) -> Option<u32> {
        self.viewing.as_ref().map(|(id, _)| *id)
    }

    pub fn set_rows(&mut self, rows: Vec<JobRow>) {
        let keep = self.current().map(|row| row.id);
        self.rows = rows;
        self.selected = keep
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
    }

    pub fn set_tail(&mut self, id: u32, text: &str) {
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        self.scroll = lines.len();
        self.viewing = Some((id, lines));
    }

    fn current(&self) -> Option<&JobRow> {
        self.rows.get(self.selected)
    }

    pub fn key(&mut self, key: KeyKind) -> JobsAction {
        if let Some((id, lines)) = &self.viewing {
            let id = *id;
            let last = lines.len().saturating_sub(1);
            match key {
                KeyKind::Esc | KeyKind::Char('q') | KeyKind::Left => self.viewing = None,
                KeyKind::Down | KeyKind::Char('j') => self.scroll = (self.scroll + 1).min(last),
                KeyKind::Up | KeyKind::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                KeyKind::PageDown => self.scroll = (self.scroll + 10).min(last),
                KeyKind::PageUp => self.scroll = self.scroll.saturating_sub(10),
                KeyKind::Home | KeyKind::Char('g') => self.scroll = 0,
                KeyKind::End | KeyKind::Char('G') => self.scroll = last,
                KeyKind::Char('r') => return JobsAction::View(id),
                KeyKind::Char('x') => return JobsAction::Stop(id),
                _ => {}
            }
            return JobsAction::None;
        }
        match key {
            KeyKind::Esc | KeyKind::Char('q') => self.open = false,
            KeyKind::Down | KeyKind::Char('j') => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
            }
            KeyKind::Up | KeyKind::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyKind::Enter | KeyKind::Right => {
                if let Some(row) = self.current() {
                    return JobsAction::View(row.id);
                }
            }
            KeyKind::Char('x') => {
                if let Some(row) = self.current().filter(|r| r.running) {
                    return JobsAction::Stop(row.id);
                }
                self.notice = Some("that job is not running".to_string());
            }
            KeyKind::Char('r') => return JobsAction::Refresh,
            _ => {}
        }
        JobsAction::None
    }
}

fn age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

fn fit(text: &str, width: usize) -> String {
    let first = text.lines().next().unwrap_or("");
    if first.width() <= width {
        return first.to_string();
    }
    let mut out: String = first.chars().take(width.saturating_sub(1)).collect();
    out.push_str(g().ellipsis);
    out
}

pub fn render_jobs_overlay(frame: &mut Frame, overlay: &JobsOverlay, tick: usize) {
    if !overlay.open {
        return;
    }
    let full = frame.area();
    frame.render_widget(Clear, full);
    if full.height < 4 || full.width < 20 {
        return;
    }
    let width = usize::from(full.width);
    let running = overlay.rows.iter().filter(|r| r.running).count();
    let title = match overlay.viewing() {
        Some(id) => format!(" Background shells  job {id} log"),
        None => format!(
            " Background shells  {running} running, {} total",
            overlay.rows.len()
        ),
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
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some((_, log)) = &overlay.viewing {
        let h = usize::from(body.height);
        if log.is_empty() {
            lines.push(Line::from(Span::styled(
                " (no output yet)",
                styles::faint(),
            )));
        }
        let end = (overlay.scroll + 1).min(log.len());
        let start = end.saturating_sub(h);
        for l in &log[start..end] {
            lines.push(Line::from(Span::styled(
                format!(" {}", fit(l, width - 2)),
                styles::fg(),
            )));
        }
    } else if overlay.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            " no background jobs in this workspace — the agent starts one with shell{background:true}",
            styles::muted(),
        )));
    } else {
        let h = usize::from(body.height);
        let start = (overlay.selected + 1).saturating_sub(h);
        for (i, row) in overlay.rows.iter().enumerate().skip(start).take(h) {
            let selected = i == overlay.selected;
            let (mark, mark_style) = if row.running {
                (g().spin(tick).to_string(), styles::spinner(tick))
            } else if row.status.starts_with("exited 0") {
                (g().ok.to_string(), styles::ok())
            } else {
                (g().fail.to_string(), styles::err())
            };
            let fill = if selected {
                styles::selected_row()
            } else {
                ratatui::style::Style::default()
            };
            let caret = if selected { g().prompt } else { " " };
            let head = format!(
                " {:<7} {:<10} {:>4}  ",
                row.id,
                row.status,
                age(row.age_secs)
            );
            let room = width.saturating_sub(head.width() + 4);
            lines.push(Line::from(vec![
                Span::styled(format!("{caret} "), styles::brand().patch(fill)),
                Span::styled(mark, mark_style.patch(fill)),
                Span::styled(head, styles::muted().patch(fill)),
                Span::styled(fit(&row.command, room), styles::fg().patch(fill)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), body);
    let hints: Vec<(&str, &str)> = if overlay.viewing.is_some() {
        vec![
            ("j/k", "scroll"),
            ("r", "reload"),
            ("x", "stop"),
            ("Esc", "back"),
        ]
    } else {
        vec![
            ("j/k", "move"),
            ("Enter", "view log"),
            ("x", "stop"),
            ("r", "refresh"),
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

    fn row(id: u32, running: bool) -> JobRow {
        JobRow {
            id,
            command: format!("npm run dev {id}"),
            running,
            status: if running {
                "running".into()
            } else {
                "exited 0".into()
            },
            age_secs: 125,
        }
    }

    fn draw(o: &JobsOverlay, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render_jobs_overlay(f, o, 0)).unwrap();
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

    #[test]
    fn lists_jobs_and_asks_the_host_to_view_and_stop() {
        let mut o = JobsOverlay::default();
        o.open();
        o.set_rows(vec![row(11, true), row(12, false)]);
        let text = draw(&o, 80, 8);
        assert!(
            text.contains("1 running, 2 total") && text.contains("npm run dev 11"),
            "{text}"
        );
        assert_eq!(o.key(KeyKind::Char('x')), JobsAction::Stop(11));
        assert_eq!(o.key(KeyKind::Enter), JobsAction::View(11));
        o.key(KeyKind::Char('j'));
        assert_eq!(
            o.key(KeyKind::Char('x')),
            JobsAction::None,
            "an exited job is not stopped"
        );
        assert!(o.notice.is_some());
    }

    #[test]
    fn the_log_view_scrolls_and_esc_steps_back_before_closing() {
        let mut o = JobsOverlay::default();
        o.open();
        o.set_rows(vec![row(11, true)]);
        o.set_tail(11, "one\ntwo\nthree");
        assert!(draw(&o, 80, 8).contains("three"));
        o.key(KeyKind::Char('k'));
        o.key(KeyKind::Char('k'));
        assert!(!draw(&o, 80, 8).contains("three"));
        assert_eq!(o.key(KeyKind::Char('r')), JobsAction::View(11));
        o.key(KeyKind::Esc);
        assert!(o.viewing().is_none() && o.open);
        o.key(KeyKind::Esc);
        assert!(!o.open);
    }

    #[test]
    fn the_selection_follows_its_job_when_the_list_reorders() {
        let mut o = JobsOverlay::default();
        o.open();
        o.set_rows(vec![row(1, true), row(2, true)]);
        o.key(KeyKind::Down);
        o.set_rows(vec![row(3, true), row(1, true), row(2, true)]);
        assert_eq!(o.key(KeyKind::Char('x')), JobsAction::Stop(2));
        o.set_rows(vec![row(3, true)]);
        assert_eq!(o.selected(), 0);
    }

    #[test]
    fn an_empty_list_explains_itself() {
        let mut o = JobsOverlay::default();
        o.open();
        assert!(draw(&o, 100, 6).contains("no background jobs"));
    }
}
