//! `/mesh`: the routing inspector.

use super::*;
use crate::theme::glyphs::g;

/// A 14-cell colour-coded meter for a fraction, eased by `ease` (animation grow-in).
fn mesh_meter(frac: f64, ease: f32, status: &str) -> Vec<Span<'static>> {
    let shown = (frac as f32 * ease).clamp(0.0, 1.0);
    let filled = (shown * 14.0).round() as usize;
    let style = match status {
        "Exhausted" => styles::err(),
        "Warning" => styles::warn(),
        _ if frac >= 0.6 => styles::warn(),
        _ => styles::ok(),
    };
    let glyphs = g();
    vec![
        Span::styled(glyphs.gauge_full.repeat(filled), style),
        Span::styled(glyphs.gauge_empty.repeat(14 - filled), styles::faint()),
    ]
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
    let title = if settled {
        "Model routing".to_string()
    } else {
        format!("{} Model routing", g().spin(o.anim_tick as usize))
    };
    let inner = crate::overlays::padded_panel(
        f,
        inspector,
        surface::title(title, surface::SurfaceTone::Tool),
        Some(Span::styled(" /mesh ", styles::faint())),
        Some(Span::styled(
            format!(" {}browse candidates {} Esc close ", g().up_down, g().sep),
            styles::faint(),
        )),
        surface::SurfaceTone::Tool,
    );

    if inner.width < 20 || inner.height < 4 {
        return;
    }
    // Show loading spinner while bridge stats + routing explanation are fetched in background.
    if o.loading {
        let spinner = g().spin(o.anim_tick as usize);
        f.render_widget(
            ratatui::widgets::Paragraph::new(format!("{spinner} analyzing routing…"))
                .style(styles::muted()),
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
            styles::muted(),
        )));
    } else {
        top.push(Line::from(vec![
            Span::styled("task  ", styles::muted()),
            Span::styled(
                mesh_truncate(&o.prompt, inner.width.saturating_sub(8) as usize),
                styles::fg(),
            ),
        ]));
    }
    if !o.classified.is_empty() || !o.routed.is_empty() {
        let tier = if !o.routed.is_empty() && o.routed != o.classified {
            format!("{} → {}   ({})", o.classified, o.routed, o.reasons)
        } else {
            format!("{}   ({})", o.classified, o.reasons)
        };
        top.push(Line::from(vec![
            Span::styled("decision  ", styles::muted()),
            Span::styled(tier, styles::user().bold()),
        ]));
    }
    if !o.pick.is_empty() {
        let mut spans = vec![
            Span::styled("selected  ", styles::muted()),
            Span::styled(o.pick.clone(), styles::ok().bold()),
        ];
        if let Some(rung) = &o.pick_effort {
            spans.push(Span::styled(format!("⟨{rung}⟩"), styles::ok().bold()));
        }
        top.push(Line::from(spans));
    }
    if !o.classifier.is_empty() {
        top.push(Line::from(vec![
            Span::styled("classifier  ", styles::muted()),
            Span::styled(o.classifier.clone(), styles::muted()),
        ]));
    }
    if !o.fallbacks.is_empty() {
        top.push(Line::from(Span::styled(
            mesh_truncate(
                &format!("fallbacks  {}", o.fallbacks.join(" → ")),
                inner.width as usize,
            ),
            styles::muted(),
        )));
    }
    top.push(Line::from(""));
    for q in &o.quota {
        let mut spans = vec![Span::styled(format!("  {:<11} ", q.provider), styles::fg())];
        let plan = if q.plan.is_empty() { "?" } else { &q.plan };
        let usage = q
            .fraction
            .map(|fraction| {
                spans.extend(mesh_meter(fraction, ease, &q.status));
                format!(" {:.0}%", fraction * 100.0 * ease as f64)
            })
            .unwrap_or_else(|| " unknown".to_string());
        spans.push(Span::styled(
            format!(
                "{usage} · {plan} · {} · spread {:.0}%{}",
                q.status,
                q.spread_complex * 100.0,
                crate::theme::glyphs::normalise(&mesh_pace_suffix(
                    q.projected_fraction_at_reset,
                    q.exhaustion_warning
                )),
            ),
            if q.exhaustion_warning {
                styles::warn()
            } else {
                styles::muted()
            },
        ));
        if !q.pace_note.is_empty() {
            spans.push(Span::styled(
                format!(" · {}", q.pace_note),
                if q.over_pace {
                    styles::warn()
                } else {
                    styles::muted()
                },
            ));
        }
        top.push(Line::from(spans));
    }
    if !o.conserve_line.is_empty() {
        let col = if o.conserve_fired {
            styles::warn()
        } else {
            styles::muted()
        };
        top.push(Line::from(Span::styled(
            format!("  conserve  {}", o.conserve_line),
            col,
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
        let glyphs = g();
        let marker = match (i == cursor, c.selected) {
            (true, _) => glyphs.prompt,
            (false, true) => glyphs.ok,
            _ => " ",
        };
        let pen = if c.penalty > 0.0 {
            format!(" −{:.0}", c.penalty)
        } else {
            String::new()
        };
        let tag = format!(
            "{}{}{}{}{}{}",
            c.cost_tag,
            pen,
            // Same ⟨rung⟩ notation the statusline glues to the model id, so the two surfaces read
            // as one vocabulary rather than two.
            c.effort
                .as_ref()
                .map_or_else(String::new, |rung| format!(" · ⟨{rung}⟩")),
            c.reorder_reason
                .as_ref()
                .map_or_else(String::new, |reason| format!(" · ↕ {reason}")),
            if c.frontier { " · frontier" } else { "" },
            if c.usable { "" } else { " · unusable" },
        );
        let mut base = if c.selected {
            styles::ok().add_modifier(Modifier::BOLD)
        } else if !c.usable {
            styles::muted()
        } else {
            styles::fg()
        };
        // The browsing cursor (↑/↓) is independent of the routed pick (✓): the raised row and
        // `❯` caret mark whichever row is highlighted, on top of the pick/usability colour.
        if i == cursor {
            base = base.patch(styles::selected_row());
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
                    styles::muted()
                },
            ),
        ]));
    }
    rows.push(Line::from(""));
    let mut pick_spans = vec![
        Span::styled("pick  ", styles::muted()),
        Span::styled(o.pick.clone(), styles::ok().add_modifier(Modifier::BOLD)),
    ];
    if let Some(rung) = &o.pick_effort {
        pick_spans.push(Span::styled(
            format!("⟨{rung}⟩"),
            styles::ok().add_modifier(Modifier::BOLD),
        ));
    }
    rows.push(Line::from(pick_spans));
    if !o.rationale.is_empty() {
        rows.push(Line::from(Span::styled(
            mesh_truncate(&format!("why   {}", o.rationale), inner.width as usize),
            styles::muted(),
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
