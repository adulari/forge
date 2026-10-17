//! The `/status` inspector: every fact the header, composer and status line can drop at narrow
//! widths, in one place. Opened by `/status`, closed with Esc.

use super::status_line::dropped_widgets;
use super::status_widgets::{fmt_cost, mesh_effort_chip, model_effort_marker};
use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

const LABEL_W: usize = 9;

type Row = (&'static str, Vec<Span<'static>>);

fn text(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), styles::fg())
}

fn muted(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), styles::muted())
}

/// A row's value is a list of parts; the layout joins them with ` · ` and wraps between parts.
fn joined(parts: Vec<Span<'static>>) -> Vec<Span<'static>> {
    parts
}

fn home_relative(path: &std::path::Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && shown.starts_with(&home) => {
            format!("~{}", &shown[home.len()..])
        }
        _ => shown,
    }
}

fn model_row(app: &App) -> Row {
    let mut parts = Vec::new();
    match &app.routing {
        Some(routing) => {
            parts.push(text(routing.model.clone()));
            parts.push(muted(format!("{} tier", routing.tier)));
        }
        None => parts.push(Span::styled("no model yet", styles::faint())),
    }
    let (mesh, _) = mesh_effort_chip(app.effort, 0);
    parts.push(muted(mesh));
    let rung = app.routing.as_ref().and_then(|r| {
        model_effort_marker(&r.model, app.routed_effort.or(app.effort))
            .map(|(marker, _)| marker.trim_matches(['⟨', '⟩']).to_string())
    });
    parts.push(muted(format!(
        "effort {}",
        rung.as_deref().unwrap_or("auto")
    )));
    ("MODEL", joined(parts))
}

fn mode_row(app: &App) -> Row {
    let mut parts = Vec::new();
    if app.temper.is_empty() {
        parts.push(muted("not set"));
    } else {
        parts.push(Span::styled(
            format!("{} (Shift+Tab)", app.temper),
            styles::temper(&app.temper),
        ));
    }
    parts.push(muted(if app.show_thinking {
        "thinking shown"
    } else {
        "thinking hidden"
    }));
    if let Some(goal) = &app.goal_chip {
        parts.push(muted(format!("goal {goal}")));
    }
    if let Some(l) = &app.loop_chip {
        parts.push(muted(format!("loop {l}")));
    }
    if !app.queued.is_empty() {
        parts.push(Span::styled(
            format!("{} queued", app.queued.len()),
            styles::warn(),
        ));
    }
    ("MODE", joined(parts))
}

fn where_row(app: &App) -> Row {
    let mut parts = Vec::new();
    if let Some(dir) = &app.git_workspace {
        parts.push(text(super::header::middle_truncate(
            &home_relative(dir),
            48,
        )));
    } else if let Some(repo) = &app.repo_name {
        parts.push(text(repo.clone()));
    }
    match &app.git_location {
        Some(location) => {
            match &location.head {
                crate::git_location::GitHead::Branch(name) => parts.push(muted(name.clone())),
                crate::git_location::GitHead::Detached(_) => {
                    parts.push(Span::styled(location.branch_label(), styles::warn()))
                }
            }
            parts.push(muted(if location.worktree {
                "worktree"
            } else {
                "not a worktree"
            }));
        }
        None => parts.push(muted("not a git checkout")),
    }
    parts.push(muted(if app.remote_active {
        "remote control on"
    } else {
        "remote control off"
    }));
    ("WHERE", joined(parts))
}

fn context_row(app: &App) -> Row {
    let used = app.context_tokens;
    match app.context_limit.filter(|l| *l > 0) {
        Some(limit) => {
            let frac = (used as f64 / f64::from(limit)).clamp(0.0, 1.0);
            let style = styles::gauge(frac);
            let bar = g().bar(frac, 10);
            let bar = if g().unicode { bar } else { format!("[{bar}]") };
            (
                "CONTEXT",
                joined(vec![
                    Span::styled(
                        format!(
                            "{bar} {} / {} ({:.0}%)",
                            human(used),
                            human(u64::from(limit)),
                            frac * 100.0
                        ),
                        style,
                    ),
                    muted("auto-compact at 80%"),
                ]),
            )
        }
        None if used > 0 => (
            "CONTEXT",
            joined(vec![
                text(format!("{} tokens", human(used))),
                muted("window unknown"),
            ]),
        ),
        None => (
            "CONTEXT",
            vec![Span::styled("nothing sent yet", styles::faint())],
        ),
    }
}

