use super::*;

/// How a subscription is sitting relative to its rolling usage window (quota-aware routing, L3,
/// mesh-routing.md). Ordered so the stricter wins with `.max()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum QuotaStatus {
    /// Comfortably within the window.
    #[default]
    Ok,
    /// Near the window limit (or using overage) — demote the subscription below alternatives.
    Warning,
    /// At/over the limit — skip the subscription entirely (route around it), like a benched model.
    Exhausted,
}

/// One observation of a CLI-bridge subscription's quota, surfaced by the bridge's event stream
/// (e.g. Claude Code's `rate_limit_event`) alongside a completion. Recorded by the store so the
/// router can avoid overrunning a near-limit plan.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaHint {
    /// Bridge provider prefix the quota belongs to (`claude-cli` / `codex-cli`).
    pub provider: String,
    /// The rolling-window kind the provider reported (`five_hour`, `weekly`, …); `""` if unknown.
    pub window: String,
    pub status: QuotaStatus,
    /// Epoch seconds when the window resets, if the provider told us.
    pub resets_at: Option<i64>,
    /// Fraction of the window consumed (0.0–1.0), if the provider told us.
    pub fraction_used: Option<f64>,
}

/// A snapshot of every subscription's current quota pressure, built by the store from the
/// `subscription_usage` table (rows whose window hasn't reset). Consulted by the mesh router to
/// demote or skip a pressured subscription. Carries no clock/I/O (filtering happens at build).
#[derive(Debug, Default, Clone)]
/// Documented in docs/features/mesh-routing.md.
pub struct SubscriptionQuota {
    by_provider: std::collections::HashMap<String, QuotaStatus>,
    /// Fraction (0.0–1.0) of the strictest active window consumed, per provider. Drives the
    /// graduated subscription-conservation spreading (distinct from the coarse `QuotaStatus`).
    fraction: std::collections::HashMap<String, f64>,
    /// Subscription plan slug per provider (`claude-cli` → `max-20x`, `codex-cli` → `plus`), from
    /// config. A larger plan has more headroom, so it is spent more freely.
    plans: std::collections::HashMap<String, String>,
    /// Pace projection per provider (mesh-routing.md), derived from `quota_history` — where
    /// the strictest active window is headed by its reset time, not just its current fraction.
    /// Absent when there isn't enough history to derive a rate (see `compute_quota_pace`).
    pace: std::collections::HashMap<String, QuotaPace>,
    /// Per-window pacing decisions. Unlike `fraction`, this retains both rolling windows so a
    /// short window can constrain routing even when the weekly window remains healthy.
    pacing: std::collections::HashMap<String, SubscriptionPacing>,
    /// Whether proactive subscription-conservation spreading is enabled (config opt-out).
    conserve: bool,
}

/// Maximum age of a Codex account-wide quota observation. Codex usage may be consumed through
/// either Forge's OAuth surface or the official CLI, so older data is not authoritative enough to
/// influence mesh conservation or provider selection.
pub const CODEX_QUOTA_FRESHNESS_SECS: i64 = 5 * 60;

impl SubscriptionQuota {
    pub fn new(by_provider: std::collections::HashMap<String, QuotaStatus>) -> Self {
        Self {
            by_provider,
            ..Default::default()
        }
    }

    /// Attach per-provider usage fractions (the strictest active window).
    pub fn with_fractions(mut self, fraction: std::collections::HashMap<String, f64>) -> Self {
        self.fraction = fraction;
        self
    }

    /// Attach per-provider subscription plan slugs (from `config.mesh.subscriptions`).
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_plans(mut self, plans: std::collections::HashMap<String, String>) -> Self {
        self.plans = plans;
        self
    }

    /// Attach per-provider quota-pace projections (mesh-routing.md).
    pub fn with_paces(mut self, pace: std::collections::HashMap<String, QuotaPace>) -> Self {
        self.pace = pace;
        self
    }

    /// Attach the strictest real-time pacing decision for each subscription provider.
    ///
    /// The store computes this from every active quota window at its authoritative clock, rather
    /// than collapsing windows into the legacy maximum-fraction snapshot.
    pub fn with_pacing(
        mut self,
        pacing: std::collections::HashMap<String, SubscriptionPacing>,
    ) -> Self {
        self.pacing = pacing;
        self
    }

    /// Enable/disable proactive conservation spreading (`config.mesh.subscription_conserve`).
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_conserve(mut self, on: bool) -> Self {
        self.conserve = on;
        self
    }

