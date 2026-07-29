//! Pure routing and usage overlay presentation data.
//!
//! These types are populated from presenter events and rendered by `App`; they
//! own no terminal or session side effects.

/// The Mesh routing decision currently displayed.
#[derive(Debug, Clone, Default)]
pub struct RoutingView {
    pub tier: String,
    pub model: String,
    pub rationale: String,
}

/// Data for the `/usage` overlay — API spend + token breakdown across providers.
#[derive(Debug, Default, Clone)]
pub struct UsageOverlay {
    pub open: bool,
    /// True while bridge stats are still loading in the background (subscription %s absent).
    pub loading: bool,
    /// Per-model rows for the last 5 hours: (model, cost_usd, input_tokens, output_tokens).
    pub by_model_5h: Vec<(String, f64, u64, u64)>,
    /// Per-model rows for today: (model, cost_usd, input_tokens, output_tokens).
    pub by_model: Vec<(String, f64, u64, u64)>,
    /// Per-model rows for this week: (model, cost_usd, input_tokens, output_tokens).
    pub by_model_week: Vec<(String, f64, u64, u64)>,
    /// This month's total spend in USD (scalar; not per-model).
    pub month_usd: f64,
    /// Session spend in USD (from the running Cost events).
    pub session_usd: f64,
    /// Session input tokens.
    pub session_in: u64,
    /// Session output tokens.
    pub session_out: u64,
    /// Daily cap (from config), None if uncapped.
    pub daily_cap: Option<f64>,
    /// Weekly cap (from config), None if uncapped.
    pub weekly_cap: Option<f64>,
    /// Monthly cap (from config), None if uncapped.
    pub monthly_cap: Option<f64>,
    /// Codex 5-hour used % (0–100), from latest local session file.
    pub codex_5h_pct: Option<f64>,
    /// Codex weekly used % (0–100), from latest local session file.
    pub codex_weekly_pct: Option<f64>,
    /// Claude 5-hour used % (0–100), from ~/.claude/.rate-limits-cache.json written by statusline.
    pub claude_5h_pct: Option<f64>,
    /// Claude weekly used % (0–100), from ~/.claude/.rate-limits-cache.json.
    pub claude_weekly_pct: Option<f64>,
    /// Claude tokens (input incl cache) used in the last 5 hours.
    pub claude_5h_in: u64,
    pub claude_5h_out: u64,
    /// Claude tokens used this ISO week.
    pub claude_weekly_in: u64,
    pub claude_weekly_out: u64,
    /// Age (seconds) of the Claude rate-limit cache, if present — drives a "Xh ago" staleness
    /// note so the overlay never presents an old percentage as if it were live.
    pub claude_rl_age_secs: Option<i64>,
    /// Animation tick counter (incremented each tick, used for spinner).
    pub anim_tick: u32,
}

impl UsageOverlay {
    pub(crate) fn totals(rows: &[(String, f64, u64, u64)]) -> (f64, u64, u64) {
        rows.iter().fold((0.0, 0, 0), |acc, r| {
            (acc.0 + r.1, acc.1 + r.2, acc.2 + r.3)
        })
    }
}

/// The statusline's pace meter, one window at a time — whichever subscription window the latest
/// `QuotaPace` events say is closest to exhaustion (mesh-routing.md). Plain data pushed in
/// by the core (which owns the store + the pure `compute_quota_pace`); the TUI only renders it.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaPaceInfo {
    pub provider: String,
    pub window: String,
    /// Fraction of the window consumed per hour, at the observed rate.
    pub rate_per_hour: f64,
    /// Fraction of the window projected to be used at its reset time, if known.
    pub projected_fraction_at_reset: Option<f64>,
    pub exhaustion_warning: bool,
}

/// One subscription's quota row in the `/mesh` inspector.
#[derive(Debug, Default, Clone)]
pub struct MeshQuotaRow {
    pub provider: String,
    /// Window fraction consumed (0.0–1.0).
    pub fraction: f64,
    pub plan: String,
    /// "Ok" / "Warning" / "Exhausted".
    pub status: String,
    /// Probability a complex task spreads off this subscription (0.0–1.0). Pace-projected
    /// (mesh-routing.md) — matches what real routing uses, not just `fraction` above.
    pub spread_complex: f64,
    /// Fraction the window is projected to reach by its reset time, if derivable. `None` when
    /// there isn't enough `quota_history` yet.
    pub projected_fraction_at_reset: Option<f64>,
    /// True when that projection would exceed the window before it resets.
    pub exhaustion_warning: bool,
}

/// One scored candidate row in the `/mesh` inspector.
#[derive(Debug, Default, Clone)]
pub struct MeshCandRow {
    pub rank: usize,
    pub model: String,
    pub score: f64,
    /// "free" / "subscription" / "paid".
    pub cost_tag: String,
    pub frontier: bool,
    pub usable: bool,
    pub selected: bool,
    /// Conservation demotion applied (0.0 = none).
    pub penalty: f64,
}

/// Data for the `/mesh` overlay — a legible, animated trace of one routing decision (or the
/// per-tier overview when no prompt is given). Populated by the binary from the mesh's
/// RoutingExplanation engine; the TUI only renders the plain fields.
#[derive(Debug, Default, Clone)]
pub struct MeshOverlay {
    pub open: bool,
    /// True while bridge stats + routing explanation are loading in the background.
    pub loading: bool,
    /// The explained prompt ("" = overview mode).
    pub prompt: String,
    pub classified: String,
    /// Human-readable classifier label: "heuristic" / "llm (model)" / "hybrid — …".
    pub classifier: String,
    pub routed: String,
    pub code_heavy: bool,
    pub reasons: String,
    /// Pre-rendered conservation verdict line.
    pub conserve_line: String,
    pub conserve_fired: bool,
    pub quota: Vec<MeshQuotaRow>,
    pub candidates: Vec<MeshCandRow>,
    pub pick: String,
    pub fallbacks: Vec<String>,
    pub rationale: String,
    /// Animation tick — drives the bar-fill ease and the row-by-row candidate reveal. Stops
    /// advancing once the reveal settles (so the spinner doesn't spin forever).
    pub anim_tick: u32,
    /// Index into `candidates` of the row ↑/↓ highlights (browsing, independent of `selected` —
    /// the routed pick — which never moves). Clamped to `candidates.len() - 1` at render time.
    /// The viewport scroll offset needed to keep this visible is derived at render time (render
    /// takes `&App`, not `&mut App`, so it can't persist a scroll field across frames itself).
    pub cursor: usize,
}

impl MeshOverlay {
    /// The tick at which the open animation is fully settled (bars eased + every candidate row
    /// revealed). Past this the inspector is static — no more redraws, no infinite spinner.
    pub fn settle_tick(&self) -> u32 {
        self.candidates.len() as u32 * 2 + 12
    }
}
