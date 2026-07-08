//! Abuse prevention (ADR-0012 §4): per-IP and per-device-token token-bucket limiting via
//! `governor`, plus a global daily send-cap circuit breaker. There is no existing precedent for
//! this anywhere in the forge workspace — this module is new territory, sized to the relay's
//! actual (narrow) risk: an abuser can only burn the operator's own Apple push quota against one
//! allowlisted topic, never reach arbitrary devices or see any session/code/account data.

use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

type KeyedLimiter = RateLimiter<
    String,
    governor::state::keyed::DefaultKeyedStateStore<String>,
    governor::clock::DefaultClock,
>;

/// Per-IP and per-device-token throttles, plus the daily circuit breaker. One instance lives for
/// the process lifetime (`Arc`-shared into the axum router state).
pub(crate) struct RateLimiters {
    per_ip: KeyedLimiter,
    per_token: KeyedLimiter,
    daily_sent: AtomicU64,
    daily_cap: u64,
}

/// Why a request was rejected — distinct reasons so the HTTP handler can log/respond precisely
/// rather than a single opaque "rate limited."
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RejectReason {
    Ip,
    DeviceToken,
    DailyCap,
}

fn quota_for(limit_per_window: u32, window_secs: u64) -> Quota {
    let burst = NonZeroU32::new(limit_per_window.max(1)).unwrap();
    let period = Duration::from_secs(window_secs.max(1)) / burst.get();
    Quota::with_period(period)
        .expect("non-zero period")
        .allow_burst(burst)
}

impl RateLimiters {
    pub(crate) fn new(limit_per_window: u32, window_secs: u64, daily_cap: u64) -> Self {
        let quota = quota_for(limit_per_window, window_secs);
        Self {
            per_ip: RateLimiter::keyed(quota),
            per_token: RateLimiter::keyed(quota),
            daily_sent: AtomicU64::new(0),
            daily_cap,
        }
    }

    /// Checks the daily cap first (cheapest, and the one that matters most under sustained
    /// abuse), then per-IP, then per-device-token — first failure wins. Does NOT increment the
    /// daily counter itself; call [`Self::record_sent`] only after the upstream Apple call
    /// actually goes out, so a request rejected by the allowlist/validation (which never reaches
    /// Apple) doesn't consume daily budget.
    pub(crate) fn check(&self, ip: &str, device_token: &str) -> Result<(), RejectReason> {
        if self.daily_sent.load(Ordering::Relaxed) >= self.daily_cap {
            return Err(RejectReason::DailyCap);
        }
        if self.per_ip.check_key(&ip.to_string()).is_err() {
            return Err(RejectReason::Ip);
        }
        if self.per_token.check_key(&device_token.to_string()).is_err() {
            return Err(RejectReason::DeviceToken);
        }
        Ok(())
    }

    pub(crate) fn record_sent(&self) {
        self.daily_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn daily_sent_count(&self) -> u64 {
        self.daily_sent.load(Ordering::Relaxed)
    }
}

/// Resets the daily counter every 24h. Deliberately simple (process-uptime-relative, not
/// midnight-UTC-aligned) — good enough for a circuit breaker whose job is "never let unbounded
/// abuse run forever," not precise billing-period accounting.
pub(crate) fn spawn_daily_reset(limiters: Arc<RateLimiters>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
            let prev = limiters.daily_sent.swap(0, Ordering::Relaxed);
            tracing::info!("daily send cap reset (was {prev}/{})", limiters.daily_cap);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_ip_allows_burst_then_rejects() {
        let limiters = RateLimiters::new(3, 60, 1_000_000);
        for _ in 0..3 {
            assert!(limiters.check("1.2.3.4", "tokA").is_ok());
        }
        // 4th call within the window from the same IP (different token, so it's the IP limiter
        // that must be the one rejecting) should fail.
        assert_eq!(limiters.check("1.2.3.4", "tokB"), Err(RejectReason::Ip));
    }

    #[test]
    fn per_token_allows_burst_then_rejects() {
        let limiters = RateLimiters::new(3, 60, 1_000_000);
        for i in 0..3 {
            assert!(limiters.check(&format!("1.2.3.{i}"), "tokX").is_ok());
        }
        assert_eq!(
            limiters.check("9.9.9.9", "tokX"),
            Err(RejectReason::DeviceToken)
        );
    }

    #[test]
    fn daily_cap_rejects_once_reached() {
        let limiters = RateLimiters::new(1000, 60, 2);
        assert!(limiters.check("1.1.1.1", "tokA").is_ok());
        limiters.record_sent();
        assert!(limiters.check("2.2.2.2", "tokB").is_ok());
        limiters.record_sent();
        assert_eq!(
            limiters.check("3.3.3.3", "tokC"),
            Err(RejectReason::DailyCap)
        );
    }
}
