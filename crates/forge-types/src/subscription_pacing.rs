//! Pacing a subscription's spend evenly across its quota window.
//!
//! `SubscriptionQuota` (in `lib.rs`) tracks current pressure; this module adds the time term
//! that decides whether that pressure is ahead of schedule.

/// The fraction of a subscription window ordinary pacing may spend. The final quarter is held in
/// reserve and is never made available by the pacing policy.
pub const SUBSCRIPTION_PACE_SPEND_FRACTION: f64 = 0.75;

/// One observed subscription quota window, including enough timing context for pacing.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionWindow {
    pub window: String,
    pub fraction_used: f64,
    /// Earliest observation belonging to this window. Used only when the provider omits reset.
    pub oldest_observed_at: i64,
    /// Provider-authoritative reset instant, when available.
    pub resets_at: Option<i64>,
}

/// The visible result of comparing a subscription window against its linear allowance.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionPacing {
    pub window: String,
    pub fraction_used: f64,
    pub allowed_fraction: f64,
    pub elapsed_secs: i64,
    pub total_secs: i64,
    pub resets_at: Option<i64>,
    /// True when `resets_at` was unavailable and timing began at the oldest observation.
    pub used_nominal_fallback: bool,
}

impl SubscriptionPacing {
    /// Select the most constrained observed window for `now`. Unknown window kinds cannot be
    /// paced safely and are ignored; known quota pressure still follows the existing path.
    pub fn from_windows(windows: &[SubscriptionWindow], now: i64) -> Option<Self> {
        windows
            .iter()
            .filter_map(|window| Self::from_window(window, now))
            .max_by(|a, b| {
                (a.fraction_used - a.allowed_fraction)
                    .total_cmp(&(b.fraction_used - b.allowed_fraction))
            })
    }

    fn from_window(window: &SubscriptionWindow, now: i64) -> Option<Self> {
        let total_secs = nominal_window_secs(&window.window)?;
        let (start, used_nominal_fallback) = match window.resets_at {
            Some(reset) => (reset - total_secs, false),
            None => (window.oldest_observed_at, true),
        };
        let elapsed_secs = (now - start).clamp(0, total_secs);
        let allowed_fraction =
            (elapsed_secs as f64 / total_secs as f64) * SUBSCRIPTION_PACE_SPEND_FRACTION;
        Some(Self {
            window: window.window.clone(),
            fraction_used: window.fraction_used.clamp(0.0, 1.0),
            allowed_fraction,
            elapsed_secs,
            total_secs,
            resets_at: window.resets_at,
            used_nominal_fallback,
        })
    }

    /// Above the linear allowance is over pace. Equality remains within pace.
    pub fn is_over_pace(&self) -> bool {
        self.fraction_used > self.allowed_fraction
    }

    /// Fraction of the window that has elapsed (0.0–1.0) — the "at X% elapsed" half of a pacing
    /// claim, which is meaningless without the used/allowed pair beside it.
    pub fn elapsed_fraction(&self) -> f64 {
        if self.total_secs <= 0 {
            return 0.0;
        }
        self.elapsed_secs as f64 / self.total_secs as f64
    }
}

