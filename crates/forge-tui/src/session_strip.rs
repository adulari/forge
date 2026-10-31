//! The session strip: one row at the top of the chat that lists the daemon's other sessions, so a
//! person running several can see which one is busy or waiting on them and jump to it.
//!
//! This is the renderer-independent half. The host (`forge-cli`'s `run/session_strip.rs`) keeps
//! the rows current from the daemon's `/ws/fleet` socket and `GET /api/sessions` and performs the
//! attach when [`SessionStrip::slot`] names a row; nothing here touches the network.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::board::FleetRow;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Alt+1..Alt+9 are the only slots with a key; later sessions are counted in the overflow chip.
pub const MAX_SLOTS: usize = 9;
const TITLE_MAX: usize = 22;

/// One session on the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripRow {
    pub id: String,
    pub title: String,
    pub busy: bool,
    /// Blocked on a permission or a question for the person.
    pub waiting: bool,
}

/// What the strip shows and whether it is switched on (`[tui] session_strip`).
#[derive(Debug, Clone, Default)]
pub struct SessionStrip {
    pub enabled: bool,
    rows: Vec<StripRow>,
    /// The daemon could not be reached on the last refresh.
    pub offline: bool,
}

impl SessionStrip {
    /// Rows the strip takes from the layout: one while it is on, none otherwise.
    pub fn height(&self) -> u16 {
        u16::from(self.enabled)
    }

    pub fn rows(&self) -> &[StripRow] {
        &self.rows
    }

    /// Replace the rows from a fleet read. Read-only terminal sessions with no title are kept: a
    /// session the person cannot drive is still one they may want to look at.
    pub fn set_fleet(&mut self, fleet: &[FleetRow]) {
        self.offline = false;
        self.rows = fleet
            .iter()
            .map(|row| StripRow {
                id: row.id.clone(),
                title: display_title(row),
                busy: row.busy,
                waiting: row.waiting,
            })
            .collect();
    }

    /// The row Alt+`n` (1-based) selects.
    pub fn slot(&self, n: u8) -> Option<&StripRow> {
        let index = usize::from(n).checked_sub(1)?;
        if index >= MAX_SLOTS {
            return None;
        }
        self.rows.get(index)
    }

    /// How many sessions wait on the person.
    pub fn waiting(&self) -> usize {
        self.rows.iter().filter(|r| r.waiting).count()
    }
}

fn display_title(row: &FleetRow) -> String {
    let title = row.title.trim();
    if !title.is_empty() {
        return title.to_string();
    }
    let leaf = std::path::Path::new(&row.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty());
    leaf.unwrap_or_else(|| row.id.chars().take(8).collect())
}

fn clip(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if out.width() + 1 >= max {
            break;
        }
        out.push(ch);
    }
    out.push_str(g().ellipsis);
    out
}

