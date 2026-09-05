use super::*;

/// Render one statusline widget into a span list, or `None` if the widget has no data to show.
fn render_statusline_widget<'a>(
    widget: &forge_config::StatuslineWidget,
    app: &App,
    w: u16,
) -> Option<Vec<Span<'a>>> {
    use forge_config::StatuslineWidget as W;
    match widget {
        W::Model => {
            let model = app
                .routing
                .as_ref()
                .map(|r| r.model.as_str())
                .unwrap_or("—");
            let tier = app.routing.as_ref().map(|r| r.tier.as_str());
            let mut spans: Vec<Span> = Vec::new();
            if app.model_search.is_some() && w >= 40 {
                let f = SPINNER[app.tick % SPINNER.len()];
                let label = match &app.model_search {
                    Some((_, true)) => "retrying model",
                    _ => "finding a model",
                };
                spans.push(Span::styled(
                    format!("{f} {label}"),
                    Style::default().fg(WARNYEL).bg(STATUSBG),
                ));
                spans.push(Span::styled(
                    "  │  ",
                    Style::default().fg(SEPCOL).bg(STATUSBG),
                ));
            } else if app.busy && w >= 40 {
                let f = SPINNER[app.tick % SPINNER.len()];
                spans.push(Span::styled(
                    format!("{f} {}", app.turn_activity.phase.label()),
                    Style::default().fg(ACCENT).bold().bg(STATUSBG),
                ));
                spans.push(Span::styled(
                    "  │  ",
                    Style::default().fg(SEPCOL).bg(STATUSBG),
                ));
            }
            if let (Some(t), true) = (tier, w >= 52) {
                spans.push(Span::styled(
                    format!("[{t}] "),
                    Style::default().fg(DIM).bg(STATUSBG),
                ));
            }
            // The PROVIDER is part of the answer to "what am I talking to": the same model served
            // over codex-oauth, codex-cli and explabs is three different routes with three
            // different costs, quotas and effort ladders. It is dimmed rather than dropped so the
            // model name still reads as the headline. Narrow terminals fall back to the bare tail,
            // matching the activity panel and transcript: at 80 columns the row already carries
            // tier, cost, effort and temper, and a provider prefix would push the widgets after it
            // off the line entirely.
            if w >= 100 {
                if let Some((provider, _)) = model.split_once("::") {
                    spans.push(Span::styled(
                        format!("{provider}::"),
                        Style::default().fg(DIM).bg(STATUSBG),
                    ));
                }
            }
            spans.push(Span::styled(
                model_short(Some(model)),
                Style::default().fg(ACCENT).bold().bg(STATUSBG),
            ));
            // The rung the MODEL runs at, glued to the model id so it reads as part of the model's
            // identity rather than as another independent chip. `mesh_effort_chip` renders Forge's
            // own routing effort separately — see `model_effort_marker` for why they differ.
            // `routed_effort.or(effort)` mirrors exactly what the request sends: mesh's chosen
            // rung when it had a measured ladder, otherwise the session pin. Reading the pin alone
            // would show ⟨auto⟩ for a turn mesh had in fact picked a rung for.
            if let Some((marker, style)) =
                model_effort_marker(model, app.routed_effort.or(app.effort))
            {
                spans.push(Span::styled(marker, style));
            }
            Some(spans)
        }
        W::Tier => {
            let tier = app.routing.as_ref().map(|r| r.tier.clone())?;
            Some(vec![Span::styled(
                format!("[{tier}]"),
                Style::default().fg(DIM).bg(STATUSBG),
            )])
        }
        W::SessionCost => {
            let model_id = app.routing.as_ref().map(|r| r.model.as_str()).unwrap_or("");
            let text = cost_cell(model_id, app.cost_usd);
            let style = if text.starts_with('$') {
                Style::default().fg(OKGREEN).bold().bg(STATUSBG)
            } else {
                Style::default().fg(DIM).bg(STATUSBG)
            };
            Some(vec![Span::styled(format!("◈ {text}"), style)])
        }
        W::Effort => {
            // Rendered even when nothing is pinned. The old widget returned `None` there, so a
            // statusline configured to show effort showed nothing at all — the most common case,
            // and the one where a user most needs to see that mesh is choosing.
            // Gated wider than most cells because it now renders even when unpinned, so unlike
            // before it always costs its columns. At 80 the row is full; taking ~16 columns there
            // would silently clip whatever the user configured after it.
            if w < 90 {
                return None;
            }
            let (label, style) = mesh_effort_chip(app.effort);
            Some(vec![Span::styled(label, style)])
        }
        W::Mode => {
            if app.temper.is_empty() {
                return None;
            }
            if w < 46 {
                return None;
            }
            Some(vec![Span::styled(
                format!("◆ {}", app.temper),
                Style::default()
                    .fg(temper_color(&app.temper))
                    .bold()
                    .bg(STATUSBG),
            )])
        }
        W::TurnElapsed => {
            if !app.busy && !app.turn_ran {
                return None;
            }
            Some(vec![Span::styled(
                format!("⧖ {}", fmt_dur(app.turn_elapsed_secs)),
                Style::default()
                    .fg(if app.busy { ACCENT } else { DIM })
                    .bg(STATUSBG),
            )])
        }
        W::TokensIn => {
            if !app.busy && !app.turn_ran {
                return None;
            }
            if app.turn_in == 0 {
                return None;
            }
            Some(vec![Span::styled(
                format!("↑{}", human(app.turn_in)),
                Style::default()
                    .fg(if app.busy { ACCENT } else { DIM })
                    .bg(STATUSBG),
            )])
        }
        W::TokensOut => {
            if !app.busy && !app.turn_ran {
                return None;
            }
            if app.turn_out == 0 {
                return None;
            }
            Some(vec![Span::styled(
                format!("↓{}", human(app.turn_out)),
                Style::default()
                    .fg(if app.busy { ACCENT } else { DIM })
                    .bg(STATUSBG),
            )])
        }
        W::Throughput => {
            let shown = app
                .throughput
                .display(app.busy, std::time::Instant::now())?;
            Some(vec![Span::styled(shown.label(), throughput_style(&shown))])
        }
        W::SessionTokens => {
            if app.session_in == 0 && app.session_out == 0 {
                return None;
            }
            Some(vec![Span::styled(
                format!("Σ ↑{} ↓{}", human(app.session_in), human(app.session_out)),
                Style::default().fg(DIM).bg(STATUSBG),
            )])
        }
        W::GitBranch => {
            let branch = app.git_branch.as_deref()?;
            Some(vec![Span::styled(
                format!("⎇ {branch}"),
                Style::default().fg(DIM).bg(STATUSBG),
            )])
        }
        W::RepoName => {
            let repo = app.repo_name.as_deref()?;
            Some(vec![Span::styled(
                format!("⚑ {repo}"),
                Style::default().fg(DIM).bg(STATUSBG),
            )])
        }
        W::QuotaClaude => {
            let pct = app.usage_overlay.claude_5h_pct?;
            let color = if pct >= 90.0 {
                ERRRED
            } else if pct >= 70.0 {
                WARNYEL
            } else {
                DIM
            };
            Some(vec![Span::styled(
                format!("claude {pct:.0}%"),
                Style::default().fg(color).bg(STATUSBG),
            )])
        }
        W::QuotaCodex => {
            let pct = app.usage_overlay.codex_5h_pct?;
            let color = if pct >= 90.0 {
                ERRRED
            } else if pct >= 70.0 {
                WARNYEL
            } else {
                DIM
            };
            Some(vec![Span::styled(
                format!("codex {pct:.0}%"),
                Style::default().fg(color).bg(STATUSBG),
            )])
        }
        W::QuotaOpencodeGo => {
            let (window, pct) = app.usage_overlay.opencode_go_tightest_window()?;
            let color = if pct >= 90.0 {
                ERRRED
            } else if pct >= 70.0 {
                WARNYEL
            } else {
                DIM
            };
            Some(vec![Span::styled(
                format!("go {window} {pct:.0}%"),
                Style::default().fg(color).bg(STATUSBG),
            )])
        }
        W::QuotaPace => {
            let p = app.quota_pace.as_ref()?;
            let color = if p.exhaustion_warning {
                ERRRED
            } else if p.projected_fraction_at_reset.unwrap_or(0.0) >= 0.70 {
                WARNYEL
            } else {
                DIM
            };
            let short_window = match p.window.as_str() {
                "five_hour" => "5h",
                "weekly" => "wk",
                "monthly" => "mo",
                "" => "?",
                other => other,
            };
            let text = match p.projected_fraction_at_reset {
                Some(proj) => format!(
                    "⏱ {} {short_window} → {:.0}%",
                    p.provider.trim_end_matches("-cli"),
                    proj * 100.0
                ),
                None => format!(
                    "⏱ {} {short_window} {:+.1}%/hr",
                    p.provider.trim_end_matches("-cli"),
                    p.rate_per_hour * 100.0
                ),
            };
            Some(vec![Span::styled(
                text,
                Style::default().fg(color).bg(STATUSBG),
            )])
        }
        W::McpStatus => {
            if app.mcp_count == 0 {
                return None;
            }
            Some(vec![Span::styled(
                format!("⌬ {} mcp", app.mcp_count),
                Style::default().fg(DIM).bg(STATUSBG),
            )])
        }
        W::Custom {
            text,
            shell: Some(cmd),
            ..
        } => {
            let out = app
                .custom_widget_cache
                .get(cmd)
                .map(String::as_str)
                .unwrap_or(text);
            if out.is_empty() {
                return None;
            }
            Some(vec![Span::styled(
                out.to_string(),
                Style::default().fg(DIM).bg(STATUSBG),
            )])
        }
        W::Custom {
            text, shell: None, ..
        } => {
            if text.is_empty() {
                return None;
            }
            Some(vec![Span::raw(text.clone())])
        }
    }
}