    /// The pressure for a provider prefix (defaults to `Ok` when unknown/unconstrained).
    /// Documented in docs/features/mesh-routing.md.
    pub fn status_for(&self, provider: &str) -> QuotaStatus {
        self.by_provider
            .get(provider)
            .copied()
            .unwrap_or(QuotaStatus::Ok)
    }

    /// Fraction of the strictest window consumed for routing. Unknown observations deliberately
    /// default to zero pressure: routing must remain available until a provider emits a fresh
    /// server observation. Display code must use [`Self::observed_fraction_for`] instead.
    pub fn fraction_for(&self, provider: &str) -> f64 {
        self.fraction.get(provider).copied().unwrap_or(0.0)
    }

    /// Fraction of the strictest currently observed window, if a fresh observation exists.
    /// Unlike [`Self::fraction_for`], this preserves absence for honest user-facing displays.
    pub fn observed_fraction_for(&self, provider: &str) -> Option<f64> {
        self.fraction.get(provider).copied()
    }

    /// The pace projection for a provider, if one was attached and there was enough quota
    /// history to derive it (see [`compute_quota_pace`]).
    pub fn pace_for(&self, provider: &str) -> Option<QuotaPace> {
        self.pace.get(provider).copied()
    }

    /// The active pacing decision for `provider`, if quota-window observations were available.
    pub fn pacing_for(&self, provider: &str) -> Option<&SubscriptionPacing> {
        self.pacing.get(provider)
    }

    /// The conservation input for a provider (mesh-routing.md): `fraction_for(provider)`
    /// raised to the pace's `projected_fraction_at_reset` (clamped to 1.0) when a pace is present
    /// AND projects HIGHER than the current fraction — a window projected to hit 90% by reset is
    /// treated as if it's already at 90%, so spreading ramps up ahead of the overrun instead of
    /// reacting to it only once it actually crosses a threshold. The projection can only raise,
    /// never lower, the fraction: a cooling-down window (rate has since dropped, so the projection
    /// now reads under the current fraction) still conserves on what's already spent, not on a
    /// stale lower number.
    /// Documented in docs/features/mesh-routing.md.
    pub fn effective_fraction_for(&self, provider: &str) -> f64 {
        let frac = self.fraction_for(provider);
        let projected = self
            .pace_for(provider)
            .and_then(|p| p.projected_fraction_at_reset)
            .unwrap_or(0.0)
            .min(1.0);
        frac.max(projected)
    }

    /// The configured plan slug for a provider (`""` when unset).
    pub fn plan_for(&self, provider: &str) -> &str {
        self.plans.get(provider).map(String::as_str).unwrap_or("")
    }

    /// Providers with a currently constraining or display-worthy quota observation.
    pub fn providers(&self) -> impl Iterator<Item = &str> {
        self.by_provider
            .keys()
            .chain(self.fraction.keys())
            .map(String::as_str)
    }

    /// Whether proactive conservation spreading is enabled.
    pub fn conserve_enabled(&self) -> bool {
        self.conserve
    }

    /// At/over the limit — route around it.
    pub fn is_exhausted(&self, provider: &str) -> bool {
        self.status_for(provider) == QuotaStatus::Exhausted
    }

    /// Near or over the limit — usable but demoted below alternatives.
    pub fn is_pressured(&self, provider: &str) -> bool {
        self.status_for(provider) >= QuotaStatus::Warning
    }

    pub fn is_empty(&self) -> bool {
        self.by_provider.is_empty()
    }
}

/// One historical observation of a subscription window's usage (mesh-routing.md),
/// read back from the store's append-only `quota_history` table. Distinct from [`QuotaHint`]
/// (the latest snapshot only) — a series of these is what lets [`compute_quota_pace`] derive a
/// rate of consumption instead of just a point-in-time fraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuotaHistoryPoint {
    /// Epoch seconds this observation was recorded.
    pub observed_at: i64,
    /// Fraction of the window consumed at `observed_at` (0.0–1.0).
    pub fraction_used: f64,
}

/// A minimum span of wall-clock time between the earliest and latest history point before
/// [`compute_quota_pace`] will derive a rate from them. Two samples seconds apart (e.g. right
/// after a window resets) would otherwise divide by a near-zero denominator and spike into a
/// nonsensical rate/projection — this floor makes "not enough data yet" an explicit `None`
/// instead of a false reading.
pub const QUOTA_PACE_MIN_ELAPSED_SECS: i64 = 300;

