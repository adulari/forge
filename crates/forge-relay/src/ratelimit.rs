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

    /// Applies the keyed limits, then atomically reserves one global send slot. Reserving before
    /// the network call makes the cap a hard ceiling even when many requests race concurrently.
    /// A transport failure still consumes its slot conservatively. Keeping the atomic reservation
    /// last also avoids a reset/release race that could under-count the new day's sends.
    pub(crate) fn check_and_reserve(
        &self,
        ip: &str,
        device_token: &str,
    ) -> Result<(), RejectReason> {
        if self.per_ip.check_key(&ip.to_string()).is_err() {
            return Err(RejectReason::Ip);
        }
        if self.per_token.check_key(&device_token.to_string()).is_err() {
            return Err(RejectReason::DeviceToken);
        }
        self.daily_sent
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sent| {
                (sent < self.daily_cap).then_some(sent + 1)
            })
            .map(|_| ())
            .map_err(|_| RejectReason::DailyCap)
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
            // Governor's keyed stores otherwise retain every unique attacker-supplied IP/token
            // forever. Entries whose quotas have fully replenished are safe to discard.
            limiters.per_ip.retain_recent();
            limiters.per_token.retain_recent();
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
            assert!(limiters.check_and_reserve("1.2.3.4", "tokA").is_ok());
        }
        // 4th call within the window from the same IP (different token, so it's the IP limiter
        // that must be the one rejecting) should fail.
        assert_eq!(
            limiters.check_and_reserve("1.2.3.4", "tokB"),
            Err(RejectReason::Ip)
        );
    }

    #[test]
    fn per_token_allows_burst_then_rejects() {
        let limiters = RateLimiters::new(3, 60, 1_000_000);
        for i in 0..3 {
            assert!(limiters
                .check_and_reserve(&format!("1.2.3.{i}"), "tokX")
                .is_ok());
        }
        assert_eq!(
            limiters.check_and_reserve("9.9.9.9", "tokX"),
            Err(RejectReason::DeviceToken)
        );
    }

    #[test]
    fn daily_cap_rejects_once_reached() {
        let limiters = RateLimiters::new(1000, 60, 2);
        assert!(limiters.check_and_reserve("1.1.1.1", "tokA").is_ok());
        assert!(limiters.check_and_reserve("2.2.2.2", "tokB").is_ok());
        assert_eq!(
            limiters.check_and_reserve("3.3.3.3", "tokC"),
            Err(RejectReason::DailyCap)
        );
    }

    #[test]
    fn daily_cap_is_atomic_under_concurrency() {
        let limiters = Arc::new(RateLimiters::new(10_000, 1, 8));
        let accepted = Arc::new(AtomicU64::new(0));
        std::thread::scope(|scope| {
            for i in 0..64 {
                let limiters = limiters.clone();
                let accepted = accepted.clone();
                scope.spawn(move || {
                    if limiters
                        .check_and_reserve(&format!("10.0.0.{i}"), &format!("tok-{i}"))
                        .is_ok()
                    {
                        accepted.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });
        assert_eq!(accepted.load(Ordering::Relaxed), 8);
        assert_eq!(limiters.daily_sent_count(), 8);
    }
}