/// Context-aware right-hint for the statusline. Highest-priority state first, so the most
/// actionable keybind for the current mode is what the user sees. Returns a `&'static str` so
/// the render path never allocates per frame.
pub(crate) fn statusline_hint(app: &App) -> &'static str {
    if app.command_center.open {
        "↑↓ select · ⏎ open · esc close"
    } else if app.palette.open {
        "↑↓ move · ⏎ run · esc close"
    } else if app.picker.open {
        "↑↓ move · ⏎ select · esc close"
    } else if app.pending_shell_fix.is_some() {
        "F apply fix · esc"
    } else if app.busy {
        "esc stop · Ctrl↑ escalate"
    } else if app.done {
        "done · esc quit"
    } else if app.input.is_empty() {
        "Ctrl+K actions · / · ? keys"
    } else {
        "⏎ send · ⇧⇥ temper"
    }
}

/// The recovery state that must outrank routine model/cost widgets. On narrow terminals this is
/// rendered as the whole first status row, so the next safe action is never clipped away.
fn stop_notice(app: &App, compact: bool) -> Option<(&'static str, Color)> {
    if !app.done {
        return None;
    }
    match app.last_stop_reason {
        Some(forge_types::StopReason::MaxSteps) => Some((
            if compact {
                "⚠ step limit — send continue"
            } else {
                "⚠ step limit — send `continue`"
            },
            WARNYEL,
        )),
        Some(forge_types::StopReason::BudgetExhausted) => Some(("✕ budget cap", ERRRED)),
        Some(forge_types::StopReason::TasksUnfinished) => Some((
            if compact {
                "✕ tasks unfinished"
            } else {
                "✕ stopped with tasks unfinished"
            },
            ERRRED,
        )),
        Some(forge_types::StopReason::NoOutput) => Some((
            if compact {
                "✕ no output — turn failed"
            } else {
                "✕ turn produced no answer and changed nothing"
            },
            ERRRED,
        )),
        _ => None,
    }
}

