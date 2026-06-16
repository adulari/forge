//! Rendering for the in-place subagent transcript browser (Ctrl+O in the chat). Unlike a blocking
//! alternate-screen modal, this is drawn by the normal render loop from the LIVE `App` state, so
//! the selected child's log auto-updates as new progress arrives. Pure (no terminal I/O) → the
//! layout is unit-tested; the live viewport just grows to host it (see `App::live_height`).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TextLine, Span};

use crate::app::SubagentView;

// Brand palette (mirrors the per-module consts elsewhere in the crate).
const ORANGE: Color = Color::Rgb(255, 145, 60);
const DIM: Color = Color::Rgb(110, 110, 120);
const TOOLCYAN: Color = Color::Rgb(120, 200, 215);
const BODY: Color = Color::Rgb(205, 205, 215);

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    } else {
        s.to_string()
    }
}

/// Build the transcript view for `views[selected]` scrolled to `scroll`, sized to `height`×`width`:
/// a header (which child, of how many, + status/cost), the visible slice of that child's live log,
/// and a footer with the position + key hints. Pure — unit-tested. `selected`/`scroll` are clamped.
pub fn transcript_lines(
    views: &[SubagentView],
    selected: usize,
    scroll: usize,
    height: u16,
    width: u16,
) -> Vec<TextLine<'static>> {
    let h = height as usize;
    if views.is_empty() {
        return vec![
            TextLine::from(Span::styled(
                "  ⚒ subagent transcript",
                Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
            )),
            TextLine::from(Span::styled(
                "  no subagents in this batch yet",
                Style::default().fg(DIM),
            )),
        ];
    }
    let selected = selected.min(views.len() - 1);
    let view = &views[selected];
    let status = if view.done { "done" } else { "running" };
    let title_w = (width as usize).saturating_sub(40);
    let mut lines = vec![
        TextLine::from(vec![
            Span::styled(
                format!("  ⚒ transcript [{}/{}] ", selected + 1, views.len()),
                Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} ", view.agent),
                Style::default().fg(TOOLCYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("· {status} · ${:.4}", view.cost),
                Style::default().fg(DIM),
            ),
        ]),
        TextLine::from(Span::styled(
            format!("  {}", truncate(&view.task, title_w.max(10))),
            Style::default().fg(DIM),
        )),
    ];

    // Body: the log window. Reserve 2 header + 1 footer rows.
    let body_h = h.saturating_sub(3).max(1);
    let total = view.log.len();
    let max_scroll = total.saturating_sub(body_h);
    let scroll = scroll.min(max_scroll);
    if view.log.is_empty() {
        lines.push(TextLine::from(Span::styled(
            "  (no activity captured yet)",
            Style::default().fg(DIM),
        )));
    }
    for entry in view.log.iter().skip(scroll).take(body_h) {
        lines.push(TextLine::from(Span::styled(
            format!("  {}", truncate(entry, width.saturating_sub(2) as usize)),
            Style::default().fg(BODY),
        )));
    }
    // Pad so the footer sits at the bottom of the region.
    while lines.len() < h.saturating_sub(1) {
        lines.push(TextLine::default());
    }
    let shown_end = (scroll + body_h).min(total);
    lines.push(TextLine::from(Span::styled(
        format!(
            "  ── {}-{}/{} lines · ↑↓ scroll · ←→ switch agent · Esc close ──",
            scroll.min(total.saturating_sub(1)) + usize::from(total > 0),
            shown_end,
            total
        ),
        Style::default().fg(DIM),
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(agent: &str, n: usize, done: bool) -> SubagentView {
        SubagentView {
            agent: agent.into(),
            task: "scan the repo".into(),
            done,
            cost: 0.0,
            log: (0..n).map(|i| format!("line {i}")).collect(),
        }
    }

    #[test]
    fn header_shows_selected_of_total_and_agent() {
        let vs = vec![view("alpha", 3, false), view("beta", 1, true)];
        let txt = render(&transcript_lines(&vs, 1, 0, 20, 80));
        assert!(txt.contains("[2/2]"), "selector: {txt}");
        assert!(txt.contains("beta"));
        assert!(txt.contains("done"));
    }

    #[test]
    fn scroll_offsets_the_log_window() {
        let vs = vec![view("a", 40, false)];
        let body: Vec<String> = transcript_lines(&vs, 0, 5, 12, 80)
            .iter()
            .filter_map(|l| l.spans.first().map(|s| s.content.trim().to_string()))
            .filter(|s| s.starts_with("line "))
            .collect();
        assert_eq!(body.first().unwrap(), "line 5");
        assert!(!body.iter().any(|s| s == "line 0"));
    }

    #[test]
    fn selected_and_scroll_are_clamped() {
        let vs = vec![view("a", 3, false)];
        // Out-of-range selected + scroll must not panic.
        assert!(!transcript_lines(&vs, 99, 999, 10, 80).is_empty());
    }

    #[test]
    fn empty_views_render_placeholder() {
        let txt = render(&transcript_lines(&[], 0, 0, 10, 80));
        assert!(txt.contains("no subagents"));
    }

    fn render(lines: &[TextLine]) -> String {
        lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}
