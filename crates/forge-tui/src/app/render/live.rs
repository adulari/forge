use super::*;

pub fn render_live(frame: &mut Frame, app: &App) {
    // The dedicated workflow view takes over the whole frame while open (auto-opened when a
    // workflow starts; Esc backgrounds it). Same-terminal rendering, like the activity viewer.
    if app.workflow.open {
        crate::workflow_view::render_workflow_view(frame, app);
        return;
    }
    // The in-loop activity viewer (full-screen mode) takes over the whole frame, rendered through
    // the SAME terminal as the chat — no nested alternate screen, so it can't collide with it.
    if let Some(v) = &app.viewer {
        // Headers only: the chrome is rebuilt every frame (so status/cost stay live) while the
        // selected entry's transcript comes from the revision-keyed wrap cache.
        let views = app.activity_view_headers();
        let selected = v.selected.min(views.len().saturating_sub(1));
        let scroll = if v.follow { usize::MAX / 2 } else { v.scroll };
        let a = frame.area();
        let wrapped = app.ensure_viewer_wrapped(&views, selected, a.width.saturating_sub(1));
        // Record the scroll geometry so `viewer_key` can re-enable follow at the tail. `body_h`
        // mirrors `transcript_lines` (2 header + 1 footer rows reserved).
        let body_h = a.height.saturating_sub(3).max(1);
        app.viewer_geom.set(Some((wrapped.rows.len(), body_h)));
        frame.render_widget(
            Paragraph::new(crate::transcript::transcript_lines_from_wrapped(
                &views,
                selected,
                scroll,
                a.height,
                a.width,
                &wrapped.rows,
            )),
            a,
        );
        return;
    }
    app.viewer_geom.set(None);
    app.forget_viewer_wrapped();
    const MIN_STREAM: u16 = 1;
    // The input box grows with wrapped/multiline content (capped); the stream area absorbs the
    // change, so the inline viewport's total height is untouched (never resized at runtime).
    let input_h = input_box_height(&app.input, frame.area().width);
    let permission_h = prompt_height(app);
    let status_h = statusline_height(app);
    // Persistent event-backed heartbeat for the in-flight turn. Reserve it independently from the
    // optional task/activity panels so a plain single-model request is never just "working…".
    let turn_activity_h = u16::from(app.busy);
    // A one-row band between the input and the statusline: shows the animated compaction bar while
    // compacting, otherwise an "approaching auto-compact" hint when the context fills up.
    let band_h = compact_band_height(app);
    let fixed = permission_h + input_h + band_h + status_h + turn_activity_h;
    let avail = frame.area().height.saturating_sub(fixed);
    let panel_avail = avail.saturating_sub(MIN_STREAM);

    // Basic chat shows a one-line work summary. Ctrl+O expands the existing task and activity
    // panels on demand, retaining every detail without making parallel work visually dominant.
    let details_open = app.activity_focused;
    let (activity_h, task_h) = if details_open {
        split_panel_budget(
            app.activity_panel_height(),
            app.tasks_panel_height(),
            panel_avail,
        )
    } else {
        (0, 0)
    };
    let work_summary_h = u16::from(
        !details_open
            && (app.has_activity() || !app.tasks.is_empty() || app.running_assay_critics() > 0),
    );
    // One-line status band while a live workflow's view is backgrounded (Esc'd away): the run's
    // only always-visible trace, since its rows don't join the activity panel.
    let wf_band_h = u16::from(app.workflow.band_visible());
    let stream_h = avail.saturating_sub(activity_h + task_h + wf_band_h + work_summary_h);

    let areas = Layout::vertical([
        Constraint::Length(stream_h),
        Constraint::Length(wf_band_h),
        Constraint::Length(work_summary_h),
        Constraint::Length(activity_h),
        Constraint::Length(task_h),
        Constraint::Length(turn_activity_h),
        Constraint::Length(permission_h),
        Constraint::Length(input_h),
        Constraint::Length(band_h),
        Constraint::Length(status_h),
    ])
    .split(frame.area());

    // areas[0]: the main region. The slash-command palette and @path picker are *completion popups*
    // — in full-screen they show as a small bottom-anchored list with the transcript still visible
    // above (not the whole screen). The session picker stays a full modal. Otherwise areas[0] is the
    // transcript (full-screen) or the in-flight reply edge (inline).
    const POPUP_MAX: u16 = 10;
    if app.command_center.open {
        if app.fullscreen {
            render_transcript_area(frame, areas[0], app);
        } else {
            render_preview(frame, areas[0], app);
        }
    } else if app.palette.open || app.at_picker.open {
        let (top, popup) = if app.fullscreen && areas[0].height > POPUP_MAX + 1 {
            let popup_h = POPUP_MAX;
            let top = Rect {
                height: areas[0].height - popup_h,
                ..areas[0]
            };
            let popup = Rect {
                y: areas[0].y + areas[0].height - popup_h,
                height: popup_h,
                ..areas[0]
            };
            (Some(top), popup)
        } else {
            (None, areas[0])
        };
        if let Some(top) = top {
            render_transcript_area(frame, top, app);
        }
        if app.palette.open {
            render_palette(frame, popup, app);
        } else {
            render_at_path_picker(frame, popup, app);
        }
    } else if app.picker.open {
        render_picker(frame, areas[0], app);
    } else if app.fullscreen {
        render_transcript_area(frame, areas[0], app);
    } else {
        render_preview(frame, areas[0], app);
    }
    // Effort slider overlays the bottom of areas[0] when open (3 rows, anchored at its bottom).
    if app.effort_slider {
        render_effort_slider(frame, areas[0], app);
    }
    if wf_band_h > 0 {
        frame.render_widget(
            Paragraph::new(crate::workflow_view::workflow_band_line(app)),
            areas[1],
        );
    }
    if work_summary_h > 0 {
        render_work_summary(frame, areas[2], app);
    }
    if activity_h > 0 {
        render_activity_panel(frame, areas[3], app);
    }
    if task_h > 0 {
        frame.render_widget(
            Paragraph::new(tasks_panel_lines(&app.tasks, areas[4].height)),
            areas[4],
        );
    }
    if turn_activity_h > 0 {
        render_turn_activity(frame, areas[5], app);
    }
    if permission_h > 0 {
        render_permission(frame, areas[6], app);
    }
    render_input(frame, areas[7], app);
    if band_h > 0 {
        render_compact_band(frame, areas[8], app);
    }
    render_statusline(frame, areas[9], app);
    // Usage overlay renders last so it appears on top of everything.
    render_usage_overlay(frame, app);
    render_mesh_overlay(frame, app);
    render_voice_overlay(frame, app);
    crate::config_editor::render_config_overlay(frame, &app.config_editor);
    render_command_center(frame, frame.area(), app);
}