pub(crate) fn render_statusline(frame: &mut Frame, area: Rect, app: &App) {
    let bg = Style::default().bg(STATUSBG);
    let w = area.width;
    let sep = |s: &str| Span::styled(s.to_string(), Style::default().fg(SEPCOL).bg(STATUSBG));
    let widget_sep = || sep(&app.statusline_config.separator);

    // ── Row 1 ─────────────────────────────────────────────────────────────────
    // Build the configurable LEFT segment from the widget list.
    let mut left_spans: Vec<Span> = vec![Span::styled(" ", bg)];
    let mut first_widget = true;

    for widget in &app.statusline_config.left {
        if let Some(spans) = render_statusline_widget(widget, app, w) {
            if !first_widget {
                left_spans.push(widget_sep());
            }
            first_widget = false;
            left_spans.extend(spans);
        }
    }

    // Always-shown burst indicators appended after the configured widgets.
    // These are situational and not worth making configurable.
    if app.remote_active && w >= 52 {
        if !first_widget {
            left_spans.push(widget_sep());
        }
        first_widget = false;
        left_spans.push(Span::styled(
            "◉ remote",
            Style::default().fg(OKGREEN).bold().bg(STATUSBG),
        ));
    }
    if !app.queued.is_empty() {
        if !first_widget {
            left_spans.push(widget_sep());
        }
        first_widget = false;
        left_spans.push(Span::styled(
            format!("⏳ {} queued", app.queued.len()),
            Style::default().fg(WARNYEL).bold().bg(STATUSBG),
        ));
    }
    let narrow_stop = w < 50 && stop_notice(app, true).is_some();
    if let Some((label, color)) = stop_notice(app, narrow_stop) {
        if narrow_stop {
            // A model/cost segment is useful context; a recovery instruction is more important.
            // Do not leave a narrow terminal with only a clipped trailing warning.
            left_spans = vec![
                Span::styled(" ", bg),
                Span::styled(label, Style::default().fg(color).bold().bg(STATUSBG)),
            ];
        } else {
            if !first_widget {
                left_spans.push(widget_sep());
            }
            first_widget = false;
            left_spans.push(Span::styled(
                label,
                Style::default().fg(color).bold().bg(STATUSBG),
            ));
        }
    }
    let _ = first_widget; // suppress unused warning

    let version = concat!("v", env!("CARGO_PKG_VERSION"));
    let hint = statusline_hint(app);
    let row1 = Rect { height: 1, ..area };
    if narrow_stop {
        frame.render_widget(Paragraph::new(TextLine::from(left_spans)).style(bg), row1);
    } else if w >= 70 {
        let right_text = format!("{version}  {hint}");
        let right_len = right_text.chars().count() as u16;
        let cols =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(right_len)]).split(row1);
        frame.render_widget(
            Paragraph::new(TextLine::from(left_spans)).style(bg),
            cols[0],
        );
        frame.render_widget(
            Paragraph::new(TextLine::from(vec![
                Span::styled(
                    format!("{version}  "),
                    Style::default().fg(DIM).bg(STATUSBG),
                ),
                Span::styled(hint, Style::default().fg(DIM).bg(STATUSBG)),
            ]))
            .alignment(Alignment::Right)
            .style(bg),
            cols[1],
        );
    } else if w >= 40 {
        // Narrow: a short hint so the longer idle hint never overruns the version string.
        let short_hint = "/ · ? keys";
        let right_text = format!("{version}  {short_hint}");
        let right_len = right_text.chars().count() as u16;
        let cols =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(right_len)]).split(row1);
        frame.render_widget(
            Paragraph::new(TextLine::from(left_spans)).style(bg),
            cols[0],
        );
        frame.render_widget(
            Paragraph::new(TextLine::from(vec![
                Span::styled(
                    format!("{version}  "),
                    Style::default().fg(DIM).bg(STATUSBG),
                ),
                Span::styled(short_hint, Style::default().fg(DIM).bg(STATUSBG)),
            ]))
            .alignment(Alignment::Right)
            .style(bg),
            cols[1],
        );
    } else {
        // Too narrow: left-only, no hint (never overrun the version string).
        frame.render_widget(Paragraph::new(TextLine::from(left_spans)).style(bg), row1);
    }

    // ── Row 2 ─────────────────────────────────────────────────────────────────
    // Row 2: token timer, context gauge, session totals — unchanged from the original. Gated on
    // the same signal `statusline_height` used to decide whether to reserve this row at all (NOT
    // on raw `area.height`, which now also grows for unrelated `extra_rows` — conflating the two
    // would make row 2 swallow the row space actually meant for the first extra row whenever the
    // app is otherwise idle).
    let mut next_y = area.y + 1;
    if statusline_wants_row2(app) {
        let row2 = Rect {
            y: next_y,
            height: 1,
            ..area
        };
        next_y += 1;
        let mut line2: Vec<Span> = vec![Span::styled(" ", bg)];
        // Per-turn timer + this-turn token deltas: live (orange) while the turn runs, frozen (dim)
        // once it ends — like the per-response readout in Claude Code / Codex.
        let show_turn = app.busy || app.turn_ran;
        if show_turn {
            // CLI bridge models (agy-cli, codex-cli, claude-cli) don't report API token usage;
            // suppress the ↑/↓ counts when both are zero to avoid showing stale "↑0 ↓0".
            let has_token_data = app.turn_in > 0 || app.turn_out > 0;
            let turn_label = if has_token_data {
                if app.turn_cached_in.is_some_and(|cached| cached > 0) {
                    format!(
                        "⧖ {} ↑{} (↻{}) ↓{}",
                        fmt_dur(app.turn_elapsed_secs),
                        human(app.turn_in),
                        human(app.turn_cached_in.unwrap_or(0)),
                        human(app.turn_out)
                    )
                } else {
                    format!(
                        "⧖ {} ↑{} ↓{}",
                        fmt_dur(app.turn_elapsed_secs),
                        human(app.turn_in),
                        human(app.turn_out)
                    )
                }
            } else {
                format!("⧖ {}", fmt_dur(app.turn_elapsed_secs))
            };
            line2.push(Span::styled(
                turn_label,
                Style::default()
                    .fg(if app.busy { ACCENT } else { DIM })
                    .bg(STATUSBG),
            ));
            // Throughput sits with the turn readout rather than in its own cluster: "how much"
            // and "how fast" are the same question, and separating them costs a divider on a row
            // that is already the first thing to be clipped on a narrow terminal.
            if let Some(shown) = app.throughput.display(app.busy, std::time::Instant::now()) {
                line2.push(Span::styled(" ", bg));
                line2.push(Span::styled(shown.label(), throughput_style(&shown)));
            }
        }
        // Context gauge next — it's the most important readout, so it comes before the session
        // totals and survives right-truncation on a narrow terminal.
        if app.context_tokens > 0 || app.context_limit.is_some() {
            if line2.len() > 1 {
                line2.push(sep("  │  "));
            }
            line2.extend(context_gauge_spans(app.context_tokens, app.context_limit));
        }
        // Session running totals last (least critical — the per-turn figures are above): if the row
        // is too narrow this is what gets clipped, not the gauge.
        // Only show when session differs from turn delta: on the first turn turn_base_in=0 so
        // turn_in == session_in — showing both would be identical, useless duplication.
        let session_differs = app.session_in != app.turn_in
            || app.session_cached_in != app.turn_cached_in
            || app.session_out != app.turn_out;
        if (app.session_in > 0 || app.session_out > 0) && (!show_turn || session_differs) {
            if line2.len() > 1 {
                line2.push(sep("  │  "));
            }
            // Omitted both when nothing was cached and when the provider does not report caching:
            // the row states a cache figure only where one was actually measured.
            let total_label = if app.session_cached_in.is_some_and(|cached| cached > 0) {
                format!(
                    "Σ ↑{} (↻{}) ↓{}",
                    human(app.session_in),
                    human(app.session_cached_in.unwrap_or(0)),
                    human(app.session_out)
                )
            } else {
                format!("Σ ↑{} ↓{}", human(app.session_in), human(app.session_out))
            };
            line2.push(Span::styled(
                total_label,
                Style::default().fg(DIM).bg(STATUSBG),
            ));
        }
        frame.render_widget(Paragraph::new(TextLine::from(line2)).style(bg), row2);
    }

    // ── Extra rows (user-configured) ────────────────────────────────────────────
    // Each `extra_rows` entry is one more left-aligned row below row 1 (and row 2, if shown),
    // using the same widget rendering + separator as row 1. `statusline_height` already reserved
    // the space; `next_y` picks up wherever row 2 left off (or right after row 1 if row 2 didn't
    // render this frame).
    for row_widgets in &app.statusline_config.extra_rows {
        let y = next_y;
        if y >= area.y + area.height {
            break;
        }
        next_y += 1;
        let mut spans: Vec<Span> = vec![Span::styled(" ", bg)];
        let mut first = true;
        for widget in row_widgets {
            if let Some(w_spans) = render_statusline_widget(widget, app, w) {
                if !first {
                    spans.push(widget_sep());
                }
                first = false;
                spans.extend(w_spans);
            }
        }
        let row = Rect {
            y,
            height: 1,
            ..area
        };
        frame.render_widget(Paragraph::new(TextLine::from(spans)).style(bg), row);
    }
}

/// Colour a throughput readout by band. A finished turn's average is always dim: it is a
/// different measurement from the live rate (a turn is mostly tool calls, not generation) and
/// must not read as the speed the model is running at right now.
fn throughput_style(shown: &crate::throughput::Throughput) -> Style {
    use crate::throughput::{FAST_TOK_PER_SEC, SLOW_TOK_PER_SEC};
    if !shown.live {
        return Style::default().fg(DIM).bg(STATUSBG);
    }
    let colour = if shown.tok_per_sec >= FAST_TOK_PER_SEC {
        OKGREEN
    } else if shown.tok_per_sec < SLOW_TOK_PER_SEC {
        WARNYEL
    } else {
        ACCENT
    };
    Style::default().fg(colour).bg(STATUSBG)
}