fn turn_row(app: &App) -> Row {
    let glyphs = g();
    if !app.turn_ran {
        return ("TURN", vec![Span::styled("no turn yet", styles::faint())]);
    }
    let mut parts = vec![text(fmt_dur(app.turn_elapsed_secs))];
    if app.turn_in > 0 || app.turn_out > 0 {
        let mut tokens = format!("{}{}", glyphs.tok_up, human(app.turn_in));
        if let Some(cached) = app.turn_cached_in.filter(|c| *c > 0) {
            tokens.push_str(&format!(" ({} cached)", human(cached)));
        }
        tokens.push_str(&format!(" {}{}", glyphs.tok_down, human(app.turn_out)));
        parts.push(muted(tokens));
    }
    if let Some(rate) = app.throughput.display(app.busy, std::time::Instant::now()) {
        parts.push(muted(rate.label_with_trend()));
    }
    ("TURN", joined(parts))
}

/// What the running turn is doing, with the counters the composer's one live line leaves out:
/// phase, the model at the provider-call boundary, hidden-reasoning and answer volume, buffered
/// provider events, tool calls and the age of the last event.
fn activity_row(app: &App) -> Option<Row> {
    if !app.busy {
        return None;
    }
    let a = &app.turn_activity;
    let mut parts = vec![text(a.phase.label())];
    let model = a
        .model
        .as_deref()
        .or_else(|| app.routing.as_ref().map(|r| r.model.as_str()));
    if let Some(model) = model {
        parts.push(muted(model_short(Some(model))));
    }
    if a.reasoning_chars > 0 {
        parts.push(muted(format!(
            "thought {} chars",
            human(a.reasoning_chars as u64)
        )));
    }
    if a.response_chars > 0 {
        parts.push(muted(format!(
            "answer {} chars",
            human(a.response_chars as u64)
        )));
    }
    if let Some((name, chars)) = &a.composing_tool {
        parts.push(muted(format!(
            "writing {name} call, {} chars",
            human(*chars as u64)
        )));
    } else if a.thinking_tokens > 0 {
        parts.push(muted(format!(
            "thinking ~{} tokens",
            human(a.thinking_tokens)
        )));
    } else if a.provider_events > 0 {
        parts.push(muted(format!("{} buffered events", a.provider_events)));
    }
    if a.tool_calls > 0 {
        parts.push(muted(format!("{} tools", a.tool_calls)));
    }
    if !a.detail.is_empty() && matches!(a.phase, TurnPhase::Auxiliary) {
        parts.push(muted(a.detail.clone()));
    }
    let quiet = app.turn_quiet_secs();
    parts.push(if quiet <= 1 {
        muted("event now")
    } else {
        let style = if quiet >= 60 {
            styles::warn()
        } else {
            styles::muted()
        };
        Span::styled(format!("last event {} ago", fmt_dur(quiet)), style)
    });
    Some(("ACTIVITY", joined(parts)))
}

fn session_row(app: &App) -> Row {
    let glyphs = g();
    let id = if app.session_id.is_empty() {
        "new session".to_string()
    } else {
        app.session_id.chars().take(8).collect()
    };
    let model_id = app.routing.as_ref().map(|r| r.model.as_str()).unwrap_or("");
    let cost = {
        let cell = cost_cell(model_id, app.cost_usd);
        if cell.starts_with('$') {
            fmt_cost(app.cost_usd)
        } else {
            cell
        }
    };
    let mut parts = vec![
        text(id),
        muted(format!(
            "{} turn{}",
            app.turns_finished,
            if app.turns_finished == 1 { "" } else { "s" }
        )),
        muted(cost),
    ];
    if app.session_in > 0 || app.session_out > 0 {
        let mut tokens = format!("{}{}", glyphs.tok_up, human(app.session_in));
        if let Some(cached) = app.session_cached_in.filter(|c| *c > 0) {
            tokens.push_str(&format!(" ({} cached)", human(cached)));
        }
        tokens.push_str(&format!(" {}{}", glyphs.tok_down, human(app.session_out)));
        parts.push(muted(tokens));
    }
    ("SESSION", joined(parts))
}

fn quotas_row(app: &App) -> Row {
    let o = &app.usage_overlay;
    let mut parts = Vec::new();
    let mut add = |label: String, pct: f64| {
        let style = if pct >= 90.0 {
            styles::err()
        } else if pct >= 70.0 {
            styles::warn()
        } else {
            styles::muted()
        };
        parts.push(Span::styled(format!("{label} {pct:.0}%"), style));
    };
    if let Some(p) = o.claude_5h_pct {
        add("claude 5h".into(), p);
    }
    if let Some(p) = o.codex_5h_pct {
        add("codex 5h".into(), p);
    }
    if let Some((window, p)) = o.opencode_go_tightest_window() {
        add(format!("go {window}"), p);
    }
    if let Some(p) = &app.quota_pace {
        let window = match p.window.as_str() {
            "five_hour" => "5h",
            "weekly" => "wk",
            "monthly" => "mo",
            other => other,
        };
        let text = match p.projected_fraction_at_reset {
            Some(proj) => format!(
                "{} {window} projected {:.0}% at reset",
                p.provider.trim_end_matches("-cli"),
                proj * 100.0
            ),
            None => format!(
                "{} {window} {:+.1}%/hr",
                p.provider.trim_end_matches("-cli"),
                p.rate_per_hour * 100.0
            ),
        };
        parts.push(muted(text));
    }
    if parts.is_empty() {
        return (
            "QUOTAS",
            vec![Span::styled("no quota data yet", styles::faint())],
        );
    }
    ("QUOTAS", joined(parts))
}