/// Nominal duration for quota windows with a defined subscription pacing policy.
///
/// `monthly` is a calendar window, so 30 days is only a nominal length: providers that report it
/// (OpenCode Go) always send an authoritative `resets_at`, which [`SubscriptionPacing::from_window`]
/// prefers. The nominal value is the fallback that keeps the window paceable rather than silently
/// ignored when a reset instant is missing.
pub fn nominal_window_secs(window: &str) -> Option<i64> {
    match window {
        "monthly" => Some(30 * 24 * 60 * 60),
        "weekly" => Some(7 * 24 * 60 * 60),
        "five_hour" => Some(5 * 60 * 60),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(
        kind: &str,
        fraction_used: f64,
        oldest_observed_at: i64,
        resets_at: Option<i64>,
    ) -> SubscriptionWindow {
        SubscriptionWindow {
            window: kind.to_string(),
            fraction_used,
            oldest_observed_at,
            resets_at,
        }
    }

    #[test]
    fn pacing_uses_the_real_weekly_reset_and_preserves_the_reserve_boundary() {
        let total = nominal_window_secs("weekly").unwrap();
        let reset = 1_000_000;
        let pacing = SubscriptionPacing::from_windows(
            &[window("weekly", 0.75, reset - total, Some(reset))],
            reset,
        )
        .unwrap();
        assert!((pacing.allowed_fraction - 0.75).abs() < 1e-9);
        assert!(!pacing.is_over_pace(), "exactly 75% is within pace");
        let reserve = SubscriptionPacing::from_windows(
            &[window("weekly", 0.750_001, reset - total, Some(reset))],
            reset,
        )
        .unwrap();
        assert!(
            reserve.is_over_pace(),
            "ordinary pacing cannot enter the 25% reserve"
        );
    }

    #[test]
    fn pacing_falls_back_to_nominal_window_from_oldest_observation() {
        let total = nominal_window_secs("weekly").unwrap();
        let oldest = 10_000;
        let pacing = SubscriptionPacing::from_windows(
            &[window("weekly", 0.37, oldest, None)],
            oldest + 2 * 24 * 60 * 60,
        )
        .unwrap();
        assert!(pacing.used_nominal_fallback);
        assert!((pacing.allowed_fraction - (2.0 / 7.0 * 0.75)).abs() < 1e-9);
        assert!(pacing.is_over_pace());
        assert_eq!(pacing.total_secs, total);
    }

    #[test]
    fn five_hour_pacing_overrides_a_healthy_weekly_window() {
        let now = 100_000;
        let five_hours = nominal_window_secs("five_hour").unwrap();
        let weekly = nominal_window_secs("weekly").unwrap();
        let pacing = SubscriptionPacing::from_windows(
            &[
                window("weekly", 0.20, now - weekly / 2, Some(now + weekly / 2)),
                window(
                    "five_hour",
                    0.90,
                    now - five_hours / 5,
                    Some(now + four_hours(five_hours)),
                ),
            ],
            now,
        )
        .unwrap();
        assert_eq!(pacing.window, "five_hour");
        assert!(pacing.is_over_pace());
    }

    fn four_hours(five_hours: i64) -> i64 {
        five_hours - 60 * 60
    }

    #[test]
    fn monthly_windows_are_paced_against_their_authoritative_reset() {
        let total = nominal_window_secs("monthly").unwrap();
        assert_eq!(total, 30 * 24 * 60 * 60);
        let reset = 2_000_000;
        // Half the month elapsed: linear allowance is half of the 75% spendable fraction.
        let now = reset - total / 2;
        let pacing =
            SubscriptionPacing::from_windows(&[window("monthly", 0.50, 0, Some(reset))], now)
                .unwrap();
        assert_eq!(pacing.window, "monthly");
        assert!(!pacing.used_nominal_fallback);
        assert!((pacing.allowed_fraction - 0.375).abs() < 1e-9);
        assert!(
            pacing.is_over_pace(),
            "50% spent at 37.5% allowed is over pace"
        );
    }

    #[test]
    fn a_pressured_monthly_window_can_outrank_healthy_shorter_windows() {
        // OpenCode Go reports all three windows at once; the most constrained must win, including
        // when that is the new monthly kind.
        let now = 5_000_000;
        let month = nominal_window_secs("monthly").unwrap();
        let week = nominal_window_secs("weekly").unwrap();
        let five = nominal_window_secs("five_hour").unwrap();
        let pacing = SubscriptionPacing::from_windows(
            &[
                window("five_hour", 0.01, now, Some(now + five / 2)),
                window("weekly", 0.05, now, Some(now + week / 2)),
                window("monthly", 0.80, now, Some(now + month / 2)),
            ],
            now,
        )
        .unwrap();
        assert_eq!(pacing.window, "monthly");
        assert!(pacing.is_over_pace());
    }
}
