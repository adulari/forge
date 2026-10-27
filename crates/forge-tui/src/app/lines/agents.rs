//! Scrollback line builders: subagent groups and workflow history.

use crate::theme::glyphs::g;
use crate::theme::styles::{self, NoticeKind};
use ratatui::text::{Line as TextLine, Span};

use crate::app::render::panels::fmt_cost;
use crate::app::{truncate, width_cap};

/// Opens the subagent group: `◆ Agents` with faint branches beneath.
pub(crate) fn subagent_header_line() -> TextLine<'static> {
    TextLine::from(vec![
        Span::styled(format!("  {} ", g().group), styles::brand()),
        Span::styled("Agents", styles::fg().bold()),
        Span::styled("  Ctrl+O, then Enter opens one", styles::faint()),
    ])
}

/// One completed subagent as a branch of the group.
pub(crate) fn subagent_branch_line(
    agent: &str,
    ok: bool,
    cost_usd: f64,
    summary: &str,
    width: u16,
) -> TextLine<'static> {
    let glyphs = g();
    let (mark, style) = if ok {
        (glyphs.ok, styles::ok())
    } else {
        (glyphs.fail, styles::err())
    };
    let branch = if glyphs.unicode { "├" } else { "|-" };
    let cost = fmt_cost(cost_usd);
    let cap = width_cap(width, 12 + agent.chars().count() + cost.chars().count(), 40);
    TextLine::from(vec![
        Span::styled(format!("  {branch} "), styles::faint()),
        Span::styled(format!("{mark} "), style),
        Span::styled(agent.to_string(), styles::fg()),
        Span::styled(format!("  {cost}  "), styles::faint()),
        Span::styled(truncate(summary, cap), styles::muted()),
    ])
}

/// Closes the subagent group with a count and total.
pub(crate) fn subagent_footer_line(n: usize, total_usd: f64) -> TextLine<'static> {
    let end = if g().unicode { "└" } else { "`-" };
    TextLine::from(vec![
        Span::styled(format!("  {end} "), styles::faint()),
        Span::styled(
            format!(
                "{n} agent{} {} {}",
                if n == 1 { "" } else { "s" },
                g().sep,
                fmt_cost(total_usd)
            ),
            styles::muted(),
        ),
    ])
}

fn notice_line(kind: NoticeKind, body: Vec<Span<'static>>) -> TextLine<'static> {
    let (glyph, style) = styles::notice(kind);
    let mut spans = vec![Span::styled(format!(" {glyph} "), style)];
    spans.extend(body);
    TextLine::from(spans)
}

/// Scrollback record of a workflow run (the live rendering is the dedicated workflow view; these
/// lines are the durable history that remains in the transcript after it closes). They use the
/// notice grammar: a glyph at column 1, text hanging at column 3.
pub(crate) fn workflow_started_line(name: Option<&str>) -> TextLine<'static> {
    let mut body = vec![Span::styled("workflow ", styles::fg())];
    if let Some(name) = name {
        body.push(Span::styled(format!("{name} "), styles::fg().bold()));
    }
    body.push(Span::styled("started", styles::muted()));
    notice_line(NoticeKind::Info, body)
}

pub(crate) fn workflow_phase_line(title: &str) -> TextLine<'static> {
    TextLine::from(vec![
        Span::styled(format!(" {} ", g().group), styles::warn()),
        Span::styled("phase ", styles::muted()),
        Span::styled(title.to_string(), styles::fg().bold()),
    ])
}

pub(crate) fn workflow_log_line(msg: &str, max: usize) -> TextLine<'static> {
    let first = msg.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    notice_line(
        NoticeKind::Info,
        vec![Span::styled(truncate(first, max), styles::muted())],
    )
}

pub(crate) fn workflow_finished_line(ok: bool, summary: &str, max: usize) -> TextLine<'static> {
    let kind = if ok { NoticeKind::Ok } else { NoticeKind::Warn };
    notice_line(
        kind,
        vec![
            Span::styled("workflow finished", styles::fg()),
            Span::styled(
                format!(" {} {}", g().sep, truncate(summary, max)),
                styles::muted(),
            ),
        ],
    )
}
