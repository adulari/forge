//! The tasks and agents band between the transcript and the composer: one summary row while
//! collapsed, the full lists while details are open (`Ctrl+O`).

use super::common::{aligned, fmt_cost, fmt_elapsed};
use super::*;
use crate::app::render::{human, model_short};
use crate::theme::glyphs::g;
use crate::theme::styles;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Done and total task counts.
pub(crate) fn task_counts(tasks: &[forge_types::TodoItem]) -> (usize, usize) {
    let done = tasks
        .iter()
        .filter(|t| t.status == forge_types::TodoStatus::Done)
        .count();
    (done, tasks.len())
}

/// The glyph and style of a task row.
fn task_mark(status: forge_types::TodoStatus) -> (&'static str, ratatui::style::Style) {
    use forge_types::TodoStatus::*;
    let glyphs = g();
    match status {
        Done => (glyphs.task_done, styles::ok()),
        InProgress => (glyphs.task_active, styles::brand()),
        Pending => (glyphs.task_open, styles::faint()),
    }
}

/// The task the summary names: the one in progress, else the next open one, else a done note.
fn headline_task(
    tasks: &[forge_types::TodoItem],
) -> Option<(&'static str, ratatui::style::Style, String)> {
    use forge_types::TodoStatus::*;
    let pick = tasks
        .iter()
        .find(|t| t.status == InProgress)
        .or_else(|| tasks.iter().find(|t| t.status == Pending));
    match pick {
        Some(task) => {
            let (glyph, style) = task_mark(task.status);
            Some((glyph, style, task.title.clone()))
        }
        None if !tasks.is_empty() => Some((g().task_done, styles::ok(), "all done".to_string())),
        None => None,
    }
}

/// The collapsed band: `▸ Tasks 1/3   ◉ title` on the left, running agents and the `Ctrl+O` key
/// flush right.
pub(crate) fn render_summary(frame: &mut Frame, area: Rect, app: &App) {
    let glyphs = g();
    let width = area.width as usize;
    let agents = app.running_subagents();
    let critics = app.running_assay_critics();
    let (done, total) = task_counts(&app.tasks);

    let mut left = vec![Span::styled(
        format!(" {} ", glyphs.disclose_closed),
        styles::faint(),
    )];
    if total > 0 {
        left.push(Span::styled("Tasks", styles::fg().bold()));
        left.push(Span::styled(format!(" {done}/{total}"), styles::muted()));
        if let Some((glyph, style, title)) = headline_task(&app.tasks) {
            left.push(Span::raw("   "));
            left.push(Span::styled(format!("{glyph} "), style));
            left.push(Span::styled(title, styles::fg()));
        }
    } else {
        left.push(Span::styled("Agents", styles::fg().bold()));
        let n = app.activity_len().saturating_sub(1);
        left.push(Span::styled(format!(" {n}"), styles::muted()));
    }

    let mut status = Vec::new();
    if agents > 0 {
        status.push(format!("{} running", plural(agents, "agent", "agents")));
    }
    if critics > 0 {
        status.push(plural(critics, "review", "reviews"));
    }
    let tail = |with_status: bool| {
        let mut right = Vec::new();
        if with_status && !status.is_empty() {
            right.push(Span::styled(
                format!("{} {} ", status.join(" · "), glyphs.sep),
                styles::muted(),
            ));
        }
        right.push(Span::styled("Ctrl+O", styles::key_chip()));
        right.push(Span::styled(" details", styles::faint()));
        right
    };
    let left_w: usize = super::common::spans_width(&left);
    let full = tail(true);
    let full_w: usize = super::common::spans_width(&full);
    let right = if left_w.min(30) + full_w + 3 <= width {
        full
    } else {
        tail(false)
    };
    frame.render_widget(Paragraph::new(aligned(left, right, width, 1)), area);
}

/// A selected row gets the raised fill (where fills are painted), bold text and a `❯` caret; the
/// rest of the row is padded so the fill spans the band.
fn select(mut spans: Vec<Span<'static>>, selected: bool, width: usize) -> Vec<Span<'static>> {
    if !selected {
        return spans;
    }
    let fill = styles::selected_row();
    for span in &mut spans {
        span.style = span.style.patch(fill);
    }
    let used = super::common::spans_width(&spans);
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), fill));
    spans
}

