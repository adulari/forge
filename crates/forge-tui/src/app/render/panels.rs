use super::live::split_panel_budget;
use super::*;
use crate::theme::{color, styles};

pub(crate) fn render_work_summary(frame: &mut Frame, area: Rect, app: &App) {
    let open_tasks = app
        .tasks
        .iter()
        .filter(|task| task.status != forge_types::TodoStatus::Done)
        .count();
    let agents = app.running_subagents();
    let critics = app.running_assay_critics();
    let mut parts = vec![Span::styled(
        if app.busy {
            format!(" {}", app.turn_activity.phase.label())
        } else {
            " Work summary".to_string()
        },
        styles::user().bold(),
    )];
    if open_tasks > 0 {
        parts.push(Span::styled(
            format!(
                "  {open_tasks} task{}",
                if open_tasks == 1 { "" } else { "s" }
            ),
            styles::fg(),
        ));
    }
    if agents > 0 {
        parts.push(Span::styled(
            format!("  {agents} agent{}", if agents == 1 { "" } else { "s" }),
            styles::muted(),
        ));
    }
    if critics > 0 {
        parts.push(Span::styled(
            format!("  {critics} review{}", if critics == 1 { "" } else { "s" }),
            styles::warn(),
        ));
    }
    parts.push(Span::styled("  Ctrl+O details", styles::muted()));
    frame.render_widget(Paragraph::new(TextLine::from(parts)), area);
}

