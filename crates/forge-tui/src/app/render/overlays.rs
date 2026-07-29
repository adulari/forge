use super::*;

/// Render `/usage` as a centered, summary-first inspector. Spend and token details stay in one
/// focused view that leaves the active conversation stable in the background.
pub fn render_usage_overlay(f: &mut Frame, app: &App) {
    if !app.usage_overlay.open {
        return;
    }
    let area = f.area();
    // Five summary rows, one table header, and every model row. The inspector grows only as far
    // as its content needs, preventing a small spend report from becoming a tall empty panel.
    let content_rows = 6u16.saturating_add(app.usage_overlay.by_model.len() as u16);
    let inspector = surface::inspector_area(area, content_rows);

    let spinner = SPINNER[(app.usage_overlay.anim_tick as usize) % SPINNER.len()];
    let title = if app.usage_overlay.loading {
        format!("{spinner} Usage · loading")
    } else {
        "Usage".to_string()
    };
    let inner = surface::render_panel(
        f,
        inspector,
        surface::title(title, surface::SurfaceTone::Accent),
        Some(surface::hint("Esc close")),
        surface::SurfaceTone::Accent,
    );

    if inner.width < 20 || inner.height < 4 {
        return;
    }
    let summary_h = inner.height.min(5);
    let chunks = Layout::vertical([Constraint::Length(summary_h), Constraint::Min(0)]).split(inner);

    let o = &app.usage_overlay;

    // Derive totals from per-model breakdowns so subscription ($0) rows still show tokens.
    let (cost_5h, in_5h, out_5h) = UsageOverlay::totals(&o.by_model_5h);
    let (cost_today, in_today, out_today) = UsageOverlay::totals(&o.by_model);
    let (cost_week, in_week, out_week) = UsageOverlay::totals(&o.by_model_week);

    // Bridge-provider annotation for each period row.
    // A staleness note for the Claude rate-limit %, so an old reading is never shown as live.
    let claude_age = rl_age_note(o.claude_rl_age_secs);
    let bridge_5h = {
        let mut parts = Vec::new();
        if let Some(p) = o.codex_5h_pct {
            parts.push(format!("codex:{:.0}%", p));
        }
        if let Some(p) = o.claude_5h_pct {
            parts.push(format!("claude:{:.0}%{}", p, claude_age));
        } else if o.claude_rl_age_secs.is_some() {
            // Cache exists but the 5h reading is too old to trust (5h window) — say so plainly
            // rather than falling back to a confusing multi-million raw-token sum.
            parts.push(format!("claude:5h stale{claude_age}"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("  [{}]", parts.join("  "))
        }
    };
    let bridge_week = {
        let mut parts = Vec::new();
        if let Some(p) = o.codex_weekly_pct {
            parts.push(format!("codex:{:.0}%", p));
        }
        if let Some(p) = o.claude_weekly_pct {
            parts.push(format!("claude:{:.0}%{}", p, claude_age));
        } else if o.claude_rl_age_secs.is_some() {
            parts.push(format!("claude:wk stale{claude_age}"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("  [{}]", parts.join("  "))
        }
    };

    let fmt_period =
        |label: &str, cost: f64, inp: u64, out: u64, cap: Option<f64>, bridge: &str| -> String {
            let tok_str = format!("↑{} ↓{}", format_tok(inp), format_tok(out));
            let cost_str = if cost > 0.0 {
                format!("${cost:.4}")
            } else {
                "sub".to_string()
            };
            if let Some(c) = cap {
                let pct = (cost / c * 100.0).min(100.0);
                format!("{label:<8}{tok_str}  {cost_str} / ${c:.2} ({pct:.0}%){bridge}")
            } else {
                format!("{label:<8}{tok_str}  {cost_str}{bridge}")
            }
        };

    let month_str = if let Some(cap) = o.monthly_cap {
        let pct = (o.month_usd / cap * 100.0).min(100.0);
        format!(
            "{:<8}${:.4} / ${:.2}  ({:.0}%)",
            "Month", o.month_usd, cap, pct
        )
    } else {
        format!("{:<8}${:.4}", "Month", o.month_usd)
    };
    let session_str = format!(
        "{:<8}↑{} ↓{}  ${:.4}",
        "Session",
        format_tok(o.session_in),
        format_tok(o.session_out),
        o.session_usd,
    );
    let summary_text = ratatui::text::Text::from(vec![
        ratatui::text::Line::from(fmt_period("5h", cost_5h, in_5h, out_5h, None, &bridge_5h)),
        ratatui::text::Line::from(fmt_period(
            "Today",
            cost_today,
            in_today,
            out_today,
            o.daily_cap,
            "",
        )),
        ratatui::text::Line::from(fmt_period(
            "Week",
            cost_week,
            in_week,
            out_week,
            o.weekly_cap,
            &bridge_week,
        )),
        ratatui::text::Line::from(month_str),
        ratatui::text::Line::from(session_str),
    ]);
    f.render_widget(Paragraph::new(summary_text), chunks[0]);

    use ratatui::style::Modifier;
    use ratatui::widgets::{Cell, Row, Table};
    let header = Row::new(vec![
        Cell::from("Model").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cost (today)").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("↑ In").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("↓ Out").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);
    let rows: Vec<Row> = o
        .by_model
        .iter()
        .map(|(model, cost, inp, out)| {
            let display = if model.is_empty() {
                "side calls".to_string()
            } else {
                model.clone()
            };
            let style = if display.starts_with("claude-cli")
                || display.starts_with("codex-cli")
                || display.starts_with("agy-cli")
            {
                Style::default().fg(TOOLCYAN)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(display.clone()).style(style),
                Cell::from(cost_cell(&display, *cost)).style(style),
                Cell::from(format_tok(*inp)).style(style),
                Cell::from(format_tok(*out)).style(style),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(50),
            Constraint::Percentage(20),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ],
    )
    .header(header)
    .block(Block::default());
    f.render_widget(table, chunks[1]);
}

/// Honest cost label for a per-model row. A flat-rate subscription bridge (Claude Code / Codex
/// CLI) genuinely costs $0 per call, so it reads "subscription". A priced model shows its dollar
/// cost. At $0 with no bridge, `forge_mesh::catalog::is_free` tells apart a model we KNOW charges
/// nothing (ollama, groq, a `:free` gateway variant, …) from one we simply have no price for
/// (unpriced OpenRouter/OpenCode Zen models, which may still burn real gateway credit) — the
/// former reads "free", the latter "untracked" rather than lying that it's costless.
pub(crate) fn cost_cell(model: &str, cost: f64) -> String {
    let subscription = forge_mesh::catalog::is_subscription(model);
    if subscription {
        "subscription".to_string()
    } else if cost > 0.0 {
        format!("${cost:.5}")
    } else if forge_mesh::catalog::is_free(model, cost, subscription) {
        "free".to_string()
    } else {
        "untracked".to_string()
    }
}

/// A 14-cell colour-coded meter for a fraction, eased by `ease` (animation grow-in).
fn mesh_meter(frac: f64, ease: f32, status: &str) -> Vec<Span<'static>> {
    let shown = (frac as f32 * ease).clamp(0.0, 1.0);
    let filled = (shown * 14.0).round() as usize;
    let col = match status {
        "Exhausted" => ERRRED,
        "Warning" => WARNYEL,
        _ if frac >= 0.6 => WARNYEL,
        _ => OKGREEN,
    };
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(col)),
        Span::styled("░".repeat(14 - filled), Style::default().fg(DIM)),
    ]
}

/// A compact `→ 93% at reset ⚠` suffix for a quota line when a pace projection exists
/// (mesh-routing.md) — `""` when there isn't enough history to project one yet.
pub(crate) fn mesh_pace_suffix(
    projected_fraction_at_reset: Option<f64>,
    exhaustion_warning: bool,
) -> String {
    match projected_fraction_at_reset {
        Some(p) => format!(
            " → {:.0}% at reset{}",
            p * 100.0,
            if exhaustion_warning { " ⚠" } else { "" }
        ),
        None => String::new(),
    }
}

fn mesh_truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

/// Render `/mesh` as a centered routing inspector. The decision and fallback path lead; the full
/// ranked candidate list remains below for users who need to inspect the routing mechanics.
pub fn render_mesh_overlay(f: &mut Frame, app: &App) {
    if !app.mesh_overlay.open {
        return;
    }
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Text};

    let o = &app.mesh_overlay;
    let area = f.area();
    // Keep the first routing decision and a useful candidate window visible. Larger candidate
    // sets retain the existing cursor-following scroll behavior instead of stretching the dialog.
    let decision_rows = 1u16
        .saturating_add(u16::from(!o.classified.is_empty() || !o.routed.is_empty()))
        .saturating_add(u16::from(!o.pick.is_empty()))
        .saturating_add(u16::from(!o.classifier.is_empty()))
        .saturating_add(u16::from(!o.fallbacks.is_empty()));
    let quota_rows = o.quota.len() as u16 + u16::from(!o.conserve_line.is_empty());
    let candidate_rows =
        (o.candidates.len() as u16).clamp(1, 8) + 2 + u16::from(!o.rationale.is_empty());
    let content_rows = decision_rows + quota_rows + candidate_rows + 2;
    let inspector = surface::inspector_area(area, content_rows);

    let settled = o.anim_tick >= o.settle_tick();
    let glyph = if settled {
        "◈"
    } else {
        SPINNER[(o.anim_tick as usize) % SPINNER.len()]
    };
    let title = format!("{glyph} Model routing");
    let inner = surface::render_panel(
        f,
        inspector,
        surface::title(title, surface::SurfaceTone::Tool),
        Some(surface::hint("↑/↓ browse candidates · Esc close")),
        surface::SurfaceTone::Tool,
    );

    if inner.width < 20 || inner.height < 4 {
        return;
    }
    // Show loading spinner while bridge stats + routing explanation are fetched in background.
    if o.loading {
        let spinner = SPINNER[(o.anim_tick as usize) % SPINNER.len()];
        f.render_widget(
            ratatui::widgets::Paragraph::new(format!(" {spinner} analyzing routing…"))
                .style(Style::default().fg(DIM)),
            inner,
        );
        return;
    }

    let ease = ((o.anim_tick as f32) / 6.0).min(1.0);

    // --- header + quota gauges + conservation verdict ---
    let mut top: Vec<Line> = Vec::new();
    if o.prompt.is_empty() {
        top.push(Line::from(Span::styled(
            "Routing overview — run `/mesh <task>` to trace a specific request.",
            Style::default().fg(DIM),
        )));
    } else {
        top.push(Line::from(vec![
            Span::styled("task  ", Style::default().fg(DIM)),
            Span::raw(mesh_truncate(
                &o.prompt,
                inner.width.saturating_sub(8) as usize,
            )),
        ]));
    }
    if !o.classified.is_empty() || !o.routed.is_empty() {
        let tier = if !o.routed.is_empty() && o.routed != o.classified {
            format!("{} → {}   ({})", o.classified, o.routed, o.reasons)
        } else {
            format!("{}   ({})", o.classified, o.reasons)
        };
        top.push(Line::from(vec![
            Span::styled("decision  ", Style::default().fg(DIM)),
            Span::styled(tier, Style::default().fg(ACCENT).bold()),
        ]));
    }
    if !o.pick.is_empty() {
        top.push(Line::from(vec![
            Span::styled("selected  ", Style::default().fg(DIM)),
            Span::styled(o.pick.clone(), Style::default().fg(OKGREEN).bold()),
        ]));
    }
    if !o.classifier.is_empty() {
        top.push(Line::from(vec![
            Span::styled("classifier  ", Style::default().fg(DIM)),
            Span::styled(o.classifier.clone(), Style::default().fg(DIM)),
        ]));
    }
    if !o.fallbacks.is_empty() {
        top.push(Line::from(Span::styled(
            mesh_truncate(
                &format!("fallbacks  {}", o.fallbacks.join(" → ")),
                inner.width as usize,
            ),
            Style::default().fg(DIM),
        )));
    }
    top.push(Line::from(""));
    for q in &o.quota {
        let mut spans = vec![Span::styled(
            format!("  {:<11} ", q.provider),
            Style::default(),
        )];
        spans.extend(mesh_meter(q.fraction, ease, &q.status));
        let plan = if q.plan.is_empty() { "?" } else { &q.plan };
        spans.push(Span::styled(
            format!(
                " {:>3.0}% · {plan} · {} · spread {:.0}%{}",
                q.fraction * 100.0 * ease as f64,
                q.status,
                q.spread_complex * 100.0,
                mesh_pace_suffix(q.projected_fraction_at_reset, q.exhaustion_warning),
            ),
            Style::default().fg(if q.exhaustion_warning { WARNYEL } else { DIM }),
        ));
        top.push(Line::from(spans));
    }
    if !o.conserve_line.is_empty() {
        let col = if o.conserve_fired { WARNYEL } else { DIM };
        top.push(Line::from(Span::styled(
            format!("  conserve  {}", o.conserve_line),
            Style::default().fg(col),
        )));
    }
    top.push(Line::from(""));

    let top_h = (top.len() as u16).min(inner.height.saturating_sub(3));
    let chunks = Layout::vertical([Constraint::Length(top_h), Constraint::Min(0)]).split(inner);
    f.render_widget(Paragraph::new(Text::from(top)), chunks[0]);

    // --- candidate table (revealed row-by-row) + final pick ---
    let revealed = ((o.anim_tick as usize) / 2).min(o.candidates.len());
    let model_w = inner.width.saturating_sub(40).clamp(16, 48) as usize;
    let cursor = o.cursor.min(o.candidates.len().saturating_sub(1));
    let mut cursor_line = 0u16;
    let mut rows: Vec<Line> = Vec::new();
    for (i, c) in o.candidates.iter().take(revealed.max(1)).enumerate() {
        let marker = if c.selected { "▶" } else { " " };
        let pen = if c.penalty > 0.0 {
            format!(" −{:.0}", c.penalty)
        } else {
            String::new()
        };
        let tag = format!(
            "{}{}{}{}",
            c.cost_tag,
            pen,
            if c.frontier { " · frontier" } else { "" },
            if c.usable { "" } else { " · unusable" },
        );
        let mut base = if c.selected {
            Style::default().fg(OKGREEN).add_modifier(Modifier::BOLD)
        } else if !c.usable {
            Style::default().fg(DIM)
        } else {
            Style::default()
        };
        // The browsing cursor (↑/↓) is independent of the routed pick (▶) — reverse video marks
        // whichever row is currently highlighted, on top of whatever color the pick/usability
        // already applied.
        if i == cursor {
            base = base.add_modifier(Modifier::REVERSED);
            cursor_line = rows.len() as u16;
        }
        rows.push(Line::from(vec![
            Span::styled(format!("{marker} #{:<2} ", c.rank), base),
            Span::styled(
                format!(
                    "{:<width$}",
                    mesh_truncate(&c.model, model_w),
                    width = model_w
                ),
                base,
            ),
            Span::styled(format!("  {:>6.2}  ", c.score), base),
            Span::styled(
                tag,
                if i == cursor || c.selected {
                    base
                } else {
                    Style::default().fg(DIM)
                },
            ),
        ]));
    }
    rows.push(Line::from(""));
    rows.push(Line::from(vec![
        Span::styled("pick  ", Style::default().fg(DIM)),
        Span::styled(
            o.pick.clone(),
            Style::default().fg(OKGREEN).add_modifier(Modifier::BOLD),
        ),
    ]));
    if !o.rationale.is_empty() {
        rows.push(Line::from(Span::styled(
            mesh_truncate(&format!("why   {}", o.rationale), inner.width as usize),
            Style::default().fg(DIM),
        )));
    }
    // Auto-scroll to keep the cursor row on-screen: stay at the top until the cursor scrolls past
    // the last visible row, then follow it exactly to the bottom edge. Purely a function of
    // `cursor_line` + the viewport height — no state to persist across frames.
    let body_h = chunks[1].height;
    let max_scroll = (rows.len() as u16).saturating_sub(body_h);
    let scroll = if cursor_line < body_h {
        0
    } else {
        (cursor_line + 1).saturating_sub(body_h)
    }
    .min(max_scroll);
    f.render_widget(
        Paragraph::new(Text::from(rows)).scroll((scroll, 0)),
        chunks[1],
    );
}
