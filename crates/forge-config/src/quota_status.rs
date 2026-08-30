//! Shared subscription-quota status classification (docs/features/mesh-routing.md).
//!
//! A rolling-window usage fraction (0.0–1.0) is classified into a coarse
//! [`forge_types::QuotaStatus`] that the mesh router uses to decide whether a subscription may
//! still serve a turn. Six independent observation sources — the Claude/Codex CLI-bridge event
//! streams, the Codex OAuth HTTP and WebSocket transports, and the `forge models` seeding path —
//! each used to duplicate this threshold as a separate literal. That let them drift, and it let a
//! subscription reporting a fraction past the point where it can actually serve a turn keep
//! reporting `Warning` (a soft down-rank) instead of `Exhausted` (an exclusion). Routing every
//! source through one function is what makes "status and fraction must not disagree"
//! (docs/features/mesh-routing.md) an invariant instead of a hope.

use forge_types::QuotaStatus;

/// Fraction of a rolling window at or above which a subscription is EXCLUDED from routing
/// entirely ([`forge_types::SubscriptionQuota::is_exhausted`]), not merely down-ranked. A score
/// penalty cannot express "this will fail" — only removal from the candidate set can.
///
/// Evidence, not taste (quota-stall incident, 2026-08-30): a codex-cli five-hour window measured
/// at a 0.97 fraction had already stopped serving turns — the bridge process exited 0 with no
/// assistant content, no tool calls, and no error. Under the previous 0.98 gate that observation
/// stayed `Warning`, worth only a 5-point routing penalty (`catalog::route_score`), which a
/// same-tier alternative's capability-score gap routinely exceeds — so the exhausted subscription
/// kept winning complex-tier routing while producing nothing. Setting the gate to the measured
/// failure point (rather than one point past it) ensures a window this full is excluded before it
/// fails a turn, not after.
pub const EXHAUSTED_FRACTION: f64 = 0.97;

/// Fraction at or above which a subscription is down-ranked (still routable as a fallback) but
/// not yet excluded. Unchanged from the pre-incident value — nothing in the incident evidence
/// challenged where the *soft* warning line sits, only where the *hard* exclusion line sits.
pub const WARNING_FRACTION: f64 = 0.80;

/// Classify a rolling-window usage fraction (0.0–1.0) into a [`QuotaStatus`]. Every quota
/// observation source should route through this so a given fraction always means the same thing
/// to the router, regardless of which transport reported it.
pub fn status_from_fraction(fraction: f64) -> QuotaStatus {
    if fraction >= EXHAUSTED_FRACTION {
        QuotaStatus::Exhausted
    } else if fraction >= WARNING_FRACTION {
        QuotaStatus::Warning
    } else {
        QuotaStatus::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries() {
        assert_eq!(status_from_fraction(0.0), QuotaStatus::Ok);
        assert_eq!(status_from_fraction(0.79), QuotaStatus::Ok);
        assert_eq!(status_from_fraction(0.80), QuotaStatus::Warning);
        assert_eq!(status_from_fraction(0.96), QuotaStatus::Warning);
        assert_eq!(status_from_fraction(0.97), QuotaStatus::Exhausted);
        assert_eq!(status_from_fraction(1.0), QuotaStatus::Exhausted);
    }

    /// The exact incident measurement: a codex-cli five-hour window reported at 0.97 must be
    /// excluded, not merely warned about.
    #[test]
    fn measured_incident_fraction_is_exhausted_not_warning() {
        assert_eq!(status_from_fraction(0.97), QuotaStatus::Exhausted);
    }
}
