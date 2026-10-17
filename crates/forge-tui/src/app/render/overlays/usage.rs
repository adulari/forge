//! `/usage`: spend and token totals by period, subscription quotas as bars, and today's spend
//! per model, in aligned columns with explicit empty states.

use unicode_width::UnicodeWidthStr;

use super::*;
use crate::app::render::{cost_cell, format_tok, rl_age_note};
use crate::overlays::{fmt_cost, UsageOverlay};
use crate::theme::glyphs::g;

const LABEL_W: usize = 9;
const NUM_W: usize = 9;
const COST_W: usize = 12;

/// Width of the quota bars, in cells.
const BAR_CELLS: usize = 10;

fn section(label: &str) -> TextLine<'static> {
    TextLine::from(Span::styled(label.to_string(), styles::muted().bold()))
}

fn right(text: &str, width: usize) -> String {
    format!("{text:>width$}")
}

/// Cost cell for a period row: dollars, `$0` or `<$0.01`.
fn period_cost(cost: f64) -> String {
    fmt_cost(cost)
}

/// Per-model cost: the honest label from [`cost_cell`] (subscription, free, untracked), with
/// priced rows in the same two-decimal format as every other cost on the screen.
fn model_cost(model: &str, cost: f64) -> String {
    let cell = cost_cell(model, cost);
    if cell.starts_with('$') {
        fmt_cost(cost)
    } else {
        cell
    }
}

fn period_row(
    label: &str,
    inp: Option<u64>,
    out: Option<u64>,
    cost: String,
    note: Option<String>,
) -> TextLine<'static> {
    let tok = |v: Option<u64>| v.map_or_else(|| "—".to_string(), format_tok);
    let mut spans = vec![
        Span::styled(format!("{label:<LABEL_W$}"), styles::fg()),
        Span::styled(right(&tok(inp), NUM_W), styles::fg()),
        Span::styled(right(&tok(out), NUM_W), styles::fg()),
        Span::styled(right(&cost, COST_W), styles::fg().bold()),
    ];
    if let Some(note) = note {
        spans.push(Span::styled(format!("  {note}"), styles::muted()));
    }
    TextLine::from(spans)
}

/// `of $5.00 (42%)` for a capped period.
fn cap_note(cost: f64, cap: Option<f64>) -> Option<String> {
    cap.map(|c| format!("of ${c:.2} ({:.0}%)", (cost / c * 100.0).min(100.0)))
}

fn quota_row(label: &str, pct: f64, stale: &str) -> TextLine<'static> {
    let frac = (pct / 100.0).clamp(0.0, 1.0);
    TextLine::from(vec![
        Span::styled(format!("{label:<12}"), styles::fg()),
        Span::styled(g().bar(frac, BAR_CELLS), styles::gauge(frac)),
        Span::styled(format!(" {pct:.0}%"), styles::muted()),
        Span::styled(stale.to_string(), styles::faint()),
    ])
}