/// The expanded tasks section: header with counts, then a row per task with its glyph and
/// elapsed time. When the region is short, open work wins the rows and finished tasks fold into
/// `… +N completed`.
pub(crate) fn task_section_lines(app: &App, height: u16, width: u16) -> Vec<TextLine<'static>> {
    use forge_types::TodoStatus;
    let glyphs = g();
    let h = height as usize;
    let w = width as usize;
    let tasks = &app.tasks;
    let (done, total) = task_counts(tasks);
    let header = aligned(
        vec![
            Span::styled(format!(" {} ", glyphs.disclose_open), styles::faint()),
            Span::styled("Tasks", styles::fg().bold()),
            Span::styled(format!(" {done}/{total}"), styles::muted()),
        ],
        if app.has_activity() {
            Vec::new()
        } else {
            vec![
                Span::styled("Ctrl+O", styles::key_chip()),
                Span::styled(" hide", styles::faint()),
            ]
        },
        w,
        1,
    );
    let mut lines = vec![header];
    let body_h = h.saturating_sub(1);
    let mut order: Vec<usize> = (0..total).collect();
    let mut folded = 0usize;
    if total > body_h {
        let open: Vec<usize> = order
            .iter()
            .copied()
            .filter(|&i| tasks[i].status != TodoStatus::Done)
            .collect();
        let finished: Vec<usize> = order
            .iter()
            .copied()
            .filter(|&i| tasks[i].status == TodoStatus::Done)
            .collect();
        let room = body_h
            .saturating_sub(open.len())
            .saturating_sub(usize::from(!finished.is_empty()));
        let shown = room.min(finished.len());
        folded = finished.len() - shown;
        let keep: std::collections::HashSet<usize> = open
            .iter()
            .copied()
            .chain(finished.iter().copied().take(shown))
            .collect();
        order.retain(|i| keep.contains(i));
    }
    for i in order {
        let task = &tasks[i];
        let (glyph, style) = task_mark(task.status);
        let elapsed = app.panel_elapsed(
            &format!("task:{}", task.title),
            task.status == TodoStatus::InProgress,
            task.status == TodoStatus::Done,
        );
        let right = elapsed
            .map(|d| vec![Span::styled(fmt_elapsed(d), styles::faint())])
            .unwrap_or_default();
        let title_style = match task.status {
            TodoStatus::Done => styles::muted(),
            TodoStatus::InProgress => styles::fg().bold(),
            TodoStatus::Pending => styles::fg(),
        };
        let mut left = vec![
            Span::styled(format!("   {glyph} "), style),
            Span::styled(task.title.clone(), title_style),
        ];
        if let Some(who) = task
            .assignee
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            left.push(Span::styled(format!(" @{who}"), styles::muted()));
        }
        lines.push(aligned(left, right, w, 1));
    }
    if folded > 0 {
        lines.push(TextLine::from(Span::styled(
            format!("     {} +{folded} completed", glyphs.ellipsis),
            styles::faint(),
        )));
    }
    lines
}

/// The glyph, style and trailing detail of an activity entry.
fn activity_mark(app: &App, v: &ActivitySummary) -> (String, ratatui::style::Style) {
    let glyphs = g();
    match (v.kind, v.status) {
        (ActivityKind::MainChat, _) => (glyphs.live_dot.to_string(), styles::faint()),
        (ActivityKind::AssayCritic, ActivityStatus::Running) => {
            (glyphs.group.to_string(), styles::warn())
        }
        (_, ActivityStatus::Running) => {
            (glyphs.spin(app.tick).to_string(), styles::spinner(app.tick))
        }
        (_, ActivityStatus::Done) => (glyphs.ok.to_string(), styles::ok()),
        (_, ActivityStatus::Failed) => (glyphs.fail.to_string(), styles::err()),
        (_, ActivityStatus::Skipped) => ("-".to_string(), styles::faint()),
    }
}

/// What an entry is doing now: a subagent's latest progress line, else its task or focus.
fn activity_detail(app: &App, index: usize, v: &ActivitySummary) -> String {
    match v.kind {
        ActivityKind::Subagent => app
            .subagents
            .get(index.saturating_sub(1))
            .map(|r| r.last.trim())
            .filter(|last| !last.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| v.subtitle.clone()),
        ActivityKind::AssayCritic => v.subtitle.clone(),
        ActivityKind::MainChat => format!("{} lines", v.line_count),
    }
}

