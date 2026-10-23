use super::*;
use crate::theme::{color, styles};

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

/// How much of the gauge to draw: bar and "of limit", bar only, or just the percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GaugeSize {
    Full,
    Compact,
    Minimal,
}

/// A context window as a short label: `32k`, `128k`, `1M` (no trailing `.0`).
fn window_label(limit: u64) -> String {
    human(limit).replace(".0k", "k").replace(".0M", "M")
}

/// The window the gauge measures against: the model's real limit, or a conservative fallback
/// (marked `~`) so a percentage can still be shown.
fn gauge_limit(limit: Option<u32>) -> (u64, bool) {
    match limit {
        Some(l) if l > 0 => (l as u64, false),
        _ => (CONTEXT_FALLBACK_LIMIT, true),
    }
}

/// The context gauge in the shape the status line asks for: `ctx ▰▱▱▱▱▱▱▱ 12% of 32k`, coloured
/// muted below 70 percent, warn from 70 and err from 90.
pub(crate) fn context_gauge(used: u64, limit: Option<u32>, size: GaugeSize) -> Vec<Span<'static>> {
    let glyphs = crate::theme::glyphs::g();
    // No number yet: a gauge with nothing in it says nothing.
    if used == 0 && limit.is_none() {
        return Vec::new();
    }
    let (limit, approx) = gauge_limit(limit);
    let frac = (used as f64 / limit as f64).clamp(0.0, 1.0);
    let pct = (frac * 100.0).round() as u64;
    let style = styles::gauge(frac);
    let mut spans = vec![Span::styled("ctx ", styles::faint())];
    if size != GaugeSize::Minimal {
        let cells = if size == GaugeSize::Full { 8 } else { 4 };
        let bar = glyphs.bar(frac, cells);
        let bar = if glyphs.unicode {
            bar
        } else {
            format!("[{bar}]")
        };
        spans.push(Span::styled(bar, style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(format!("{pct}%"), style));
    if size == GaugeSize::Full {
        let tilde = if approx { "~" } else { "" };
        spans.push(Span::styled(
            format!(" of {tilde}{}", window_label(limit)),
            styles::muted(),
        ));
    }
    spans
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
    let bg = Style::default();
    let spans: Vec<Span> = if let Some(c) = &app.compaction {
        let elapsed = app.tick.saturating_sub(c.start_tick) as f64 * 0.06;
        // Indeterminate work (one summarizer call): ease toward a ceiling instead of faking a real
        // fraction; CompactionFinished clears the band (the "snap to done").
        let frac = 1.0 - (-elapsed / COMPACT_EASE_TAU_SECS).exp();
        let pct = (frac * 95.0).round() as u64;
        const CELLS: usize = 16;
        let glyphs = crate::theme::glyphs::g();
        let spin = glyphs.spin(app.tick);
        let bar = glyphs.bar(frac * 0.95, CELLS);
        let label = if c.auto {
            "auto-compacting"
        } else {
            "compacting"
        };
        vec![
            Span::styled(format!(" {spin} {label} "), styles::user().bold()),
            Span::styled(bar, styles::user()),
            Span::styled(format!(" {pct}%  {elapsed:.1}s"), styles::muted()),
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
        let style = if frac >= AUTO_COMPACT_FRACTION {
            styles::err()
        } else if frac >= 0.72 {
            styles::warn()
        } else {
            styles::muted()
        };
        let msg = if frac >= AUTO_COMPACT_FRACTION {
            format!(
                " {} context {pct}% — auto-compact imminent",
                crate::theme::glyphs::g().warn
            )
        } else {
            format!(
                " {} context {pct}% — auto-compact at {:.0}% (~{} left)",
                crate::theme::glyphs::g().warn,
                AUTO_COMPACT_FRACTION * 100.0,
                human(left)
            )
        };
        vec![Span::styled(msg, style)]
    };
    frame.render_widget(Paragraph::new(TextLine::from(spans)).style(bg), area);
}

/// Session cost for the status line: `$0`, `<$0.01` for dust, otherwise two decimals.
pub(crate) fn fmt_cost(cost: f64) -> String {
    if cost <= 0.0 {
        "$0".to_string()
    } else if cost < 0.01 {
        "<$0.01".to_string()
    } else {
        format!("${cost:.2}")
    }
}

/// One row for the built-in status line, plus one per `statusline_config.extra_rows` entry (a
/// static, config-driven count) and one for a `[statusline] command` once it has output. Used by
/// [`render_live`] to allocate the status area.
pub fn statusline_height(app: &App) -> u16 {
    1 + app.statusline_config.extra_rows.len() as u16 + u16::from(command_line(app).is_some())
}

/// The first line a `[statusline] command` produced, once it has produced one.
pub(crate) fn command_line(app: &App) -> Option<&str> {
    let cmd = app.statusline_config.command.as_ref()?;
    app.custom_widget_cache
        .get(cmd)
        .map(String::as_str)
        .filter(|out| !out.is_empty())
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

/// Full rung name for the mesh chip, which has room for it. Deliberately not the same spelling as
/// the glued marker: seeing "mesh medium" beside "⟨med⟩" makes it obvious at a glance that the two
/// cells are answering different questions rather than repeating one value.
fn rung_long(level: forge_types::EffortLevel) -> &'static str {
    match level {
        forge_types::EffortLevel::Low => "low",
        forge_types::EffortLevel::Medium => "medium",
        forge_types::EffortLevel::High => "high",
        forge_types::EffortLevel::XHigh => "xhigh",
        forge_types::EffortLevel::WhiteHot => "white-hot",
    }
}

/// Compact rung name for the marker glued to the model id, where every column competes with the
/// model name itself.
fn rung_short(level: forge_types::EffortLevel) -> &'static str {
    match level {
        forge_types::EffortLevel::Low => "low",
        forge_types::EffortLevel::Medium => "med",
        forge_types::EffortLevel::High => "high",
        forge_types::EffortLevel::XHigh => "xhigh",
        forge_types::EffortLevel::WhiteHot => "max",
    }
}

/// The rung marker glued to the model id: what the MODEL is running at.
///
/// Deliberately different from the mesh chip beside it. Three states, because the honest answer has
/// three cases and collapsing them is how the old statusline came to report a level nothing was
/// running at:
///
/// - `⟨med⟩` bright — Forge set this rung and it is on the wire.
/// - `⟨auto⟩` dim   — the model has a reasoning control but Forge sent nothing, so the PROVIDER
///   chose the rung. Forge does not know which one, and must not name one.
/// - nothing        — the model has no reasoning control at all, so it has no rung to show.
pub(crate) fn model_effort_marker(
    model_id: &str,
    pinned: Option<forge_types::EffortLevel>,
) -> Option<(String, Style)> {
    if !forge_types::effort::has_control(model_id) {
        return None;
    }
    let decision = forge_types::effort::resolve_id(model_id, pinned);
    Some(match decision.sent {
        Some(level) => (
            format!("⟨{}⟩", rung_short(level)),
            styles::effort(level, 0).bold(),
        ),
        None => ("⟨auto⟩".to_string(), styles::muted()),
    })
}

/// Forge's OWN effort: the level mesh routes with. A separate cell from the marker above because
/// they answer different questions and are frequently different values — mesh may route at high
/// while the chosen model is asked for medium.
pub(crate) fn mesh_effort_chip(
    pinned: Option<forge_types::EffortLevel>,
    tick: usize,
) -> (String, Style) {
    match pinned {
        Some(forge_types::EffortLevel::WhiteHot) => (
            "mesh white-hot".to_string(),
            styles::effort(forge_types::EffortLevel::WhiteHot, tick).bold(),
        ),
        Some(level) => (
            format!("mesh {}", rung_long(level)),
            styles::effort(level, tick),
        ),
        // Nothing pinned: mesh is choosing. Dim, and named "auto" rather than given the level mesh
        // happens to default to: the user did not set one.
        None => ("mesh auto".to_string(), styles::muted()),
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

/// Sparkle chars that cycle at XHigh stop positions and handle (Unicode tier only).
const SPARKLES: [char; 6] = ['✦', '✧', '⋆', '✺', '✼', '❋'];

/// A rung of the ember heat ramp (0 cool .. 4 white-hot); static when motion is reduced.
fn heat(step: usize) -> Color {
    crate::theme::t().palette.heat[step.min(4)].fg
}

/// Where in the ramp an animated rung sits at `tick` (always the same step under reduced motion).
fn ramp_step(tick: usize, speed: usize) -> usize {
    if crate::theme::t().animates() {
        (tick / speed.max(1)) % 5
    } else {
        3
    }
}

fn slider_idx(app: &App) -> usize {
    let cur = app.effort.unwrap_or(forge_types::EffortLevel::Medium);
    EFFORT_LEVELS.iter().position(|&l| l == cur).unwrap_or(1)
}

fn slider_border_color(idx: usize, tick: usize) -> Color {
    match idx {
        0 => color::fg_faint(),
        1 => color::fg_muted(),
        2 => color::brand(),
        3 => color::brand_hot(),
        _ => heat(ramp_step(tick, 2)),
    }
}

fn slider_fill_color(idx: usize, tick: usize, pos: usize) -> Color {
    match idx {
        0 => color::fg_faint(),
        1 => color::fg_muted(),
        2 => color::brand(),
        3 => color::brand_hot(),
        // The track heats up toward the handle: ember at the left edge, white-hot at the
        // handle, with a slow tick shimmer rippling along it.
        _ => heat(if crate::theme::t().animates() {
            (pos / 3 + tick / 4) % 5
        } else {
            3
        }),
    }
}

fn slider_handle_color(idx: usize, tick: usize) -> Color {
    match idx {
        0 => color::fg_muted(),
        1 => color::fg(),
        2 => color::brand(),
        3 => color::brand_hot(),
        // Pulse between white-hot and gold: the hottest point of the forge.
        _ => heat(
            if crate::theme::t().animates() && (tick / 3).is_multiple_of(2) {
                4
            } else {
                3
            },
        ),
    }
}

fn slider_label_style(idx: usize, tick: usize) -> Style {
    match idx {
        0 => styles::faint(),
        1 => styles::muted().bold(),
        2 => styles::brand().bold(),
        3 => styles::brand_hot().bold(),
        _ => Style::default().fg(heat(ramp_step(tick, 3))).bold(),
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
        format!(" {sp} white-hot {sp} ")
    } else if idx == 3 {
        let sp = SPARKLES[(tick / 2) % SPARKLES.len()];
        format!(" {sp} effort {sp} ")
    } else {
        " effort ".to_string()
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
            Some(_) => ('○', styles::muted()),
            None if filled => ('━', Style::default().fg(slider_fill_color(idx, tick, pos))),
            None => ('─', styles::muted()),
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans.push(Span::raw("  "));
    spans.push(Span::styled(label_text, slider_label_style(idx, tick)));

    frame.render_widget(Paragraph::new(TextLine::from(spans)), inner);
}

#[cfg(test)]
mod effort_cell_tests {
    use super::*;
    use forge_types::EffortLevel;

    fn marker(id: &str, pinned: Option<EffortLevel>) -> Option<String> {
        model_effort_marker(id, pinned).map(|(text, _)| text)
    }

    #[test]
    fn the_marker_names_the_rung_forge_actually_sent() {
        assert_eq!(
            marker("codex-oauth::gpt-6-astra", Some(EffortLevel::Medium)).as_deref(),
            Some("⟨med⟩")
        );
    }

    #[test]
    fn an_unpinned_reasoning_model_says_auto_rather_than_naming_a_level() {
        // The whole point of the cell. Forge sends no rung here, so the provider picked one and
        // Forge does not know which — naming a level would be the exact lie this replaces.
        assert_eq!(
            marker("codex-oauth::gpt-6-astra", None).as_deref(),
            Some("⟨auto⟩")
        );
        let (_, style) = model_effort_marker("codex-oauth::gpt-6-astra", None).unwrap();
        assert_eq!(
            style,
            styles::muted(),
            "a provider-chosen rung reads as dim"
        );
    }

    #[test]
    fn a_model_with_no_reasoning_control_shows_no_marker_at_all() {
        assert_eq!(marker("groq::llama-3.3-70b", Some(EffortLevel::High)), None);
        assert_eq!(marker("groq::llama-3.3-70b", None), None);
    }

    #[test]
    fn the_marker_shows_the_clamped_rung_not_the_pin() {
        // agy tops out at high. The cell reports what is on the wire, so a white-hot pin against a
        // three-rung surface must not render as "max" — that would be the pin talking, not the
        // model.
        assert_eq!(
            marker("agy-cli::gemini-3.8-flash", Some(EffortLevel::WhiteHot)).as_deref(),
            Some("⟨high⟩")
        );
        // claude does have the top rung, so there it is genuinely max.
        assert_eq!(
            marker("claude-cli::opus", Some(EffortLevel::WhiteHot)).as_deref(),
            Some("⟨max⟩")
        );
    }

    #[test]
    fn the_mesh_chip_and_the_model_marker_can_legitimately_disagree() {
        // Mesh routes at white-hot while the chosen model is only asked for high. Both are true at
        // once, which is why they are two cells and not one.
        let (chip, _) = mesh_effort_chip(Some(EffortLevel::WhiteHot), 0);
        assert_eq!(chip, "mesh white-hot");
        assert_eq!(
            marker("agy-cli::gemini-3.8-flash", Some(EffortLevel::WhiteHot)).as_deref(),
            Some("⟨high⟩")
        );
    }

    #[test]
    fn the_mesh_chip_renders_when_nothing_is_pinned() {
        // Previously the widget returned nothing at all in this state, so a statusline configured
        // to show effort showed none — in the most common case of all.
        let (chip, style) = mesh_effort_chip(None, 0);
        assert_eq!(chip, "mesh auto");
        assert_eq!(style, styles::muted());
    }
}