pub(crate) fn render_activity_panel(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    // Cheap per-frame metadata only — building full transcripts here would clone the whole main
    // log + re-render markdown every frame (jank/ghosting). Full views are built lazily on Enter.
    let views = app.activity_summaries();
    if views.is_empty() {
        return;
    }
    let h = area.height as usize;
    let w = area.width as usize;
    let spin = SPINNER[app.tick % SPINNER.len()];
    let focused = app.activity_focused;

    let mut lines: Vec<TextLine> = Vec::with_capacity(h);
    let hint = if focused {
        "↑↓ select · ⏎ open · esc"
    } else {
        "^O focus"
    };
    lines.push(TextLine::from(vec![
        Span::styled(
            format!("  ◈ activity ({})  ", views.len()),
            styles::user().bold(),
        ),
        Span::styled(hint, styles::muted()),
    ]));

    let body_h = h.saturating_sub(1);
    // Scroll so the selected row stays visible when the list overflows the panel.
    let start = if focused {
        app.activity_idx.saturating_sub(body_h.saturating_sub(1))
    } else {
        0
    };

    // Greedily take rows starting at `start` until `body_h` lines are used, accounting for the
    // extra line a `phase()` transition costs (NOT just a plain view count — a phase header is an
    // additional line the naive "1 view = 1 line" budget doesn't know about). Reserves 1 line for
    // a "+N more" hint unless the view being considered is the very last one overall (then
    // nothing will be hidden, so no reservation is needed).
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
    let overflow = end < views.len();

    // Workflow-script `phase()` groups (docs/rfcs/forge-workflow.md): a header line is inserted
    // whenever the phase changes from the previous VISIBLE row — purely additive to `lines`, never
    // counted as its own `i`, so `activity_idx`/Ctrl+O/Enter-to-zoom indexing is untouched. `None`
    // phases (every row in a plain `spawn_agents` batch) never trigger a header, so this is a
    // no-op for anything that isn't a workflow script.
    let mut last_phase: Option<&str> = None;
    for (i, v) in views.iter().enumerate().skip(start).take(end - start) {
        if needs_phase_header(last_phase, v) {
            lines.push(TextLine::from(Span::styled(
                format!("  ▶ {}", v.phase.as_deref().unwrap_or_default()),
                styles::warn().bold(),
            )));
        }
        last_phase = v.phase.as_deref();
        let selected = focused && i == app.activity_idx;
        let marker = if selected { "▸" } else { " " };
        let (kind_glyph, kind_color) = match v.kind {
            ActivityKind::MainChat => ("●", color::fg_muted()),
            ActivityKind::Subagent => ("◈", color::user()),
            ActivityKind::AssayCritic => ("⚖", color::warn()),
        };
        let status_span = match v.status {
            ActivityStatus::Running => Span::styled(format!("{spin} "), styles::muted()),
            ActivityStatus::Done => Span::styled("✓ ", styles::ok()),
            ActivityStatus::Failed => Span::styled("✗ ", styles::err()),
            ActivityStatus::Skipped => Span::styled("⏭ ", styles::muted()),
        };
        let title_style = if selected {
            styles::user().bold()
        } else {
            Style::default().fg(kind_color).bold()
        };
        let model = model_short(v.model.as_deref());
        // Trailing detail: line count for chats, the subtitle (findings/focus) for critics.
        let detail = match v.kind {
            ActivityKind::AssayCritic => v.subtitle.clone(),
            _ => format!("{} ln", v.line_count),
        };
        let cost = if v.cost > 0.0 {
            format!("  ${:.4}", v.cost)
        } else {
            String::new()
        };
        let head = format!("  {marker} {kind_glyph} ");
        let used = head.chars().count() + v.title.chars().count() + model.len() + 8;
        let detail_max = w.saturating_sub(used).max(8);
        lines.push(TextLine::from(vec![
            Span::styled(
                head,
                Style::default().fg(if selected {
                    color::user()
                } else {
                    color::fg_muted()
                }),
            ),
            status_span,
            Span::styled(format!("{} ", v.title), title_style),
            Span::styled(format!("[{model}]  "), styles::muted()),
            Span::styled(
                format!("{}{cost}", truncate(&detail, detail_max)),
                styles::muted(),
            ),
        ]));
    }
    if overflow {
        let hidden = views.len() - end;
        lines.push(TextLine::from(Span::styled(
            format!("    … +{hidden} more"),
            styles::muted(),
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The sticky tasks panel (Task list always-visible): a header with the done/total count, then
/// the items with their status glyph, sized to the fixed live region. When the list is longer
/// than the region, the in-progress item is prioritized and the overflow is summarized.
pub(crate) fn tasks_panel_lines(
    tasks: &[forge_types::TodoItem],
    height: u16,
) -> Vec<TextLine<'static>> {
    use forge_types::TodoStatus;
    let h = height as usize;
    let total = tasks.len();
    let done_count = tasks
        .iter()
        .filter(|t| t.status == TodoStatus::Done)
        .count();
    let in_progress_count = tasks
        .iter()
        .filter(|t| t.status == TodoStatus::InProgress)
        .count();
    let open_count = tasks
        .iter()
        .filter(|t| t.status == TodoStatus::Pending)
        .count();
    let header = format!(
        "  ◈ {total} tasks ({done_count} done, {in_progress_count} in progress, {open_count} open)"
    );
    let mut lines = vec![TextLine::from(Span::styled(header, styles::user().bold()))];
    let body_h = h.saturating_sub(1);
    // Prioritize: in-progress first, then pending, then done.
    let mut idxs: Vec<usize> = (0..total).collect();
    idxs.sort_by_key(|&i| match tasks[i].status {
        TodoStatus::InProgress => 0,
        TodoStatus::Pending => 1,
        TodoStatus::Done => 2,
    });
    // Count non-done (always show) vs done (may be truncated).
    let non_done: Vec<usize> = idxs
        .iter()
        .copied()
        .filter(|&i| tasks[i].status != TodoStatus::Done)
        .collect();
    let done_idxs: Vec<usize> = idxs
        .iter()
        .copied()
        .filter(|&i| tasks[i].status == TodoStatus::Done)
        .collect();
    // Always show all non-done; fill remaining rows with done items.
    let rows_for_done = body_h
        .saturating_sub(non_done.len())
        .saturating_sub(usize::from(!done_idxs.is_empty()));
    let show_done = rows_for_done.min(done_idxs.len());
    let overflow_done = done_idxs.len().saturating_sub(show_done);
    let shown_idxs: Vec<usize> = non_done
        .iter()
        .chain(done_idxs.iter().take(show_done))
        .copied()
        .collect();
    for &i in &shown_idxs {
        let t = &tasks[i];
        let (glyph, style) = match t.status {
            TodoStatus::Done => ("✔", styles::muted()),
            TodoStatus::InProgress => ("◼", styles::user().bold()),
            TodoStatus::Pending => ("○", styles::fg()),
        };
        // A delegated task names its owner inline — the panel is one line per task, so an
        // `@assignee` suffix is the only place it fits. Budgeted out of the same 62 columns the
        // title had, so an owner can never push the row past the panel width.
        let label = match t
            .assignee
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            Some(who) => format!(
                "{} @{}",
                truncate(&t.title, 62usize.saturating_sub(who.chars().count() + 2)),
                truncate(who, 24)
            ),
            None => truncate(&t.title, 62),
        };
        lines.push(TextLine::from(Span::styled(
            format!("  {glyph} {label}"),
            style,
        )));
    }
    if overflow_done > 0 {
        lines.push(TextLine::from(Span::styled(
            format!("   … +{overflow_done} completed"),
            styles::muted(),
        )));
    }
    lines
}

/// The unified sticky activity panel: lists the main chat plus every subagent and assay critic in
/// one navigable list. When focused (Ctrl+O) the selected row is highlighted and ↑↓ move it; Enter
/// opens that entry's full-screen transcript. Themed per kind: ● main chat, ⚒ subagent, ⚖ critic.
/// Whether rendering `v` (given the previous shown row's phase was `prev`) needs a phase-header
/// line first — a workflow-script `phase()` transition (docs/rfcs/forge-workflow.md). Never true
/// for `None` phases, so a plain `spawn_agents` batch (every row's phase is `None`) never groups.
pub(crate) fn needs_phase_header(prev: Option<&str>, v: &ActivitySummary) -> bool {
    matches!(v.phase.as_deref(), Some(p) if Some(p) != prev)
}

/// Height of the band between the transcript and the decision block: the one-line work summary,
/// or the expanded task and activity panels while details are open. `avail` is the room the
/// layout can spare.
pub(crate) fn band_height(app: &App, avail: u16) -> u16 {
    if app.activity_focused {
        let (activity_h, task_h) =
            split_panel_budget(app.activity_panel_height(), app.tasks_panel_height(), avail);
        activity_h + task_h
    } else {
        u16::from(app.has_activity() || !app.tasks.is_empty() || app.running_assay_critics() > 0)
            .min(avail)
    }
}

/// Draw the band into `area` (whose height is what [`band_height`] returned).
pub(crate) fn render_band(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    if !app.activity_focused {
        render_work_summary(frame, area, app);
        return;
    }
    let (activity_h, task_h) = split_panel_budget(
        app.activity_panel_height(),
        app.tasks_panel_height(),
        area.height,
    );
    let rows =
        Layout::vertical([Constraint::Length(activity_h), Constraint::Length(task_h)]).split(area);
    if activity_h > 0 {
        render_activity_panel(frame, rows[0], app);
    }
    if task_h > 0 {
        frame.render_widget(
            Paragraph::new(tasks_panel_lines(&app.tasks, rows[1].height)),
            rows[1],
        );
    }
}

/// Width of the details rail on wide terminals. Zero until the panels surface draws one.
pub(crate) fn rail_width(_app: &App, _area: Rect) -> u16 {
    0
}

/// Draw the details rail into `area` (never called while [`rail_width`] is zero).
pub(crate) fn render_rail(_frame: &mut Frame, _area: Rect, _app: &App) {}

/// Composer hints while a panel owns the keyboard. `None` leaves the composer's own hints.
pub(crate) fn hints(_app: &App) -> Option<Vec<super::chrome::Hint>> {
    None
}