/// Draw the strip into the one row `area`. `own_id` is the session this chat is, which is marked.
pub fn render_strip(
    frame: &mut Frame,
    area: Rect,
    strip: &SessionStrip,
    own_id: &str,
    tick: usize,
) {
    if area.height == 0 || area.width < 8 {
        return;
    }
    let glyphs = g();
    let width = usize::from(area.width);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    if strip.rows.is_empty() {
        let note = if strip.offline {
            "sessions: daemon not reachable (forge serve)"
        } else {
            "sessions: none other running"
        };
        spans.push(Span::styled(format!(" {note}"), styles::faint()));
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }
    // Leave room for the overflow chip so the last entry never pushes it off the row.
    let reserve = 6usize;
    let mut shown = 0usize;
    for (i, row) in strip.rows.iter().take(MAX_SLOTS).enumerate() {
        let (mark, mark_style) = if row.waiting {
            (glyphs.live_dot.to_string(), styles::warn())
        } else if row.busy {
            (glyphs.spin(tick).to_string(), styles::spinner(tick))
        } else {
            ("·".to_string(), styles::faint())
        };
        let own = row.id == own_id;
        let title = clip(&row.title, TITLE_MAX);
        let cell_w = format!(" {} {} {} ", i + 1, mark, title).width();
        if used + cell_w + reserve > width && shown > 0 {
            break;
        }
        let text_style = if own || row.waiting || row.busy {
            styles::fg()
        } else {
            styles::muted()
        };
        let fill = if own {
            styles::selected_row()
        } else {
            ratatui::style::Style::default()
        };
        spans.push(Span::styled(
            format!(" {} ", i + 1),
            styles::faint().patch(fill),
        ));
        spans.push(Span::styled(format!("{mark} "), mark_style.patch(fill)));
        spans.push(Span::styled(format!("{title} "), text_style.patch(fill)));
        used += cell_w;
        shown += 1;
    }
    let hidden = strip.rows.len().saturating_sub(shown);
    if hidden > 0 {
        let chip = format!(" +{hidden}");
        used += chip.width();
        spans.push(Span::styled(chip, styles::faint()));
    }
    let hint = "Alt+1-9 switch";
    if used + hint.width() + 2 <= width {
        let pad = width - used - hint.width() - 1;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(hint, styles::faint()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn row(id: &str, title: &str, busy: bool, waiting: bool) -> FleetRow {
        FleetRow {
            id: id.into(),
            title: title.into(),
            busy,
            waiting,
            ..FleetRow::default()
        }
    }

    fn screen(strip: &SessionStrip, own: &str, width: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(width, 1)).unwrap();
        term.draw(|f| render_strip(f, f.area(), strip, own, 0))
            .unwrap();
        let buf = term.backend().buffer().clone();
        (0..width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect()
    }

    #[test]
    fn off_by_default_and_one_row_when_on() {
        let mut strip = SessionStrip::default();
        assert_eq!(strip.height(), 0);
        strip.enabled = true;
        assert_eq!(strip.height(), 1);
    }

    #[test]
    fn slots_are_one_based_and_capped_at_nine() {
        let mut strip = SessionStrip::default();
        let fleet: Vec<FleetRow> = (0..12)
            .map(|i| row(&format!("id{i}"), &format!("s{i}"), false, false))
            .collect();
        strip.set_fleet(&fleet);
        assert_eq!(strip.slot(1).unwrap().id, "id0");
        assert_eq!(strip.slot(9).unwrap().id, "id8");
        assert!(strip.slot(0).is_none());
        assert!(strip.slot(10).is_none());
    }

    #[test]
    fn an_untitled_session_is_named_by_its_directory() {
        let mut strip = SessionStrip::default();
        let mut r = row("abcdef1234567", "", false, false);
        r.cwd = "/work/forge".into();
        strip.set_fleet(&[r]);
        assert_eq!(strip.rows()[0].title, "forge");
    }

    #[test]
    fn waiting_sessions_are_counted_and_drawn_with_a_dot() {
        let mut strip = SessionStrip {
            enabled: true,
            ..SessionStrip::default()
        };
        strip.set_fleet(&[
            row("a", "alpha", true, false),
            row("b", "beta", false, true),
            row("c", "gamma", false, false),
        ]);
        assert_eq!(strip.waiting(), 1);
        let text = screen(&strip, "a", 80);
        assert!(text.contains("1") && text.contains("alpha"), "{text}");
        assert!(text.contains(g().live_dot), "{text}");
        assert!(text.contains("beta") && text.contains("gamma"), "{text}");
    }

    #[test]
    fn a_narrow_row_counts_what_it_could_not_fit() {
        let mut strip = SessionStrip {
            enabled: true,
            ..SessionStrip::default()
        };
        let fleet: Vec<FleetRow> = (0..6)
            .map(|i| row(&format!("id{i}"), "a long session title", false, false))
            .collect();
        strip.set_fleet(&fleet);
        let text = screen(&strip, "id0", 40);
        assert!(text.contains("+"), "{text}");
    }

    #[test]
    fn no_other_sessions_says_so() {
        let mut strip = SessionStrip {
            enabled: true,
            ..SessionStrip::default()
        };
        assert!(screen(&strip, "x", 60).contains("none other running"));
        strip.offline = true;
        assert!(screen(&strip, "x", 60).contains("not reachable"));
    }
}
