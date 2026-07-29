use super::*;

/// A real status bar: working state · mesh tier+model · cost, with right-aligned key
/// hints. Lower-priority segments drop out on narrow terminals; model+cost always show.
/// Humanize a token count: `< 1000` as-is, `< 1M` as `12.3k`, else `1.1M`.
pub(crate) fn human(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// The context-window gauge spans: `◷ used/limit N%` (N% colored dim<70 / yellow≥70 / red≥90),
/// or just `◷ used` when the model's limit is unknown (no fabricated denominator).
/// Assumed context window when the model's real limit is unknown (not in the pricing table), so a
/// percentage + bar can still be shown. Marked approximate (`~`) in the UI. 128k is a common
/// mid-size window — conservative enough to warn before most models actually overflow.
const CONTEXT_FALLBACK_LIMIT: u64 = 128_000;

/// Render the context gauge: a small bar + `used/limit` + `pct%`, colored by fill. When the model's
/// real window is unknown, a conservative fallback is assumed and the reading is marked `~approx`.
pub(crate) fn context_gauge_spans(used: u64, limit: Option<u32>) -> Vec<Span<'static>> {
    let (limit, approx) = match limit {
        Some(l) if l > 0 => (l as u64, false),
        _ => (CONTEXT_FALLBACK_LIMIT, true),
    };
    let frac = (used as f64 / limit as f64).clamp(0.0, 1.0);
    let pct = (frac * 100.0).round() as u64;
    let color = if pct >= 90 {
        ERRRED
    } else if pct >= 70 {
        WARNYEL
    } else {
        DIM
    };
    // A compact 10-cell bar: filled cells scale with the fill fraction.
    const CELLS: usize = 10;
    let filled = (frac * CELLS as f64).round() as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(CELLS - filled);
    let tail = if approx { " ~approx" } else { "" };
    vec![
        Span::styled("◷ ", Style::default().fg(DIM).bg(STATUSBG)),
        Span::styled(bar, Style::default().fg(color).bg(STATUSBG)),
        Span::styled(
            format!(" {}/{} ", human(used), human(limit)),
            Style::default().fg(DIM).bg(STATUSBG),
        ),
        Span::styled(
            format!("{pct}%{tail}"),
            Style::default().fg(color).bold().bg(STATUSBG),
        ),
    ]
}

/// A compact " (Xm/Xh ago)" suffix for rate-limit data older than ~10 min; empty when fresh or
/// unknown. Keeps the overlay honest about staleness instead of presenting old % as live.
pub(crate) fn rl_age_note(age_secs: Option<i64>) -> String {
    match age_secs {
        Some(a) if a >= 3600 => format!(" ({}h ago)", a / 3600),
        Some(a) if a >= 600 => format!(" ({}m ago)", a / 60),
        _ => String::new(),
    }
}

