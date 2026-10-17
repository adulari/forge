//! Drawing the workflow view: the full-frame takeover (header row, phase checklist with nested
//! agents, log, hint row), the Enter-zoom transcript, and the one-row band shown while the view
//! is backgrounded.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{row_status, WfRow, WorkflowView};
use crate::app::{model_short, truncate, ActivityStatus, App};
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Time a row has been observed running (or ran for), through the panels' shared clock.
fn row_elapsed(app: &App, row: &WfRow) -> Option<std::time::Duration> {
    app.panel_elapsed(&format!("wf:{}", row.id), !row.done, row.done)
}

use crate::overlays::{fmt_cost, fmt_elapsed};

fn elapsed_secs(secs: u64) -> String {
    fmt_elapsed(std::time::Duration::from_secs(secs))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn width_of(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// `left`, then `right` flush against the last column (one cell of air).
fn aligned(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> TextLine<'static> {
    let right_w = width_of(&right);
    let room = width.saturating_sub(right_w + 2).max(1);
    let mut spans = clip(left, room);
    let used = width_of(&spans);
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + right_w + 1)),
    ));
    spans.extend(right);
    TextLine::from(spans)
}

/// Clip spans to `max` cells with a trailing ellipsis.
fn clip(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    if width_of(&spans) <= max {
        return spans;
    }
    let mut out = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let mut text = String::new();
        for ch in span.content.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + w + 1 > max {
                text.push_str(g().ellipsis);
                out.push(Span::styled(text, span.style));
                return out;
            }
            used += w;
            text.push(ch);
        }
        out.push(Span::styled(text, span.style));
    }
    out
}

/// The phase a live run is in: the last one announced (1-based), or `None` before the first.
fn current_phase(wf: &WorkflowView) -> Option<usize> {
    (!wf.phases.is_empty()).then_some(wf.phases.len())
}

/// The one-row band shown above the composer while a live run's view is backgrounded:
/// `✦ workflow name · phase 2 · 2 agents running` on the left, elapsed and cost on the right.
pub(crate) fn workflow_band_line(app: &App, width: u16) -> TextLine<'static> {
    let wf = &app.workflow;
    let glyphs = g();
    let (running, done, failed, cost) = wf.totals();
    // Keep every row's clock running while the view is closed.
    for row in &wf.rows {
        let _ = row_elapsed(app, row);
    }
    let sep = format!(" {} ", glyphs.sep);
    let mut left = vec![
        Span::styled(
            format!(" {} ", glyphs.spin(app.tick)),
            styles::spinner(app.tick),
        ),
        Span::styled("workflow", styles::brand().bold()),
    ];
    if let Some(name) = &wf.name {
        left.push(Span::styled(format!(" {name}"), styles::fg().bold()));
    }
    if let Some(phase) = current_phase(wf) {
        left.push(Span::styled(format!("{sep}phase {phase}"), styles::muted()));
    }
    left.push(Span::styled(
        format!("{sep}{} running", plural(running, "agent", "agents")),
        styles::muted(),
    ));
    left.push(Span::styled(format!("{sep}{done} done"), styles::muted()));
    if failed > 0 {
        left.push(Span::styled(format!("{sep}{failed} failed"), styles::err()));
    }
    let right = vec![
        Span::styled(
            format!(
                "{}{sep}{}  ",
                elapsed_secs(app.turn_elapsed_secs),
                fmt_cost(cost)
            ),
            styles::faint(),
        ),
        Span::styled("Ctrl+O", styles::key_chip()),
        Span::styled(" view", styles::faint()),
    ];
    aligned(left, right, width as usize)
}

/// Phase glyph and its status word: `✓ 3 agents 1m 02s`, `⠹ 2 agents running`, `○ starting`.
fn phase_state(
    app: &App,
    wf: &WorkflowView,
    members: &[usize],
) -> (Span<'static>, String, Option<std::time::Duration>) {
    let glyphs = g();
    let done = members.iter().filter(|&&i| wf.rows[i].done).count();
    let failed = members
        .iter()
        .filter(|&&i| wf.rows[i].done && !wf.rows[i].ok)
        .count();
    let elapsed = members
        .iter()
        .filter_map(|&i| row_elapsed(app, &wf.rows[i]))
        .max();
    if members.is_empty() {
        return (
            Span::styled(glyphs.task_open, styles::faint()),
            "starting".into(),
            None,
        );
    }
    let count = plural(members.len(), "agent", "agents");
    if done < members.len() {
        (
            Span::styled(glyphs.task_active, styles::brand()),
            format!("{count}   running"),
            elapsed,
        )
    } else if failed > 0 {
        (
            Span::styled(glyphs.fail, styles::err()),
            format!("{count}   {failed} failed"),
            elapsed,
        )
    } else {
        (Span::styled(glyphs.task_done, styles::ok()), count, elapsed)
    }
}