fn mcp_row(app: &App) -> Row {
    let value = if app.mcp_count == 0 {
        Span::styled("none connected", styles::faint())
    } else {
        muted(format!("{} connected", app.mcp_count))
    };
    ("MCP", vec![value])
}

/// Every row of the inspector, in reading order.
pub(crate) fn rows(app: &App) -> Vec<Row> {
    vec![
        model_row(app),
        mode_row(app),
        where_row(app),
        context_row(app),
        turn_row(app),
    ]
    .into_iter()
    .chain(activity_row(app))
    .chain([
        session_row(app),
        quotas_row(app),
        mcp_row(app),
        (
            "VERSION",
            vec![muted(format!("forge v{}", env!("CARGO_PKG_VERSION")))],
        ),
    ])
    .collect()
}

/// Lay the rows out for `value_w` cells of value: label column, parts joined by ` · `, wrapping
/// between parts with the continuation lines aligned under the first.
fn layout_rows(rows: Vec<Row>, value_w: usize) -> Vec<TextLine<'static>> {
    use unicode_width::UnicodeWidthStr;
    let sep = format!(" {} ", g().sep);
    let mut lines = Vec::new();
    for (label, parts) in rows {
        let mut current: Vec<Span<'static>> = Vec::new();
        let mut used = 0usize;
        let mut first = true;
        let mut flush = |current: &mut Vec<Span<'static>>, first: &mut bool| {
            let mut spans = vec![
                Span::raw("  "),
                Span::styled(
                    if *first {
                        format!("{label:<LABEL_W$}")
                    } else {
                        " ".repeat(LABEL_W)
                    },
                    styles::faint().bold(),
                ),
            ];
            spans.append(current);
            lines.push(TextLine::from(spans));
            *first = false;
        };
        for part in parts {
            let part_w = part.content.width();
            let sep_w = if current.is_empty() { 0 } else { sep.width() };
            if !current.is_empty() && used + sep_w + part_w > value_w {
                flush(&mut current, &mut first);
                used = 0;
            }
            if !current.is_empty() {
                current.push(Span::styled(sep.clone(), styles::faint()));
                used += sep.width();
            }
            used += part_w;
            current.push(part);
        }
        flush(&mut current, &mut first);
    }
    lines
}

pub(crate) fn render_status_panel(frame: &mut Frame, area: Rect, app: &App) {
    if !app.status_open || area.width < 30 || area.height < 5 {
        return;
    }
    let width = area.width.saturating_sub(8).clamp(30.min(area.width), 108);
    let value_w = (width as usize).saturating_sub(2 + 2 + LABEL_W + 2);
    let mut body = layout_rows(rows(app), value_w);
    let dropped = dropped_widgets(app, frame.area().width);
    let note = (dropped > 0).then(|| {
        TextLine::from(Span::styled(
            format!(
                "  The status line dropped {dropped} widget{} at this width; they are all above.",
                if dropped == 1 { "" } else { "s" }
            ),
            styles::faint(),
        ))
    });
    // Breathing room when there is space for it; a compact panel when the region is tight (the
    // inline live region), so no row is ever cut off.
    let roomy = body.len() + 2 + 2 + 2 * usize::from(note.is_some()) + 2 <= area.height as usize;
    if roomy {
        body.insert(0, TextLine::default());
        body.push(TextLine::default());
        if let Some(note) = note {
            body.push(note);
            body.push(TextLine::default());
        }
    } else if let Some(note) = note {
        if body.len() + 2 < area.height as usize {
            body.push(note);
        }
    }
    let height = ((body.len() + 2) as u16).min(area.height);
    let box_area = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    if app.fullscreen {
        surface::dim_backdrop(frame, area);
    } else {
        // Inline, the panel owns the whole live region above the status line, composer included.
        surface::render_backdrop(frame, area);
    }
    let inner = surface::render_panel_with_context(
        frame,
        box_area,
        surface::title("Status", surface::SurfaceTone::Accent),
        Some(surface::hint("/status")),
        Some(surface::hint("Esc close")),
        surface::SurfaceTone::Accent,
    );
    frame.render_widget(Paragraph::new(body), inner);
}