pub(crate) fn format_tok(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Context fill at/above which the "approaching auto-compact" hint appears below the input.
const COMPACT_HINT_FRACTION: f64 = 0.65;
/// The context fill at which the core auto-compacts (~80% of the usable window). Shown as the
/// target in the hint and used to color it red once reached.
const AUTO_COMPACT_FRACTION: f64 = 0.80;
/// Time constant (seconds) for easing the indeterminate compaction bar toward its ceiling.
const COMPACT_EASE_TAU_SECS: f64 = 2.5;

/// Current context fill as a fraction (0..=1), using the same fallback window as the gauge.
/// `None` when there's no token/limit signal yet (so no band is shown on a fresh session).
fn context_fraction(app: &App) -> Option<f64> {
    if app.context_tokens == 0 && app.context_limit.is_none() {
        return None;
    }
    let limit = match app.context_limit {
        Some(l) if l > 0 => l as u64,
        _ => CONTEXT_FALLBACK_LIMIT,
    };
    Some((app.context_tokens as f64 / limit as f64).clamp(0.0, 1.0))
}

/// One row while compaction runs (animated bar) or while the context is approaching the
/// auto-compact threshold (hint); zero otherwise.
pub(crate) fn compact_band_height(app: &App) -> u16 {
    if app.compaction.is_some() {
        return 1;
    }
    match context_fraction(app) {
        Some(f) if f >= COMPACT_HINT_FRACTION => 1,
        _ => 0,
    }
}

/// Render the compaction band: an animated, eased progress bar with elapsed time while compacting,
/// else a colored "approaching auto-compact" hint with the tokens remaining until the trigger.
pub(crate) fn render_compact_band(frame: &mut Frame, area: Rect, app: &App) {
    let bg = Style::default().bg(STATUSBG);
    let spans: Vec<Span> = if let Some(c) = &app.compaction {
        let elapsed = app.tick.saturating_sub(c.start_tick) as f64 * 0.06;
        // Indeterminate work (one summarizer call): ease toward a ceiling instead of faking a real
        // fraction; CompactionFinished clears the band (the "snap to done").
        let frac = 1.0 - (-elapsed / COMPACT_EASE_TAU_SECS).exp();
        let pct = (frac * 95.0).round() as u64;
        const CELLS: usize = 16;
        let filled = ((frac * 0.95) * CELLS as f64).round() as usize;
        let filled = filled.min(CELLS);
        let spin = SPINNER[app.tick % SPINNER.len()];
        let bar: String = "█".repeat(filled) + &"░".repeat(CELLS - filled);
        let label = if c.auto {
            "auto-compacting"
        } else {
            "compacting"
        };
        vec![
            Span::styled(
                format!(" {spin} {label} "),
                Style::default().fg(ACCENT).bold().bg(STATUSBG),
            ),
            Span::styled(bar, Style::default().fg(ACCENT).bg(STATUSBG)),
            Span::styled(
                format!(" {pct}%  {elapsed:.1}s"),
                Style::default().fg(DIM).bg(STATUSBG),
            ),
        ]
    } else {
        let frac = context_fraction(app).unwrap_or(0.0);
        let pct = (frac * 100.0).round() as u64;
        let limit = match app.context_limit {
            Some(l) if l > 0 => l as u64,
            _ => CONTEXT_FALLBACK_LIMIT,
        };
        let trigger = (AUTO_COMPACT_FRACTION * limit as f64) as u64;
        let left = trigger.saturating_sub(app.context_tokens);
        let color = if frac >= AUTO_COMPACT_FRACTION {
            ERRRED
        } else if frac >= 0.72 {
            WARNYEL
        } else {
            DIM
        };
        let msg = if frac >= AUTO_COMPACT_FRACTION {
            format!(" ⚠ context {pct}% — auto-compact imminent")
        } else {
            format!(
                " ⚠ context {pct}% — auto-compact at {:.0}% (~{} left)",
                AUTO_COMPACT_FRACTION * 100.0,
                human(left)
            )
        };
        vec![Span::styled(msg, Style::default().fg(color).bg(STATUSBG))]
    };
    frame.render_widget(Paragraph::new(TextLine::from(spans)).style(bg), area);
}

/// Whether row 2 (turn timer / context gauge / session totals) has anything to show. Shared by
/// `statusline_height` (to size the reserved area) and `render_statusline` (to decide whether to
/// render it) so the two can never disagree about which row extra_rows starts on.
pub(crate) fn statusline_wants_row2(app: &App) -> bool {
    app.context_tokens > 0
        || app.context_limit.is_some()
        || app.session_in > 0
        || app.session_out > 0
        || app.busy
        || app.turn_ran
}

/// Returns 1 when idle (no session data), 2 once context / token data is available, plus one row
/// per `statusline_config.extra_rows` entry (a static, config-driven count — see `extra_rows`'s
/// doc comment). Used by [`render_live`] to allocate the right number of rows for the status area.
pub fn statusline_height(app: &App) -> u16 {
    let base = if statusline_wants_row2(app) { 2 } else { 1 };
    base + app.statusline_config.extra_rows.len() as u16
}

/// Compact wall-clock duration for the turn timer: `Ns` under a minute, `MmSSs` under an hour,
/// `HhMMm` beyond. No leading zeros on the largest unit so it stays short in the statusline.
pub(crate) fn fmt_dur(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub(crate) fn effort_status(effort: forge_types::EffortLevel) -> (&'static str, Style) {
    match effort {
        forge_types::EffortLevel::Low => ("effort low", Style::default().fg(TOOLCYAN).bg(STATUSBG)),
        forge_types::EffortLevel::Medium => (
            "effort medium",
            Style::default().fg(WARNYEL).bold().bg(STATUSBG),
        ),
        forge_types::EffortLevel::High => (
            "▲ effort high",
            Style::default().fg(WARNYEL).bold().bg(STATUSBG),
        ),
        forge_types::EffortLevel::XHigh => (
            "▲▲ effort xhigh",
            Style::default().fg(ERRRED).bold().bg(STATUSBG),
        ),
        forge_types::EffortLevel::WhiteHot => (
            "⚒ WHITE-HOT",
            Style::default()
                .fg(Color::Rgb(255, 252, 235))
                .bold()
                .bg(STATUSBG),
        ),
    }
}

// ── Effort Slider ─────────────────────────────────────────────────────────────

const EFFORT_SLIDER_H: u16 = 3;
const EFFORT_LEVELS: [forge_types::EffortLevel; 5] = [
    forge_types::EffortLevel::Low,
    forge_types::EffortLevel::Medium,
    forge_types::EffortLevel::High,
    forge_types::EffortLevel::XHigh,
    forge_types::EffortLevel::WhiteHot,
];
const EFFORT_LABELS: [&str; 5] = ["LOW", "MEDIUM", "HIGH", "XHIGH", "WHITE·HOT"];

/// The WHITE·HOT ramp — the forge at the temperature where metal glows white:
/// ember → flame → forge orange → gold → white-hot.
const WHITEHOT_RAMP: [Color; 6] = [
    Color::Rgb(216, 45, 28),   // deep ember
    Color::Rgb(255, 96, 32),   // flame
    Color::Rgb(255, 152, 40),  // forge orange
    Color::Rgb(255, 208, 64),  // gold
    Color::Rgb(255, 238, 150), // near-white gold
    Color::Rgb(255, 252, 235), // white-hot
];

/// Sparkle chars that cycle at XHigh stop positions and handle.
const SPARKLES: [char; 6] = ['✦', '✧', '⋆', '✺', '✼', '❋'];

/// 12-color rainbow for XHigh — each track char gets a phase-shifted hue.
const XHIGH_COLORS: [Color; 12] = [
    Color::Rgb(255, 75, 110), // rose
    Color::Rgb(255, 110, 55), // coral
    Color::Rgb(255, 155, 30), // amber
    Color::Rgb(255, 215, 45), // gold
    Color::Rgb(190, 255, 55), // lime
    Color::Rgb(75, 230, 125), // neon-green
    Color::Rgb(35, 215, 215), // teal
    Color::Rgb(82, 162, 255), // electric-blue
    Color::Rgb(110, 95, 255), // indigo
    Color::Rgb(185, 75, 255), // violet
    Color::Rgb(255, 55, 255), // magenta
    Color::Rgb(255, 75, 160), // hot-pink
];

/// Three-phase pulse for HIGH: orange → gold → hot-red.
const HIGH_PULSE: [Color; 3] = [
    Color::Rgb(255, 138, 48),
    Color::Rgb(255, 210, 40),
    Color::Rgb(245, 55, 35),
];

fn slider_idx(app: &App) -> usize {
    let cur = app.effort.unwrap_or(forge_types::EffortLevel::Medium);
    EFFORT_LEVELS.iter().position(|&l| l == cur).unwrap_or(1)
}

fn slider_border_color(idx: usize, tick: usize) -> Color {
    match idx {
        0 => Color::Rgb(55, 60, 88),
        1 => TOOLCYAN,
        2 => HIGH_PULSE[(tick / 5) % HIGH_PULSE.len()],
        3 => XHIGH_COLORS[tick % XHIGH_COLORS.len()],
        _ => WHITEHOT_RAMP[(tick / 2) % WHITEHOT_RAMP.len()],
    }
}

fn slider_fill_color(idx: usize, tick: usize, pos: usize) -> Color {
    match idx {
        0 => Color::Rgb(85, 92, 118),
        1 => TOOLCYAN,
        2 => HIGH_PULSE[(tick / 5) % HIGH_PULSE.len()],
        3 => XHIGH_COLORS[(tick + pos) % XHIGH_COLORS.len()],
        // The track heats up toward the handle: ember at the left edge, white-hot at the
        // handle, with a slow tick shimmer rippling along it.
        _ => WHITEHOT_RAMP[(pos / 3 + tick / 4) % WHITEHOT_RAMP.len()],
    }
}

fn slider_handle_color(idx: usize, tick: usize) -> Color {
    match idx {
        0 => Color::Rgb(175, 180, 208),
        1 => Color::Rgb(115, 242, 248),
        2 => {
            let t = (tick % 12) as f32;
            let pulse = (std::f32::consts::PI * t / 12.0).sin();
            Color::Rgb(255, (120.0 + 110.0 * pulse) as u8, (30.0 * pulse) as u8)
        }
        3 => XHIGH_COLORS[(tick * 3) % XHIGH_COLORS.len()],
        // Blinding pulse between white-hot and gold — the hottest point of the forge.
        _ => {
            if (tick / 3).is_multiple_of(2) {
                Color::Rgb(255, 252, 235)
            } else {
                Color::Rgb(255, 220, 90)
            }
        }
    }
}

fn slider_label_style(idx: usize, tick: usize) -> Style {
    match idx {
        0 => Style::default().fg(DIM),
        1 => Style::default().fg(Color::Rgb(115, 242, 248)).bold(),
        2 => Style::default()
            .fg(HIGH_PULSE[(tick / 5) % HIGH_PULSE.len()])
            .bold(),
        3 => Style::default()
            .fg(XHIGH_COLORS[(tick * 2) % XHIGH_COLORS.len()])
            .bold(),
        _ => Style::default()
            .fg(WHITEHOT_RAMP[3 + (tick / 3) % 3])
            .bold(),
    }
}

/// Draw the effort slider popup: 3 rows anchored at the bottom of `area`.
/// Uses the shared surface frame so the control aligns with every other Forge overlay.
pub(crate) fn render_effort_slider(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < EFFORT_SLIDER_H || area.width < 24 {
        return;
    }
    let idx = slider_idx(app);
    let tick = app.tick;
    let border_col = slider_border_color(idx, tick);

    let box_area = Rect {
        x: area.x,
        y: area.y + area.height - EFFORT_SLIDER_H,
        width: area.width,
        height: EFFORT_SLIDER_H,
    };

    let title_text = if idx == 4 {
        let sp = SPARKLES[(tick / 2) % SPARKLES.len()];
        format!(" {sp} ⚒ forge · white-hot {sp} ")
    } else if idx == 3 {
        let sp = SPARKLES[(tick / 2) % SPARKLES.len()];
        format!(" {sp} effort {sp} ")
    } else {
        " ⚡ effort ".to_string()
    };

    let block = surface::panel(
        Span::styled(title_text, Style::default().fg(border_col).bold()),
        surface::SurfaceTone::Brand,
    )
    .border_style(Style::default().fg(border_col))
    .title_bottom(surface::hint("←/→ adjust  Esc close"));

    let inner = block.inner(box_area);
    frame.render_widget(block, box_area);

    // ── Track line in the 1-row inner area ───────────────────────────────────
    let label_text = EFFORT_LABELS[idx];
    let label_len = label_text.chars().count() as u16;
    // " " pad + track + "  " + label + " " pad = inner.width
    let track_w = (inner.width.saturating_sub(1 + 2 + label_len + 1)) as usize;
    if track_w < 4 {
        return;
    }
    let stops: [usize; 5] = std::array::from_fn(|i| i * track_w.saturating_sub(1) / 4);

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for pos in 0..track_w {
        let at_stop = stops.iter().position(|&s| s == pos);
        let filled = stops.get(idx).is_some_and(|&s| pos <= s);
        let (ch, style) = match at_stop {
            Some(si) if si == idx => {
                let hcol = slider_handle_color(idx, tick);
                let ch = if idx >= 3 {
                    SPARKLES[(tick * 2) % SPARKLES.len()]
                } else {
                    '●'
                };
                (ch, Style::default().fg(hcol).bold())
            }
            Some(si) if si < idx => {
                let fcol = slider_fill_color(idx, tick, pos);
                let ch = if idx >= 3 {
                    SPARKLES[(tick + si * 4) % SPARKLES.len()]
                } else {
                    '●'
                };
                (ch, Style::default().fg(fcol).bold())
            }
            Some(_) => ('○', Style::default().fg(DIM)),
            None if filled => ('━', Style::default().fg(slider_fill_color(idx, tick, pos))),
            None => ('─', Style::default().fg(DIM)),
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans.push(Span::raw("  "));
    spans.push(Span::styled(label_text, slider_label_style(idx, tick)));

    frame.render_widget(Paragraph::new(TextLine::from(spans)), inner);
}