/// One-row, always-visible heartbeat for a running turn. It reports only facts backed by presenter
/// events: the phase, routed model, total elapsed time, streamed reasoning/answer volume, tool-call
/// count, and the age of the last event. A long provider silence therefore looks materially
/// different from hidden reasoning that is still streaming.
fn render_turn_activity(frame: &mut Frame, area: Rect, app: &App) {
    let spin = SPINNER[app.tick % SPINNER.len()];
    let quiet = app.turn_quiet_secs();
    let waiting_for_user = app.prompt.is_some() || app.awaiting_question();
    let (phase, detail) = if app.prompt.is_some() {
        ("waiting for permission", "your response is required")
    } else if app.awaiting_question() {
        ("waiting for your answer", "your response is required")
    } else {
        (
            app.turn_activity.phase.label(),
            app.turn_activity.detail.as_str(),
        )
    };
    let stale = !waiting_for_user && quiet >= 60;
    let phase_color = if waiting_for_user || stale {
        WARNYEL
    } else {
        ACCENT
    };
    let model = truncate(
        &model_short(
            app.turn_activity
                .model
                .as_deref()
                .or_else(|| app.routing.as_ref().map(|routing| routing.model.as_str())),
        ),
        28,
    );

    let mut spans = vec![
        Span::styled(
            format!(" {spin} {phase}"),
            Style::default().fg(phase_color).bold(),
        ),
        Span::styled("  ·  ", Style::default().fg(SEPCOL)),
        Span::styled(model, Style::default().fg(TEXT).bold()),
    ];

    // Show the strongest proof of otherwise-hidden progress first. These are character counts, not
    // token estimates, so the UI never pretends to know usage before the provider reports it.
    if area.width >= 82 {
        if app.turn_activity.reasoning_chars > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(SEPCOL)));
            spans.push(Span::styled(
                format!(
                    "thought {} chars",
                    human(app.turn_activity.reasoning_chars as u64)
                ),
                Style::default().fg(DIM),
            ));
        }
        if app.turn_activity.response_chars > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(SEPCOL)));
            spans.push(Span::styled(
                format!(
                    "answer {} chars",
                    human(app.turn_activity.response_chars as u64)
                ),
                Style::default().fg(DIM),
            ));
        }
        if app.turn_activity.provider_events > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(SEPCOL)));
            spans.push(Span::styled(
                format!("{} buffered events", app.turn_activity.provider_events),
                Style::default().fg(DIM),
            ));
        }
        if app.turn_activity.tool_calls > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(SEPCOL)));
            spans.push(Span::styled(
                format!("{} tools", app.turn_activity.tool_calls),
                Style::default().fg(TOOLCYAN),
            ));
        }
    }

    spans.push(Span::styled("  ·  ", Style::default().fg(SEPCOL)));
    let heartbeat = if waiting_for_user {
        "paused for input".to_string()
    } else if quiet <= 1 {
        "event now".to_string()
    } else if quiet < 15 {
        format!("event {quiet}s ago")
    } else if quiet < 60 {
        format!("quiet {quiet}s")
    } else {
        format!("⚠ no events for {}", fmt_dur(quiet))
    };
    spans.push(Span::styled(
        heartbeat,
        Style::default().fg(if stale { WARNYEL } else { OKGREEN }),
    ));

    if area.width >= 132 && !detail.is_empty() {
        spans.push(Span::styled("  ·  ", Style::default().fg(SEPCOL)));
        spans.push(Span::styled(truncate(detail, 54), Style::default().fg(DIM)));
    }

    frame.render_widget(Paragraph::new(TextLine::from(spans)), area);
}

