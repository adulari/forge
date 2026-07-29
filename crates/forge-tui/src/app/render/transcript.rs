use super::*;

/// Render the full-screen transcript: the finalized conversation (`main_log`) plus the in-flight
/// reply edge, wrapped to the area width and scrolled to `transcript_scroll` (or the tail while
/// following). This is the full-screen counterpart to the inline scrollback + [`render_preview`].
pub(crate) fn render_transcript_area(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    app.last_width.set(area.width);
    let body_h = area.height as usize;
    // Memoized: only re-wraps the bulk log when it changed; the streaming edge is the cheap part.
    app.ensure_wrapped_main(area.width);
    let cache = app.wrap_cache.borrow();
    let committed = cache.rows.len();
    let edge_len = app.streaming_edge_len(area.width);
    let total = committed + edge_len;
    if total == 0 && !app.busy {
        // On a normal terminal, make the two operational escape hatches explicit. On compact
        // terminals the first task suggestions win; the composer still exposes the same commands.
        let show_readiness = area.height >= 20;
        let starter_height = if show_readiness { 9 } else { 7 };
        let empty = Layout::vertical([
            Constraint::Percentage(28),
            Constraint::Length(starter_height),
            Constraint::Min(0),
        ])
        .split(area);
        let mut lines = vec![
            TextLine::from(Span::styled("Forge", Style::default().fg(ORANGE).bold())),
            TextLine::from(Span::styled(TAGLINE, Style::default().fg(TEXT))),
            TextLine::default(),
            TextLine::from(Span::styled(
                "Explain this codebase",
                Style::default().fg(TOOLCYAN),
            )),
            TextLine::from(Span::styled(
                "Review the current changes",
                Style::default().fg(TOOLCYAN),
            )),
            TextLine::from(Span::styled(
                "Plan a change before editing",
                Style::default().fg(TOOLCYAN),
            )),
            TextLine::from(Span::styled(
                "Ctrl+K for all actions",
                Style::default().fg(DIM),
            )),
        ];
        if show_readiness {
            lines.push(TextLine::from(Span::styled(
                "Check readiness with forge doctor",
                Style::default().fg(DIM),
            )));
            lines.push(TextLine::from(Span::styled(
                "Preview routing with /mesh",
                Style::default().fg(DIM),
            )));
        }
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), empty[1]);
        app.transcript_geom.set(None);
        return;
    }
    let max_scroll = total.saturating_sub(body_h);
    let scroll = if app.transcript_follow {
        max_scroll
    } else {
        app.transcript_scroll.min(max_scroll)
    };
    // Clone only the visible window (~body_h rows), not the whole transcript, each frame.
    let mut lines: Vec<TextLine> = Vec::with_capacity(body_h.min(total.saturating_sub(scroll)));
    for i in scroll..(scroll + body_h).min(committed) {
        lines.push(cache.rows[i].clone());
    }
    drop(cache);
    lines.extend(app.streaming_edge(
        area.width,
        scroll.saturating_sub(committed),
        body_h.saturating_sub(lines.len()),
    ));
    frame.render_widget(Paragraph::new(lines), area);

    // Record geometry so mouse events can map a cell → a wrapped-row/col in the log.
    app.transcript_geom.set(Some(TranscriptGeom {
        col0: area.x,
        row0: area.y,
        width: area.width,
        height: area.height,
        scroll,
    }));

    // Paint the selection highlight directly onto the rendered cells (preserves fg colors).
    if let Some((a, b)) = app.selection {
        let ((r0, c0), (r1, c1)) = if a <= b { (a, b) } else { (b, a) };
        let buf = frame.buffer_mut();
        for r in r0..=r1 {
            if r < scroll || r >= scroll + body_h {
                continue;
            }
            let y = area.y + (r - scroll) as u16;
            let start = if r == r0 { c0 } else { 0 };
            let end = if r == r1 { c1 } else { area.width };
            for c in start..end.min(area.width) {
                if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(area.x + c, y)) {
                    cell.set_bg(SELECT_BG);
                }
            }
        }
    }

    // Floating "jump to bottom" bar — only while scrolled up off the tail.
    if scroll < max_scroll && area.height > 0 {
        let label = " ↓ Jump to bottom · Ctrl+End ";
        let w = (label.chars().count() as u16).min(area.width);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + area.height - 1;
        let bar = Paragraph::new(TextLine::from(Span::styled(
            label,
            Style::default().fg(STATUSBG).bg(ORANGE).bold(),
        )));
        frame.render_widget(bar, Rect::new(x, y, w, 1));
        app.jump_bar_geom.set(Some((y, x, w)));
    } else {
        app.jump_bar_geom.set(None);
    }
}

pub(crate) fn render_preview(frame: &mut Frame, area: Rect, app: &App) {
    app.last_width.set(area.width);
    // Only the in-flight reply edge lives here now; the task + subagent panels are their own
    // always-visible regions (see `render_live`), so streaming no longer hides them.
    if app.streaming_active {
        // Reuse the full-screen streaming edge so the inline reply reflows + markdown-highlights
        // exactly like the transcript, instead of a raw newline-collapsed blob.
        let body_h = area.height as usize;
        let edge_len = app.streaming_edge_len(area.width);
        let start = edge_len.saturating_sub(body_h);
        let visible = app.streaming_edge(area.width, start, body_h);
        frame.render_widget(Paragraph::new(visible), area);
    }
}

