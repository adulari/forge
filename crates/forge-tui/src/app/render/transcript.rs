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
            TextLine::from(Span::styled(
                format!("{} for all actions", app.action_chord("command_center")),
                styles::muted(),
            )),
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
        // A row a folded tool group put here animates (or shows focus) on its own terms; the
        // card it summarises is not on screen to take the live header.
        if let Some(fold) = App::fold_row_at(&cache, i) {
            if let Some(live_row) = app.live_fold_row(fold, area.width) {
                row = live_row;
            }
        } else {
            let first_of_line =
                i == 0 || cache.origins[i - 1] != src || App::fold_row_at(&cache, i - 1).is_some();
            if let Some((_, header)) = live.iter().find(|(start, _)| *start == src) {
                if first_of_line {
                    let in_open_group = crate::app::tool_groups::is_indented(&row);
                    row = header.clone();
                    if in_open_group {
                        row = crate::app::tool_groups::indented_fitted(row);
                    }
                }
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

    // The two navigation pills exist only while the view is scrolled up off the tail: at the
    // bottom there is nowhere to jump to, and a pill there would only cover the newest rows.
    let scrolled_up = scroll < max_scroll;
    // "Previous message" at the top right, while a user message starts above the view; pressing
    // or clicking it again walks to the one before. It quotes the start of that message, the way
    // Claude Code's sticky prompt header does, so you know where it goes before you press it.
    let prev = if scrolled_up && area.height > 2 {
        message_above(app, scroll).map(|preview| nav_pill(frame, area, NavPill::Previous(preview)))
    } else {
        None
    };
    app.prev_bar_geom.set(prev.flatten());
    // "Jump to bottom" centred on the last row.
    let jump = (scrolled_up && area.height > 0).then(|| nav_pill(frame, area, NavPill::Bottom));
    app.jump_bar_geom.set(jump.flatten());
}

/// The user message "Previous message" goes to: `None` when none starts above the row at the
/// top, else the start of its text when it has any to show.
fn message_above(app: &App, scroll: usize) -> Option<Option<String>> {
    let rows = app.user_message_rows();
    let &target = rows.iter().rev().find(|&&r| r < scroll)?;
    let label = *app.wrap_cache.borrow().origins.get(target)?;
    let text: String = app
        .main_log
        .get(label + 1)
        .map(|line| {
            line.spans
                .iter()
                .filter(|s| !crate::copy_mark::is_decor(s.style.add_modifier))
                .map(|s| s.content.as_ref())
                .collect()
        })
        .unwrap_or_default();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    Some((!text.is_empty()).then_some(text))
}

enum NavPill {
    /// With the start of the message it goes to, when there is one.
    Previous(Option<String>),
    Bottom,
}

/// The most cells a quoted message takes in the "previous message" pill.
const PREVIEW_CELLS: usize = 36;

/// Draw one navigation pill and return where it landed `(row, col, width)` for hit-testing. The
/// label names its chord; a terminal too narrow for the full label gets the short one, and one
/// too narrow for that gets none.
fn nav_pill(frame: &mut Frame, area: Rect, pill: NavPill) -> Option<(u16, u16, u16)> {
    let glyphs = crate::theme::glyphs::g();
    let width = area.width as usize;
    let cells = |label: &str| Span::raw(label).width();
    let top = matches!(pill, NavPill::Previous(_));
    let (labels, role) = match pill {
        NavPill::Previous(preview) => {
            let full = format!(" {} Previous message {} Ctrl+Home ", glyphs.up, glyphs.sep);
            let short = format!(" {} Ctrl+Home ", glyphs.up);
            // The quote gets what is left of half the row after the arrow and the chord, so the
            // pill never covers most of the top row.
            let chrome = cells(&short) + 3;
            let room = (width / 2).saturating_sub(chrome).min(PREVIEW_CELLS);
            let quoted = preview.filter(|_| room >= 12).map(|text| {
                let text = crate::app::tool_cards::truncate_cells(&text, room);
                format!(" {} {text} {} Ctrl+Home ", glyphs.up, glyphs.sep)
            });
            (
                quoted.into_iter().chain([full, short]).collect::<Vec<_>>(),
                styles::user(),
            )
        }
        NavPill::Bottom => (
            vec![
                format!(" {} Jump to bottom {} Ctrl+End ", glyphs.down, glyphs.sep),
                format!(" {} Ctrl+End ", glyphs.down),
            ],
            styles::brand(),
        ),
    };
    let label = labels.into_iter().find(|l| cells(l) + 2 <= width)?;
    let w = cells(&label) as u16;
    let (x, y) = if top {
        (area.x + area.width - w - 1, area.y)
    } else {
        (area.x + (area.width - w) / 2, area.y + area.height - 1)
    };
    let bar = Paragraph::new(TextLine::from(Span::styled(label, styles::pill(role))));
    frame.render_widget(bar, Rect::new(x, y, w, 1));
    Some((y, x, w))
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