/// The overlay body for `width` cells.
pub(crate) fn usage_lines(o: &UsageOverlay, width: usize) -> Vec<TextLine<'static>> {
    let (cost_5h, in_5h, out_5h) = UsageOverlay::totals(&o.by_model_5h);
    let (cost_today, in_today, out_today) = UsageOverlay::totals(&o.by_model);
    let (cost_week, in_week, out_week) = UsageOverlay::totals(&o.by_model_week);

    let mut lines = vec![TextLine::from(vec![Span::styled(
        format!(
            "{:<LABEL_W$}{}{}{}",
            "PERIOD",
            right("IN", NUM_W),
            right("OUT", NUM_W),
            right("COST", COST_W)
        ),
        styles::muted().bold(),
    )])];
    lines.push(period_row(
        "5h",
        Some(in_5h),
        Some(out_5h),
        period_cost(cost_5h),
        None,
    ));
    lines.push(period_row(
        "Today",
        Some(in_today),
        Some(out_today),
        period_cost(cost_today),
        cap_note(cost_today, o.daily_cap),
    ));
    lines.push(period_row(
        "Week",
        Some(in_week),
        Some(out_week),
        period_cost(cost_week),
        cap_note(cost_week, o.weekly_cap),
    ));
    lines.push(period_row(
        "Month",
        None,
        None,
        period_cost(o.month_usd),
        cap_note(o.month_usd, o.monthly_cap),
    ));
    lines.push(period_row(
        "Session",
        Some(o.session_in),
        Some(o.session_out),
        period_cost(o.session_usd),
        None,
    ));

    // Subscription quotas as bars. Claude's reading comes from a cache, so it says how old it is.
    let stale = rl_age_note(o.claude_rl_age_secs);
    let quotas: Vec<(&str, Option<f64>, &str)> = vec![
        ("claude 5h", o.claude_5h_pct, stale.as_str()),
        ("claude week", o.claude_weekly_pct, stale.as_str()),
        ("codex 5h", o.codex_5h_pct, ""),
        ("codex week", o.codex_weekly_pct, ""),
        ("go 5h", o.opencode_go_5h_pct, ""),
        ("go week", o.opencode_go_weekly_pct, ""),
        ("go month", o.opencode_go_monthly_pct, ""),
    ];
    let shown: Vec<_> = quotas.iter().filter(|(_, pct, _)| pct.is_some()).collect();
    lines.push(TextLine::default());
    lines.push(section("QUOTAS"));
    if shown.is_empty() {
        lines.push(TextLine::from(Span::styled(
            if o.loading {
                "Reading subscription quotas…".to_string()
            } else {
                "No subscription quota readings yet.".to_string()
            },
            styles::faint(),
        )));
    }
    for (label, pct, stale) in shown {
        lines.push(quota_row(label, pct.unwrap_or_default(), stale));
    }
    // The mesh's pacing verdict per subscription, quoted verbatim: the bars say how much is
    // used, this says whether routing is holding models back because of it.
    for note in &o.pace_notes {
        let style = if note.over_pace {
            styles::warn()
        } else {
            styles::muted()
        };
        lines.push(TextLine::from(vec![
            Span::styled(format!("{:<12}", "pace"), styles::faint()),
            Span::styled(format!("{}: {}", note.provider, note.note), style),
        ]));
    }

    lines.push(TextLine::default());
    lines.push(section("BY MODEL (today)"));
    if o.by_model.is_empty() {
        lines.push(TextLine::from(Span::styled(
            "No model usage yet today — costs appear after the first turn.",
            styles::faint(),
        )));
        return lines;
    }
    let name_w = width.saturating_sub(NUM_W * 2 + COST_W).max(12);
    for (model, cost, inp, out) in &o.by_model {
        let display = if model.is_empty() {
            "side calls".to_string()
        } else {
            model.clone()
        };
        let bridge = ["claude-cli", "codex-cli", "agy-cli"]
            .iter()
            .any(|p| display.starts_with(p));
        let style = if bridge {
            styles::muted()
        } else {
            styles::fg()
        };
        let mut name = display.clone();
        if name.width() > name_w {
            while name.width() > name_w - 1 {
                name.pop();
            }
            name.push_str(g().ellipsis);
        }
        let pad = name_w.saturating_sub(name.width());
        lines.push(TextLine::from(vec![
            Span::styled(format!("{name}{}", " ".repeat(pad)), style),
            Span::styled(right(&format_tok(*inp), NUM_W), style),
            Span::styled(right(&format_tok(*out), NUM_W), style),
            Span::styled(right(&model_cost(&display, *cost), COST_W), style),
        ]));
    }
    lines
}

