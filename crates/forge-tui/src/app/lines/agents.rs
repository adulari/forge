//! Scrollback line builders: agents.

use crate::theme::{color, styles};
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

use crate::app::{truncate, width_cap};

/// Opens the subagent group box in scrollback.
pub(crate) fn subagent_header_line() -> TextLine<'static> {
    TextLine::from(vec![
        Span::styled("  ╭─ ", styles::muted()),
        Span::styled("subagents", styles::muted().bold()),
        Span::styled(" ─────────────", styles::muted()),
    ])
}

/// One completed subagent as a branch of the group box.
pub(crate) fn subagent_branch_line(
    agent: &str,
    ok: bool,
    cost_usd: f64,
    summary: &str,
    width: u16,
) -> TextLine<'static> {
    let (mark, color) = if ok {
        ("✓", color::ok())
    } else {
        ("✗", color::err())
    };
    let cap = width_cap(width, 18 + agent.chars().count(), 44);
    TextLine::from(vec![
        Span::styled("  ├─ ", styles::muted()),
        Span::styled(format!("{mark} "), Style::default().fg(color)),
        Span::styled(format!("[{agent}] "), styles::muted()),
        Span::styled(format!("${cost_usd:.4}  "), styles::muted()),
        Span::styled(truncate(summary, cap), styles::muted()),
    ])
}

/// Scrollback record of a workflow run (the live rendering is the dedicated workflow view; these
/// lines are the durable history that remains in the transcript after it closes).
pub(crate) fn workflow_started_line(name: Option<&str>) -> TextLine<'static> {
    let label = match name {
        Some(n) => format!("  ⛓ workflow '{n}' started"),
        None => "  ⛓ workflow started".to_string(),
    };
    TextLine::from(Span::styled(label, styles::brand().bold()))
}

pub(crate) fn workflow_phase_line(title: &str) -> TextLine<'static> {
    TextLine::from(Span::styled(
        format!("  ▶ phase: {title}"),
        styles::warn().bold(),
    ))
}

pub(crate) fn workflow_log_line(msg: &str, max: usize) -> TextLine<'static> {
    let first = msg.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    TextLine::from(vec![
        Span::styled("  💬 ", styles::user()),
        Span::styled(truncate(first, max), styles::fg()),
    ])
}

pub(crate) fn workflow_finished_line(ok: bool, summary: &str, max: usize) -> TextLine<'static> {
    let (glyph, color) = if ok {
        ("✓", color::ok())
    } else {
        ("⚠", color::warn())
    };
    TextLine::from(vec![
        Span::styled(format!("  ⛓ {glyph} "), Style::default().fg(color).bold()),
        Span::styled(
            format!("workflow finished: {}", truncate(summary, max)),
            styles::fg(),
        ),
    ])
}

/// Closes the subagent group box with a total.
pub(crate) fn subagent_footer_line(n: usize, total_usd: f64) -> TextLine<'static> {
    TextLine::from(Span::styled(
        format!("  ╰─ {n} agents · ${total_usd:.4}"),
        styles::muted(),
    ))
}
