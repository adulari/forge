use super::*;
use crate::theme::styles;

/// Render the full-screen transcript: the finalized conversation (`main_log`) plus the in-flight
/// reply edge, wrapped to the area width and scrolled to `transcript_scroll` (or the tail while
/// following). This is the full-screen counterpart to the inline scrollback + [`render_preview`].
pub(crate) fn render_transcript_area(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    app.last_width.set(area.width);
    // The panels surface draws the welcome while the transcript is empty.
    if super::welcome::render_empty_state(frame, area, app) {
        app.transcript_geom.set(None);
        return;
    }
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
            TextLine::from(Span::styled("Forge", styles::brand().bold())),
            TextLine::from(Span::styled(
                crate::app::lines::banner::TAGLINE,
                styles::fg(),
            )),
            TextLine::default(),
            TextLine::from(Span::styled("Explain this codebase", styles::muted())),
            TextLine::from(Span::styled("Review the current changes", styles::muted())),
            TextLine::from(Span::styled(
                "Plan a change before editing",
                styles::muted(),
            )),
            TextLine::from(Span::styled("Ctrl+K for all actions", styles::muted())),
        ];
        if show_readiness {
            lines.push(TextLine::from(Span::styled(
                "Check readiness with forge doctor",
                styles::muted(),
            )));
            lines.push(TextLine::from(Span::styled(
                "Preview routing with /mesh",
                styles::muted(),
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
    // Clone only the visible window (~body_h rows), not the whole transcript, each frame. Two
    // things are drawn live over the baked rows: the open turn's rail (hot while it works, then
    // cooling) and the header of a running tool card (spinner, or awaiting approval).
    let hot = app.hot_rail();
    let live = app.running_card_headers(area.width.saturating_sub(1) as usize);
    let mut lines: Vec<TextLine> = Vec::with_capacity(body_h.min(total.saturating_sub(scroll)));
    for i in scroll..(scroll + body_h).min(committed) {
        let mut row = cache.rows[i].clone();
        let src = cache.origins[i];
        let first_of_line = i == 0 || cache.origins[i - 1] != src;
        if let Some((_, header)) = live.iter().find(|(start, _)| *start == src) {
            if first_of_line {
                row = header.clone();
            }
        }
        if let Some((from, style)) = hot {
            if src >= from {
                crate::app::transcript_flow::restyle_rail(&mut row, style);
            }
        }
        lines.push(row);
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
                    cell.set_style(styles::selection());
                }
            }
        }
    }

    // Floating "previous message" bar at the top — only while a user message starts above the
    // view. Pressing/clicking it again walks to the one before.
    if area.height > 1 && app.has_user_message_above() {
        let label = " ↑ Previous message · Ctrl+Home ";
        let w = (label.chars().count() as u16).min(area.width);
        let x = area.x + area.width.saturating_sub(w + 1);
        let y = area.y;
        let bar = Paragraph::new(TextLine::from(Span::styled(
            label,
            styles::pill(styles::user()),
        )));
        frame.render_widget(bar, Rect::new(x, y, w, 1));
        app.prev_bar_geom.set(Some((y, x, w)));
    } else {
        app.prev_bar_geom.set(None);
    }

    // Floating "jump to bottom" bar — only while scrolled up off the tail.
    if scroll < max_scroll && area.height > 0 {
        let label = " ↓ Jump to bottom · Ctrl+End ";
        let w = (label.chars().count() as u16).min(area.width);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + area.height - 1;
        let bar = Paragraph::new(TextLine::from(Span::styled(
            label,
            styles::pill(styles::brand()),
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