/// Render `/usage` as a centred, summary-first inspector. Spend and token details stay in one
/// focused view that leaves the active conversation stable in the background.
pub fn render_usage_overlay(f: &mut Frame, app: &App) {
    if !app.usage_overlay.open {
        return;
    }
    let area = f.area();
    let width = area.width.saturating_sub(4).clamp(10, 100).min(area.width);
    let text_w = width.saturating_sub(2 + crate::overlays::PAD_X * 2) as usize;
    let lines = usage_lines(&app.usage_overlay, text_w);
    let height = (lines.len() as u16)
        .saturating_add(2 + crate::overlays::PAD_Y * 2)
        .min(area.height.saturating_sub(2).max(5));
    let card = surface::modal_area(area, width, height);
    let title = if app.usage_overlay.loading {
        format!(
            "{} Usage · loading",
            g().spin(app.usage_overlay.anim_tick as usize)
        )
    } else {
        "Usage".to_string()
    };
    let body = crate::overlays::padded_panel(
        f,
        card,
        surface::title(title, surface::SurfaceTone::Accent),
        Some(Span::styled(" /usage ", styles::faint())),
        Some(Span::styled(" Esc close ", styles::faint())),
        surface::SurfaceTone::Accent,
    );
    if body.width < 20 || body.height < 4 {
        return;
    }
    f.render_widget(Paragraph::new(lines), body);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[TextLine<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn periods_align_and_costs_use_at_most_two_decimals() {
        let o = UsageOverlay {
            by_model_week: vec![("openai::gpt-5".into(), 0.42, 41_200, 9_800)],
            by_model_5h: vec![("openai::gpt-5".into(), 0.004, 10, 5)],
            month_usd: 3.1,
            session_in: 72,
            session_out: 30,
            ..Default::default()
        };
        let out = text(&usage_lines(&o, 90));
        assert!(out.contains("PERIOD") && out.contains("COST"), "{out}");
        assert!(!out.contains("$0.0000") && !out.contains("0.4200"), "{out}");
        let week = out.lines().find(|l| l.starts_with("Week")).unwrap();
        assert!(week.ends_with("$0.42"), "{week:?}");
        let five = out.lines().find(|l| l.starts_with("5h")).unwrap();
        assert!(five.ends_with("<$0.01"), "dust shows as <$0.01: {five:?}");
        let session = out.lines().find(|l| l.starts_with("Session")).unwrap();
        assert!(session.ends_with("$0"), "zero cost is $0: {session:?}");
        let header = out.lines().next().unwrap();
        let end_of = |l: &str| l.chars().count();
        assert_eq!(
            end_of(header),
            end_of(week),
            "numeric columns share one right edge"
        );
        assert!(out.contains("Month") && out.contains("$3.10"), "{out}");
    }

    #[test]
    fn quotas_draw_bars_and_empty_states_are_explicit() {
        let mut o = UsageOverlay::default();
        let empty = text(&usage_lines(&o, 80));
        assert!(
            empty.contains("No subscription quota readings yet."),
            "{empty}"
        );
        assert!(empty.contains("No model usage yet today"), "{empty}");

        o.claude_5h_pct = Some(61.0);
        o.codex_weekly_pct = Some(18.0);
        o.claude_rl_age_secs = Some(1_200);
        let out = text(&usage_lines(&o, 80));
        assert!(
            out.contains("claude 5h") && out.contains("▰▰▰▰▰▰▱▱▱▱ 61%"),
            "{out}"
        );
        assert!(
            out.contains("(20m ago)"),
            "stale claude reading says so: {out}"
        );
        assert!(
            out.contains("codex week") && out.contains("▰▰▱▱▱▱▱▱▱▱ 18%"),
            "{out}"
        );
        assert!(
            !out.contains("go week"),
            "only providers with a reading appear: {out}"
        );
    }

    #[test]
    fn model_rows_align_under_their_columns_and_clip_long_names() {
        let o = UsageOverlay {
            by_model: vec![
                ("ollama::llama3".into(), 0.0, 1_200, 300),
                (
                    "openrouter::some-very-long-vendor/some-extremely-long-model-name-v2".into(),
                    1.5,
                    10,
                    20,
                ),
            ],
            ..Default::default()
        };
        let out = text(&usage_lines(&o, 60));
        let rows: Vec<&str> = out
            .lines()
            .filter(|l| l.contains("llama3") || l.contains("openrouter"))
            .collect();
        assert_eq!(rows.len(), 2, "{out}");
        assert!(
            rows[1].contains('…'),
            "long names clip with an ellipsis: {:?}",
            rows[1]
        );
        assert_eq!(rows[0].chars().count(), rows[1].chars().count(), "{rows:?}");
        assert!(rows[0].contains("free"), "{rows:?}");
    }
}