/// Short model label: strip the `provider::` prefix so the panel shows e.g. `opus` not
/// `anthropic::claude-opus-4-8`-style fully-qualified ids.
pub(crate) fn model_short(model: Option<&str>) -> String {
    match model {
        Some(m) if !m.is_empty() => m.split("::").last().unwrap_or(m).to_string(),
        _ => "…".to_string(),
    }
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
            Style::default().fg(ACCENT).bold(),
        ),
        Span::styled(hint, Style::default().fg(DIM)),
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
                Style::default().fg(WARNYEL).bold(),
            )));
        }
        last_phase = v.phase.as_deref();
        let selected = focused && i == app.activity_idx;
        let marker = if selected { "▸" } else { " " };
        let (kind_glyph, kind_color) = match v.kind {
            ActivityKind::MainChat => ("●", TOOLCYAN),
            ActivityKind::Subagent => ("◈", ACCENT),
            ActivityKind::AssayCritic => ("⚖", WARNYEL),
        };
        let status_span = match v.status {
            ActivityStatus::Running => Span::styled(format!("{spin} "), Style::default().fg(DIM)),
            ActivityStatus::Done => Span::styled("✓ ", Style::default().fg(OKGREEN)),
            ActivityStatus::Failed => Span::styled("✗ ", Style::default().fg(ERRRED)),
            ActivityStatus::Skipped => Span::styled("⏭ ", Style::default().fg(DIM)),
        };
        let title_style = if selected {
            Style::default().fg(ACCENT).bold()
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
                Style::default().fg(if selected { ACCENT } else { DIM }),
            ),
            status_span,
            Span::styled(format!("{} ", v.title), title_style),
            Span::styled(format!("[{model}]  "), Style::default().fg(DIM)),
            Span::styled(
                format!("{}{cost}", truncate(&detail, detail_max)),
                Style::default().fg(DIM),
            ),
        ]));
    }
    if overflow {
        let hidden = views.len() - end;
        lines.push(TextLine::from(Span::styled(
            format!("    … +{hidden} more"),
            Style::default().fg(DIM),
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
    let mut lines = vec![TextLine::from(Span::styled(
        header,
        Style::default().fg(ACCENT).bold(),
    ))];
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
            TodoStatus::Done => ("✔", Style::default().fg(DIM)),
            TodoStatus::InProgress => ("◼", Style::default().fg(ACCENT).bold()),
            TodoStatus::Pending => ("○", Style::default().fg(TEXT)),
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
            Style::default().fg(DIM),
        )));
    }
    lines
}

/// Height reserved for the active response state.  Questions use the composer for their answer,
/// so they need only a single instruction row; permissions keep their decision keys on row two.
pub(crate) fn prompt_height(app: &App) -> u16 {
    if app.prompt.is_none() {
        0
    } else if app.awaiting_question() {
        1
    } else {
        PERMISSION_H
    }
}

pub(crate) fn render_permission(frame: &mut Frame, area: Rect, app: &App) {
    let Some(prompt) = &app.prompt else {
        return;
    };

    // `prompt` is shared by two fundamentally different interactions.  An AskUserQuestion uses
    // the composer (number/free-text + Enter), while a tool permission is a single-key decision.
    // Showing y/a/n for a question was not merely noisy: it suggested keys that could never
    // answer it.  Keep the two contracts visibly distinct.
    if app.awaiting_question() {
        let text_width = area.width.saturating_sub(11) as usize;
        let line = TextLine::from(vec![
            Span::styled(
                " ◉ ANSWER ",
                Style::default().fg(STATUSBG).bg(ORANGE).bold(),
            ),
            Span::styled(truncate(prompt, text_width), Style::default().fg(WARNYEL)),
            Span::styled("  Enter submit", Style::default().fg(DIM)),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..area });
        return;
    }

    let summary_width = area.width.saturating_sub(15) as usize;
    let summary = TextLine::from(vec![
        Span::styled(
            " ◉ PERMISSION ",
            Style::default().fg(STATUSBG).bg(ORANGE).bold(),
        ),
        Span::styled(
            truncate(prompt, summary_width),
            Style::default().fg(WARNYEL),
        ),
    ]);
    frame.render_widget(Paragraph::new(summary), Rect { height: 1, ..area });

    if area.height > 1 {
        let controls = TextLine::from(vec![
            Span::styled("   [y]es", Style::default().fg(OKGREEN).bold()),
            Span::styled("  [a]lways", Style::default().fg(OKGREEN).bold()),
            Span::styled("  [n]o", Style::default().fg(ERRRED).bold()),
            Span::styled("  Esc cancel", Style::default().fg(DIM)),
        ]);
        frame.render_widget(
            Paragraph::new(controls),
            Rect {
                y: area.y + 1,
                height: 1,
                ..area
            },
        );
    }
}