/// A lookback window wide enough to cover a `weekly` subscription window's history without
/// pulling the whole `quota_history` table (rows are one per turn's quota hint, so this is small
/// even at 8 days). Shared by the statusline's [`compute_quota_pace`] caller (forge-core's
/// `emit_quota_pace`) and the store's [`SubscriptionQuota`] pace attachment (`quota_at`) so both
/// project off the same history window.
/// Documented in docs/features/mesh-routing.md.
pub const QUOTA_PACE_LOOKBACK_SECS: i64 = 8 * 24 * 3600;

/// A projection of where a subscription window's usage is headed, derived from a short history
/// of `(observed_at, fraction_used)` points by [`compute_quota_pace`]. Pure/deterministic: no
/// clock or I/O, so it's trivially unit-testable — the caller supplies "now".
#[derive(Debug, Clone, Copy, PartialEq)]
/// Documented in docs/features/mesh-routing.md.
pub struct QuotaPace {
    /// Fraction of the window consumed per hour, at the observed rate (>= 0.0).
    pub rate_per_hour: f64,
    /// Fraction of the window consumed per day, at the observed rate (>= 0.0).
    pub rate_per_day: f64,
    /// Fraction of the window projected to be used AT `resets_at`, linearly extrapolating the
    /// observed rate. `None` when the window's reset time isn't known (nothing to project to).
    pub projected_fraction_at_reset: Option<f64>,
    /// Seconds until the window would hit 100% at the current rate. `None` when the rate is
    /// zero/negative (usage isn't climbing, so it will never reach 100% on its own).
    pub time_to_exhaustion_secs: Option<f64>,
    /// True when the projected usage will exceed the window before it resets — i.e.
    /// `time_to_exhaustion_secs < time remaining in the window`. The signal the statusline/mesh
    /// use to warn ahead of an overrun, not just react to one already at `QuotaStatus::Warning`.
    pub exhaustion_warning: bool,
}

/// Derive a [`QuotaPace`] from a subscription window's usage history. `history` need not be
/// sorted; `resets_at` is the window's known reset time (epoch secs, if any); `now` is the
/// caller-supplied clock (epoch secs) — this function performs no I/O and reads no clock itself.
///
/// Returns `None` when there isn't enough data to derive a rate: fewer than two points, or the
/// earliest and latest point are within [`QUOTA_PACE_MIN_ELAPSED_SECS`] of each other (guards the
/// "just reset" near-zero-denominator case described on [`QUOTA_PACE_MIN_ELAPSED_SECS`]).
pub fn compute_quota_pace(
    history: &[QuotaHistoryPoint],
    resets_at: Option<i64>,
    now: i64,
) -> Option<QuotaPace> {
    if history.len() < 2 {
        return None;
    }
    let mut sorted: Vec<QuotaHistoryPoint> = history.to_vec();
    sorted.sort_by_key(|p| p.observed_at);
    let earliest = sorted.first().copied()?;
    let latest = sorted.last().copied()?;

    let elapsed_secs = latest.observed_at - earliest.observed_at;
    if elapsed_secs < QUOTA_PACE_MIN_ELAPSED_SECS {
        return None;
    }

    // A window rollover mid-history would show fraction_used dropping; clamp to 0 rather than
    // report a negative rate — there's nothing sensible to project from a reset in-range.
    let delta_fraction = (latest.fraction_used - earliest.fraction_used).max(0.0);
    let rate_per_sec = delta_fraction / elapsed_secs as f64;
    let rate_per_hour = rate_per_sec * 3600.0;
    let rate_per_day = rate_per_sec * 86_400.0;

    let time_to_exhaustion_secs = if rate_per_sec > 0.0 && latest.fraction_used < 1.0 {
        Some((1.0 - latest.fraction_used) / rate_per_sec)
    } else if latest.fraction_used >= 1.0 {
        Some(0.0)
    } else {
        None
    };

    let projected_fraction_at_reset = resets_at.map(|r| {
        let remaining = (r - now).max(0) as f64;
        latest.fraction_used + rate_per_sec * remaining
    });

    let exhaustion_warning = match (resets_at, time_to_exhaustion_secs) {
        (Some(r), Some(t)) => {
            let remaining = (r - now).max(0) as f64;
            t < remaining
        }
        _ => false,
    };

    Some(QuotaPace {
        rate_per_hour,
        rate_per_day,
        projected_fraction_at_reset,
        time_to_exhaustion_secs,
        exhaustion_warning,
    })
}