/// The expanded agents section. When focused (`Ctrl+O`) the selected row is raised and `↑↓` move
/// it; `Enter` opens that entry's transcript.
pub(crate) fn activity_section_lines(app: &App, height: u16, width: u16) -> Vec<TextLine<'static>> {
    let glyphs = g();
    let h = height as usize;
    let w = width as usize;
    let views = app.activity_summaries();
    if views.is_empty() || h == 0 {
        return Vec::new();
    }
    let focused = app.activity_focused;
    let n = views.len().saturating_sub(1);
    let mut lines = vec![aligned(
        vec![
            Span::styled(format!(" {} ", glyphs.disclose_open), styles::faint()),
            Span::styled("Agents", styles::fg().bold()),
            Span::styled(format!(" {n}"), styles::muted()),
        ],
        {
            let mut right = Vec::new();
            if focused {
                right.push(Span::styled("↑↓", styles::key_chip()));
                right.push(Span::styled(" select  ", styles::faint()));
                right.push(Span::styled("Enter", styles::key_chip()));
                right.push(Span::styled(" open  ", styles::faint()));
            }
            right.push(Span::styled("Ctrl+O", styles::key_chip()));
            right.push(Span::styled(" hide", styles::faint()));
            right
        },
        w,
        1,
    )];
    let body_h = h.saturating_sub(1);
    let start = if focused {
        app.activity_idx.saturating_sub(body_h.saturating_sub(1))
    } else {
        0
    };
    // Take rows from `start` until the body is full, counting the extra line a workflow phase
    // header costs and reserving one row for the overflow note unless everything fits.
    let mut end = start.min(views.len());
    let mut used = 0usize;
    let mut last_phase: Option<&str> = None;
    while end < views.len() {
        let v = &views[end];
        let cost = 1 + usize::from(needs_phase_header(last_phase, v));
        let reserve = usize::from(end + 1 < views.len());
        if used + cost + reserve > body_h {
            break;
        }
        used += cost;
        last_phase = v.phase.as_deref();
        end += 1;
    }
    let mut last_phase: Option<&str> = None;
    for (i, v) in views.iter().enumerate().skip(start).take(end - start) {
        if needs_phase_header(last_phase, v) {
            lines.push(TextLine::from(Span::styled(
                format!(
                    "   {} {}",
                    glyphs.group,
                    v.phase.as_deref().unwrap_or_default()
                ),
                styles::warn().bold(),
            )));
        }
        last_phase = v.phase.as_deref();
        let selected = focused && i == app.activity_idx;
        let (mark, mark_style) = activity_mark(app, v);
        let caret = if selected { glyphs.prompt } else { " " };
        let running = v.status == ActivityStatus::Running;
        let elapsed = if v.kind == ActivityKind::MainChat {
            None
        } else {
            app.panel_elapsed(
                &format!("agent:{i}:{}", v.title),
                running,
                v.status != ActivityStatus::Running,
            )
        };
        let mut right = Vec::new();
        if let Some(d) = elapsed {
            right.push(Span::styled(fmt_elapsed(d), styles::faint()));
        }
        if v.cost > 0.0 {
            if !right.is_empty() {
                right.push(Span::styled(format!(" {} ", glyphs.sep), styles::faint()));
            }
            right.push(Span::styled(fmt_cost(v.cost), styles::faint()));
        }
        let title_style = if selected {
            styles::fg().bold()
        } else if running {
            styles::fg()
        } else {
            styles::muted()
        };
        let sep = format!(" {} ", glyphs.sep);
        let mut left = vec![
            Span::styled(
                format!("  {caret} "),
                if selected {
                    styles::brand()
                } else {
                    styles::faint()
                },
            ),
            Span::styled(format!("{mark} "), mark_style),
            Span::styled(v.title.clone(), title_style),
        ];
        if v.model.is_some() {
            left.push(Span::styled(
                format!("{sep}{}", model_short(v.model.as_deref())),
                styles::muted(),
            ));
        }
        let detail = activity_detail(app, i, v);
        if !detail.is_empty() {
            left.push(Span::styled(format!("{sep}{detail}"), styles::muted()));
        }
        let row = aligned(left, right, w, 1);
        lines.push(TextLine::from(select(row.spans, selected, w)));
    }
    if end < views.len() {
        lines.push(TextLine::from(Span::styled(
            format!("     {} +{} more", glyphs.ellipsis, views.len() - end),
            styles::faint(),
        )));
    }
    lines
}

/// Context line for the rail: a gauge and the percentage of the window, or just the tokens when
/// the window is unknown.
pub(crate) fn context_spans(app: &App) -> Vec<Span<'static>> {
    let glyphs = g();
    match app.context_limit.filter(|l| *l > 0) {
        Some(limit) => {
            let frac = (app.context_tokens as f64 / limit as f64).clamp(0.0, 1.0);
            vec![
                Span::styled(glyphs.bar(frac, 8), styles::gauge(frac)),
                Span::styled(
                    format!(" {:.0}% of {}", frac * 100.0, human(limit as u64)),
                    styles::muted(),
                ),
            ]
        }
        None if app.context_tokens > 0 => vec![Span::styled(
            format!("{} tokens", human(app.context_tokens)),
            styles::muted(),
        )],
        None => vec![Span::styled("—", styles::faint())],
    }
}