/// One agent row, nested under its phase: spinner or outcome, name, model, what it is doing,
/// elapsed and cost flush right. The selected row is raised with a `❯` caret.
fn agent_row(app: &App, r: &WfRow, selected: bool, width: usize) -> TextLine<'static> {
    let glyphs = g();
    let (mark, mark_style) = match row_status(r) {
        ActivityStatus::Running => (glyphs.spin(app.tick).to_string(), styles::spinner(app.tick)),
        ActivityStatus::Done => (glyphs.ok.to_string(), styles::ok()),
        _ => (glyphs.fail.to_string(), styles::err()),
    };
    let model = model_short(r.model.as_deref());
    let action = if !r.done && !r.last.trim().is_empty() {
        r.last.trim().to_string()
    } else if r.done {
        format!(
            "{} {} {}",
            if r.ok { "done" } else { "failed" },
            glyphs.sep,
            r.task
        )
    } else {
        r.task.clone()
    };
    let name_style = if selected {
        styles::fg().bold()
    } else if r.done && !r.ok {
        styles::err()
    } else {
        styles::fg()
    };
    let caret = if selected { glyphs.prompt } else { " " };
    let mut right = Vec::new();
    if let Some(d) = row_elapsed(app, r) {
        right.push(Span::styled(fmt_elapsed(d), styles::faint()));
    }
    if r.cost > 0.0 {
        right.push(Span::styled(
            format!("  {}", fmt_cost(r.cost)),
            styles::faint(),
        ));
    }
    let left = vec![
        Span::styled(
            format!("   {caret} "),
            if selected {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(format!("{mark} "), mark_style),
        Span::styled(format!("{:<12} ", truncate(&r.agent, 12)), name_style),
        Span::styled(format!("{:<14} ", truncate(&model, 14)), styles::muted()),
        Span::styled(truncate(&action, width.max(40)), styles::muted()),
    ];
    let row = aligned(left, right, width);
    if !selected {
        return row;
    }
    let fill = styles::selected_row();
    TextLine::from(
        row.spans
            .into_iter()
            .map(|mut s| {
                s.style = s.style.patch(fill);
                s
            })
            .collect::<Vec<_>>(),
    )
}

/// The header row: `✦ workflow name` left; phase, agents, elapsed, cost and the back key right.
fn header_row(app: &App, wf: &WorkflowView, width: usize) -> TextLine<'static> {
    let glyphs = g();
    let (running, done, failed, cost) = wf.totals();
    let total = wf.rows.len();
    let sep = format!(" {} ", glyphs.sep);
    let mut left = vec![
        Span::styled(format!(" {} ", glyphs.brand), styles::brand().bold()),
        Span::styled("workflow", styles::fg().bold()),
    ];
    if let Some(name) = &wf.name {
        left.push(Span::styled(format!("  {name}"), styles::fg().bold()));
    }
    match &wf.finished {
        None => {}
        Some((true, _)) => left.push(Span::styled(
            format!("  {} finished", glyphs.ok),
            styles::ok(),
        )),
        Some((false, _)) => {
            left.push(Span::styled(
                format!("  {} finished with errors", glyphs.fail),
                styles::err(),
            ));
        }
    }
    let mut facts = Vec::new();
    if let Some(phase) = current_phase(wf) {
        facts.push(format!("phase {phase}"));
    }
    let mut agents = format!("{}/{total} agents", done + failed);
    if running > 0 {
        agents.push_str(&format!("{sep}{running} running"));
    }
    if failed > 0 {
        agents.push_str(&format!("{sep}{failed} failed"));
    }
    facts.push(agents);
    facts.push(elapsed_secs(app.turn_elapsed_secs));
    facts.push(fmt_cost(cost));
    let right = vec![
        Span::styled(facts.join(&sep), styles::muted()),
        Span::styled(sep.clone(), styles::faint()),
        Span::styled("Esc", styles::key_chip()),
        Span::styled(" background", styles::faint()),
    ];
    aligned(left, right, width)
}

/// The phase checklist with nested agent rows. Returns the lines and, per line, the row index
/// it belongs to (so the cursor line is known for scrolling).
fn body_lines(
    app: &App,
    wf: &WorkflowView,
    width: usize,
) -> Vec<(TextLine<'static>, Option<usize>)> {
    let glyphs = g();
    let mut body: Vec<(TextLine<'static>, Option<usize>)> = Vec::new();
    let groups: Vec<Option<usize>> = std::iter::once(None)
        .chain((0..wf.phases.len()).map(Some))
        .collect();
    for group in groups {
        let members: Vec<usize> = wf
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.phase_idx == group)
            .map(|(i, _)| i)
            .collect();
        match group {
            Some(pi) => {
                // A phase renders before its first agent spawns: live proof the script advanced.
                let (mark, status, elapsed) = phase_state(app, wf, &members);
                let left = vec![
                    Span::raw("  "),
                    mark,
                    Span::styled(format!(" {}  ", pi + 1), styles::muted()),
                    Span::styled(
                        format!("{:<20}", truncate(&wf.phases[pi].title, 20)),
                        styles::fg().bold(),
                    ),
                    Span::styled(status, styles::muted()),
                ];
                let right = elapsed
                    .map(|d| vec![Span::styled(fmt_elapsed(d), styles::faint())])
                    .unwrap_or_default();
                body.push((aligned(left, right, width), None));
            }
            None if members.is_empty() => continue,
            None => {}
        }
        for i in members {
            let selected = i == wf.selected.min(wf.rows.len().saturating_sub(1));
            body.push((agent_row(app, &wf.rows[i], selected, width), Some(i)));
        }
    }
    if wf.rows.is_empty() && wf.phases.is_empty() {
        body.push((
            TextLine::from(Span::styled(
                format!(
                    "  {} authoring{} waiting for the first agent to spawn",
                    glyphs.spin(app.tick),
                    glyphs.ellipsis
                ),
                styles::muted(),
            )),
            None,
        ));
    }
    body
}

