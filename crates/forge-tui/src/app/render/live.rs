use super::*;

pub fn render_live(frame: &mut Frame, app: &App) {
    surface::take_panels();
    render_live_inner(frame, app);
    app.finish_frame(frame.buffer_mut());
}

fn render_live_inner(frame: &mut Frame, app: &App) {
    // The full-output viewer is opened on request, so it sits above every other takeover.
    if let Some(view) = &app.output_view {
        crate::app::output_view::render_output_view(frame, view);
        return;
    }
    if let Some(view) = app.child_view.as_ref().filter(|_| !prompt_owns_screen(app)) {
        crate::app::child_view::render_child_view(frame, view, app);
        return;
    }
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
    let full = frame.area();
    app.interaction_ui.note_screen(full.width, full.height);
    // Chrome is the header (fullscreen only), the composer and the status line. The composer grows
    // with wrapped/multiline content (capped); the stream area absorbs the change, so the inline
    // viewport's total height is untouched (never resized at runtime).
    let header_h = header::header_height(app);
    let decision_h = permission::block_height(app);
    let composer_h = if permission::replaces_composer(app) {
        0
    } else {
        input_box_height(&app.input, full.width)
    };
    let status_h = statusline_height(app);
    // A one-row band between the composer and the status line: the animated compaction bar while
    // compacting, otherwise an "approaching auto-compact" hint when the context fills up.
    let compact_h = compact_band_height(app);
    let fixed = header_h + decision_h + composer_h + compact_h + status_h;
    let avail = full.height.saturating_sub(fixed);
    // One-line status band while a live workflow's view is backgrounded (Esc'd away): the run's
    // only always-visible trace, since its rows don't join the activity panel.
    let wf_band_h = u16::from(app.workflow.band_visible());
    let band_h = panels::band_height(
        app,
        full.width,
        avail.saturating_sub(MIN_STREAM + wf_band_h),
    );
    // The queued follow-ups sit directly above whatever takes the user's input (composer or
    // decision block), so a message that left the composer is still in view.
    let queue_h = queue_band::height(
        app,
        full.width,
        avail.saturating_sub(MIN_STREAM + wf_band_h + band_h),
    );
    let update_h = update_band::height(
        app,
        full.width,
        avail.saturating_sub(MIN_STREAM + wf_band_h + band_h + queue_h),
    );
    let stream_h = avail.saturating_sub(band_h + wf_band_h + queue_h + update_h);

    let areas = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Length(stream_h),
        Constraint::Length(wf_band_h),
        Constraint::Length(band_h),
        Constraint::Length(queue_h),
        Constraint::Length(update_h),
        Constraint::Length(decision_h),
        Constraint::Length(composer_h),
        Constraint::Length(compact_h),
        Constraint::Length(status_h),
    ])
    .split(full);
    let (
        header_area,
        main_area,
        wf_area,
        band_area,
        queue_area,
        update_area,
        decision_area,
        composer_area,
        compact_area,
        status_area,
    ) = (
        areas[0], areas[1], areas[2], areas[3], areas[4], areas[5], areas[6], areas[7], areas[8],
        areas[9],
    );

    if header_h > 0 {
        header::render_header(frame, header_area, app);
    }

    // An optional right rail (wide terminals) takes its columns from the main region.
    let rail_w = panels::rail_width(app, main_area);
    let main = if rail_w > 0 && rail_w < main_area.width {
        let cols =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(rail_w)]).split(main_area);
        panels::render_rail(frame, cols[1], app);
        cols[0]
    } else {
        main_area
    };

    // The main region. The palette and @path picker are completion popups attached to the
    // composer: the transcript keeps the rows above them. Pickers are modals over the dimmed
    // transcript. A question never lives here: it replaces the composer (see `permission`).
    if app.command_center.open {
        if app.fullscreen {
            render_transcript_area(frame, main, app);
        } else {
            render_preview(frame, main, app);
        }
    } else if app.palette.open || app.at_picker.open {
        let popup_h = popups::attached_height(app, main);
        let top = Rect {
            height: main.height - popup_h,
            ..main
        };
        let popup = Rect {
            y: main.y + main.height - popup_h,
            height: popup_h,
            ..main
        };
        if top.height > 0 {
            if app.fullscreen {
                render_transcript_area(frame, top, app);
            } else {
                render_preview(frame, top, app);
            }
        }
        if app.palette.open {
            popups::render_palette(frame, popup, app);
        } else {
            popups::render_at_path_picker(frame, popup, app);
        }
    } else if app.picker.open {
        if app.fullscreen {
            render_transcript_area(frame, main, app);
        } else {
            render_preview(frame, main, app);
        }
        surface::dim_backdrop(frame, main);
        // The delete confirmation takes the picker's place so two frames never overlap.
        if app.session_delete.is_none() {
            popups::render_picker(frame, main, app);
        }
    } else if app.fullscreen {
        render_transcript_area(frame, main, app);
    } else {
        render_preview(frame, main, app);
    }
    // Effort slider overlays the bottom of the main region when open (3 rows, anchored at its bottom).
    if app.effort_slider {
        render_effort_slider(frame, main, app);
    }
    if wf_band_h > 0 {
        frame.render_widget(
            Paragraph::new(crate::workflow_view::workflow_band_line(app, wf_area.width)),
            wf_area,
        );
    }
    if band_h > 0 {
        panels::render_band(frame, band_area, app);
    }
    if queue_h > 0 {
        queue_band::render(frame, queue_area, app);
    }
    if update_h > 0 {
        update_band::render(frame, update_area, app);
    }
    if decision_h > 0 {
        permission::render_permission(frame, decision_area, app);
    }
    if composer_h > 0 {
        render_input(frame, composer_area, app);
    }
    if compact_h > 0 {
        render_compact_band(frame, compact_area, app);
    }
    render_statusline(frame, status_area, app);
    // Overlays render last so they appear on top of everything.
    render_usage_overlay(frame, app);
    render_mesh_overlay(frame, app);
    render_btw_overlay(frame, app);
    render_session_delete_overlay(frame, app);
    render_voice_overlay(frame, app);
    crate::config_editor::render_config_overlay(frame, &app.config_editor);
    // A modal never covers the header or the composer: it gets the rows between them.
    let modal_region = Rect {
        y: main_area.y,
        height: if composer_h > 0 {
            composer_area.y.saturating_sub(main_area.y)
        } else {
            full.height.saturating_sub(status_h + main_area.y - full.y)
        },
        ..full
    };
    popups::render_command_center(frame, modal_region, app);
    crate::anywhere_menu::render_anywhere_menu(frame, app, modal_region);
    crate::import_menu::render_import_menu(frame, app, modal_region);
    // The inspector sits between the header and the composer; inline, where the live region is
    // short, it takes the composer's rows too (the composer has nothing to say while it is open).
    let status_modal = if app.fullscreen {
        Rect {
            y: main_area.y,
            height: composer_area.y.saturating_sub(main_area.y),
            ..full
        }
    } else {
        Rect {
            height: full.height.saturating_sub(status_h),
            ..full
        }
    };
    status_panel::render_status_panel(frame, status_modal, app);
}

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

/// A permission prompt or question needs the chat's decision block, so the subagent view steps
/// aside until it is answered.
pub(crate) fn prompt_owns_screen(app: &App) -> bool {
    app.prompt.is_some() || app.awaiting_question()
}