fn render_work_summary(frame: &mut Frame, area: Rect, app: &App) {
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
        Style::default().fg(ACCENT).bold(),
    )];
    if open_tasks > 0 {
        parts.push(Span::styled(
            format!(
                "  {open_tasks} task{}",
                if open_tasks == 1 { "" } else { "s" }
            ),
            Style::default().fg(TEXT),
        ));
    }
    if agents > 0 {
        parts.push(Span::styled(
            format!("  {agents} agent{}", if agents == 1 { "" } else { "s" }),
            Style::default().fg(TOOLCYAN),
        ));
    }
    if critics > 0 {
        parts.push(Span::styled(
            format!("  {critics} review{}", if critics == 1 { "" } else { "s" }),
            Style::default().fg(WARNYEL),
        ));
    }
    parts.push(Span::styled("  Ctrl+O details", Style::default().fg(DIM)));
    frame.render_widget(Paragraph::new(TextLine::from(parts)), area);
}

/// Centered, searchable entry point for the complete command and skill catalogue. It is rendered
/// over the chat rather than replacing it, preserving users' place in the current conversation.
fn render_command_center(frame: &mut Frame, area: Rect, app: &App) {
    if !app.command_center.open || area.width < 28 || area.height < 8 {
        return;
    }

    let box_area = surface::modal_area(area, 92, 18);
    let inner = surface::render_panel(
        frame,
        box_area,
        surface::title("Command center", surface::SurfaceTone::Accent),
        None,
        surface::SurfaceTone::Accent,
    );
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let query = if app.command_center.query.is_empty() {
        "Search commands, skills, settings...".to_string()
    } else {
        app.command_center.query.clone()
    };
    let query_style = if app.command_center.query.is_empty() {
        Style::default().fg(DIM)
    } else {
        Style::default().fg(TEXT)
    };
    frame.render_widget(
        Paragraph::new(TextLine::from(vec![
            Span::styled("  > ", Style::default().fg(ORANGE).bold()),
            Span::styled(query, query_style),
        ])),
        rows[0],
    );

    let entries = app.command_center.matches(&app.palette.extra);
    let visible = rows[1].height as usize;
    let start = app
        .command_center
        .selected
        .saturating_sub(visible.saturating_sub(1));
    let mut previous_category: Option<&str> = None;
    let mut lines = Vec::with_capacity(visible);
    for (index, entry) in entries.iter().enumerate().skip(start).take(visible) {
        if previous_category != Some(entry.category) {
            if lines.len() >= visible {
                break;
            }
            lines.push(TextLine::from(Span::styled(
                format!("  {}", entry.category),
                Style::default().fg(DIM).add_modifier(Modifier::BOLD),
            )));
            previous_category = Some(entry.category);
        }
        if lines.len() >= visible {
            break;
        }
        let selected = index == app.command_center.selected;
        let marker = if selected { "› " } else { "  " };
        let command_style = if selected {
            Style::default().fg(Color::White).bg(SELECT_BG).bold()
        } else {
            Style::default().fg(TOOLCYAN)
        };
        let desc_style = if selected {
            Style::default().fg(Color::White).bg(SELECT_BG)
        } else {
            Style::default().fg(DIM)
        };
        lines.push(TextLine::from(vec![
            Span::styled(format!("{marker}/{:<18}", entry.name), command_style),
            Span::styled(entry.desc.clone(), desc_style),
        ]));
    }
    if lines.is_empty() {
        lines.push(TextLine::from(Span::styled(
            "  No commands or skills match.",
            Style::default().fg(DIM),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rows[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "↑/↓ select  Enter open  Esc close",
            Style::default().fg(DIM),
        )),
        rows[2],
    );
}

/// The inline slash-command palette: a scrolling window of filtered commands, selected row
/// highlighted, revealed by an ease-in animation (RFC session-management-and-commands).
fn render_palette(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return; // degenerate viewport (e.g. 0-height terminal) — nothing to draw, never clamp(1,0).
    }
    let matches = app.palette.matches();
    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new(TextLine::from(Span::styled(
                "  no commands match",
                Style::default().fg(DIM),
            ))),
            area,
        );
        return;
    }
    let h = area.height as usize;
    // Ease-in reveal: rows appear over the first few frames after opening.
    let revealed = ((app.palette.anim * h as f32).ceil() as usize).clamp(1, h);
    // Scroll so the selected row stays visible within the window.
    let start = app.palette.selected.saturating_sub(h.saturating_sub(1));
    let lines: Vec<TextLine> = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(revealed)
        .map(|(i, c)| {
            let selected = i == app.palette.selected;
            let marker = if selected { "▸ " } else { "  " };
            let name_style = if selected {
                Style::default().fg(ACCENT).bold()
            } else {
                Style::default().fg(USER)
            };
            let mut spans = vec![
                Span::styled(format!("  {marker}/{}", c.name), name_style),
                Span::styled(format!("  {}", c.desc), Style::default().fg(DIM)),
            ];
            // Inline usage/arg hint on the highlighted row so non-obvious args (`/assay`,
            // `/replay`, `/model`, and fixed-enum args like `/effort [low|…]`) are discoverable.
            if selected && !c.usage.is_empty() {
                spans.push(Span::styled(
                    format!("   {}", c.usage),
                    Style::default().fg(TOOLCYAN),
                ));
                // Best-effort enum-value completion candidates for fixed-arg commands.
                let values = crate::commands::arg_values(&c.name);
                if !values.is_empty() {
                    spans.push(Span::styled(
                        format!("   ⇥ {}", values.join(" · ")),
                        Style::default().fg(DIM),
                    ));
                }
            }
            TextLine::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The `@path` file-path picker: a scrolling, filter-narrowed list of files, revealed by the
/// same ease-in animation as the command palette.
fn render_at_path_picker(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let matches = app.at_picker.matches();
    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new(TextLine::from(Span::styled(
                "  no files match",
                Style::default().fg(DIM),
            ))),
            area,
        );
        return;
    }
    let h = area.height as usize;
    let revealed = ((app.at_picker.anim * h as f32).ceil() as usize).clamp(1, h);
    let start = app.at_picker.selected.saturating_sub(h.saturating_sub(1));
    let lines: Vec<TextLine> = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(revealed)
        .map(|(i, path)| {
            let selected = i == app.at_picker.selected;
            let marker = if selected { "▸ " } else { "  " };
            let style = if selected {
                Style::default().fg(TOOLCYAN).bold()
            } else {
                Style::default().fg(USER)
            };
            TextLine::from(Span::styled(format!("  {marker}@{path}"), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The interactive session/checkpoint picker: a heading + a scrolling, filter-narrowed window of
/// rows, the selected one highlighted, revealed by the same ease-in as the palette. Constrained
/// to the (fixed-height) inline live region, so it scrolls rather than growing.
fn render_picker(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return; // degenerate viewport — never clamp(1, 0).
    }
    let p = &app.picker;
    let matches = p.matches();
    let h = area.height as usize;
    let mut lines: Vec<TextLine> = Vec::with_capacity(h);

    // Heading: title · live filter (or hint) · position.
    let mut head = vec![Span::styled(
        format!("  {} ", p.heading),
        Style::default().fg(ACCENT).bold(),
    )];
    if p.query.is_empty() {
        head.push(Span::styled("(type to filter)", Style::default().fg(DIM)));
    } else {
        head.push(Span::styled(
            format!("/{}", p.query),
            Style::default().fg(USER),
        ));
    }
    if !matches.is_empty() {
        head.push(Span::styled(
            format!("  {}/{}", p.selected + 1, matches.len()),
            Style::default().fg(DIM),
        ));
    }
    lines.push(TextLine::from(head));

    if matches.is_empty() {
        lines.push(TextLine::from(Span::styled(
            "  no matches",
            Style::default().fg(DIM),
        )));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let list_h = h.saturating_sub(1); // rows below the heading
    let revealed = ((p.anim * list_h as f32).ceil() as usize).clamp(1, list_h.max(1));
    let start = p.selected.saturating_sub(list_h.saturating_sub(1));
    let tempers = p.kind == Some(crate::commands::PickerKind::Tempers);
    let models = p.kind == Some(crate::commands::PickerKind::Models);
    let model_pin = p.kind == Some(crate::commands::PickerKind::ModelPin);
    let tick = app.tick;
    for (i, row) in matches.iter().enumerate().skip(start).take(revealed) {
        let selected = i == p.selected;
        let marker = if selected { "▸ " } else { "  " };
        // Color rows by kind: tempers by posture, models browser by category, model-pin picker
        // by tier with the "mesh" row animated (cycles accent colors).
        let base = if tempers {
            temper_color(&row.title)
        } else if models {
            models_row_color(row)
        } else if model_pin {
            model_pin_row_color(row, tick)
        } else {
            USER
        };
        let title_style = if selected {
            Style::default().fg(base).bold()
        } else {
            Style::default().fg(base)
        };
        // ModelPin: add a tier badge between title and subtitle for at-a-glance scanning.
        let subtitle_str = if model_pin {
            truncate(&row.subtitle, 52)
        } else {
            truncate(&row.subtitle, 44)
        };
        lines.push(TextLine::from(vec![
            Span::styled(format!("  {marker}{}", row.title), title_style),
            Span::styled(format!("  {subtitle_str}"), Style::default().fg(DIM)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The in-flight streaming reply's trailing edge, scrolled to its bottom so the freshest
/// text and the `▌` cursor stay visible.
/// Divide the panel budget between the activity panel (`want_a`) and the tasks panel (`want_t`).
/// If both fit, each gets its full desired height. Otherwise split fairly: each keeps up to half,
/// and any slack the smaller panel doesn't use is handed to the larger one.
pub(crate) fn split_panel_budget(want_a: u16, want_t: u16, budget: u16) -> (u16, u16) {
    if want_a + want_t <= budget {
        return (want_a, want_t);
    }
    let half = budget / 2;
    if want_a <= half {
        (want_a, budget.saturating_sub(want_a).min(want_t))
    } else if want_t <= half {
        (budget.saturating_sub(want_t).min(want_a), want_t)
    } else {
        (half, budget.saturating_sub(half).min(want_t))
    }
}