/// Render the workflow view over the whole frame. Called by `render_live` while
/// [`WorkflowView::open`]; the Enter-zoom transcript takes over the frame while `zoom` is set.
pub(crate) fn render_workflow_view(f: &mut Frame, app: &App) {
    let wf = &app.workflow;
    let area = f.area();
    f.render_widget(Clear, area);

    // Enter-zoom: the selected agent's transcript, via the shared viewer renderer.
    if let Some(z) = &wf.zoom {
        let selected = wf.selected.min(wf.rows.len().saturating_sub(1));
        let views = wf.zoom_views(Some(selected));
        if !views.is_empty() {
            let scroll = if z.follow { usize::MAX / 2 } else { z.scroll };
            let width = area.width.saturating_sub(1);
            let wrapped = wf.ensure_zoom_wrapped(&views, selected, width);
            wf.zoom_geom.set(Some((
                wrapped.rows.len(),
                area.height.saturating_sub(3).max(1),
            )));
            f.render_widget(
                Paragraph::new(crate::transcript::transcript_lines_from_wrapped(
                    &views,
                    selected,
                    scroll,
                    area.height,
                    area.width,
                    &wrapped.rows,
                )),
                area,
            );
            return;
        }
    }
    wf.zoom_geom.set(None);
    wf.forget_zoom_wrapped();

    if area.width < 24 || area.height < 4 {
        return;
    }
    let width = area.width as usize;
    let glyphs = g();

    // Header row, then a blank row, the result summary once finished, the body, the log, and
    // the key hints in the last row.
    let mut top: Vec<TextLine<'static>> = vec![header_row(app, wf, width), TextLine::default()];
    if let Some((ok, summary)) = &wf.finished {
        let (glyph, style) = if *ok {
            (glyphs.ok, styles::ok())
        } else {
            (glyphs.warn, styles::warn())
        };
        top.push(TextLine::from(vec![
            Span::styled(format!("  {glyph} "), style),
            Span::styled(truncate(summary, width.saturating_sub(6)), styles::muted()),
        ]));
        top.push(TextLine::default());
    }
    let top_h = (top.len() as u16).min(area.height.saturating_sub(2));
    let feed_rows = if wf.logs.is_empty() {
        0
    } else {
        wf.logs.len().min(4) + 2
    };
    let feed_h = (feed_rows as u16).min(area.height.saturating_sub(top_h + 2));
    let chunks = Layout::vertical([
        Constraint::Length(top_h),
        Constraint::Min(1),
        Constraint::Length(feed_h),
        Constraint::Length(1),
    ])
    .split(area);
    f.render_widget(Paragraph::new(top), chunks[0]);

    // Reveal rows over the first frames, then keep the selected row visible: stay at the top
    // until the cursor passes the viewport, then follow it.
    let body = body_lines(app, wf, width);
    let revealed = (((wf.anim_tick as usize) * 2).max(1)).min(body.len());
    let cursor_line = body
        .iter()
        .take(revealed)
        .position(|(_, idx)| *idx == Some(wf.selected.min(wf.rows.len().saturating_sub(1))))
        .unwrap_or(0) as u16;
    let body_h = chunks[1].height;
    let max_scroll = (revealed as u16).saturating_sub(body_h);
    let scroll = if cursor_line < body_h {
        0
    } else {
        (cursor_line + 1).saturating_sub(body_h)
    }
    .min(max_scroll);
    let body_lines: Vec<TextLine> = body.into_iter().take(revealed).map(|(l, _)| l).collect();
    f.render_widget(Paragraph::new(body_lines).scroll((scroll, 0)), chunks[1]);

    if feed_h > 0 {
        let mut feed = vec![
            TextLine::default(),
            TextLine::from(Span::styled("  LOG", styles::muted().bold())),
        ];
        let shown = (feed_h as usize).saturating_sub(2);
        for msg in wf.logs.iter().rev().take(shown).rev() {
            feed.push(TextLine::from(vec![
                Span::styled(format!("  {} ", glyphs.info), styles::faint()),
                Span::styled(truncate(msg, width.saturating_sub(6).max(8)), styles::fg()),
            ]));
        }
        f.render_widget(Paragraph::new(feed), chunks[2]);
    }

    let mut hints = styles::hint_row(&[
        (glyphs.up_down, "select agent"),
        ("Enter", "zoom"),
        ("Esc", "background"),
        ("Ctrl+C", "stop"),
    ]);
    hints.insert(0, Span::raw(" "));
    f.render_widget(Paragraph::new(TextLine::from(hints)), Rect { ..chunks[3] });
}
