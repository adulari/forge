//! Waiting out a provider outage that outlasts the hot backoff.
//!
//! The pinned backoff (`mesh.pin_outage_wait_secs`, ten minutes) is sized for a blip. An overnight
//! run needs more: when the provider is still down after it, or when every model of an unpinned
//! chain failed as an outage, the turn keeps re-sending its own request on a slow schedule until
//! `mesh.outage_wait_secs` has passed. Re-sending the real request is the probe: `no_capacity` is
//! per model, so a cheaper health ping could read healthy while the turn still could not run, and
//! the same transcript resumes at the same step with nothing to reconstruct.
//!
//! The wait is a `tokio::time::sleep` inside the turn task, so Esc and the daemon's interrupt
//! cancel it by dropping the task, exactly as they cancel the hot backoff.

use super::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

const BASE_WAIT: Duration = Duration::from_secs(15);
const MAX_WAIT: Duration = Duration::from_secs(300);

/// One continuous outage, shared by every pass of the turn that meets it.
pub(crate) struct OutageEpisode {
    since: Instant,
    since_wall: SystemTime,
    until: Instant,
    attempts: u32,
    reason: String,
}

/// An error that waiting can cure. Never a permanent failure, a rate limit (which has its own
/// schedule) or a context overflow (waiting cannot shrink the input).
pub(crate) fn is_outage(err: &forge_provider::ProviderError) -> bool {
    matches!(
        err,
        forge_provider::ProviderError::Overloaded { .. }
            | forge_provider::ProviderError::Unavailable(_)
    ) && !err.is_context_overflow()
}

/// How long to sleep before probe number `attempt` (1-based): the server's `Retry-After` clamped
/// to 15s..=300s, else 15s doubling to a 300s cap with ±20% jitter (`jitter` in 0..=1). The cap
/// holds after jitter, so no sleep is ever longer than five minutes.
pub(crate) fn delay(attempt: u32, retry_after: Option<Duration>, jitter: f64) -> Duration {
    if let Some(ra) = retry_after {
        return ra.clamp(BASE_WAIT, MAX_WAIT);
    }
    let doublings = attempt.saturating_sub(1).min(8);
    let base = BASE_WAIT.saturating_mul(1 << doublings).min(MAX_WAIT);
    let factor = 0.8 + 0.4 * jitter.clamp(0.0, 1.0);
    base.mul_f64(factor).min(MAX_WAIT)
}

/// The instant the wait ends: the configured allowance from when the outage began, cut short by
/// the run budget and the turn deadline so a bounded run or a benchmark never waits past its own
/// limit.
pub(crate) fn horizon(
    now: Instant,
    since: Instant,
    allowance: Duration,
    run_deadline: Option<std::time::Instant>,
    turn_deadline: Option<std::time::Instant>,
) -> Instant {
    let mut until = since + allowance;
    for deadline in [run_deadline, turn_deadline].into_iter().flatten() {
        until = until.min(now + deadline.saturating_duration_since(std::time::Instant::now()));
    }
    until
}

fn unix_secs(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn clock(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%H:%M")
        .to_string()
}

/// `4h09m`, `12m`, `45s`: a span for a status note.
pub fn span(d: Duration) -> String {
    let secs = d.as_secs();
    match (secs / 3600, secs / 60 % 60) {
        (0, 0) => format!("{secs}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h{m:02}m"),
    }
}

fn reason_of(err: &forge_provider::ProviderError) -> String {
    match err {
        forge_provider::ProviderError::Overloaded { status, reason, .. } => {
            format!("{status} {reason}")
        }
        forge_provider::ProviderError::Unavailable(_) if err.is_unreachable() => {
            "unreachable".to_string()
        }
        forge_provider::ProviderError::Unavailable(message) => message
            .chars()
            .take(60)
            .collect::<String>()
            .trim()
            .to_string(),
        other => other.to_string(),
    }
}

impl Session {
    /// Called when a model request failed as an outage and the hot backoff has nothing left.
    /// `already_down` is how long the outage has already run (the hot phase). Sleeps until the
    /// next probe and returns true, or returns false when waiting is disabled, not applicable,
    /// or the horizon has passed (see [`Session::provider_down_error`] for the exit).
    pub(crate) async fn wait_out_outage(
        &mut self,
        model: &str,
        err: &forge_provider::ProviderError,
        already_down: Duration,
    ) -> bool {
        let allowance = Duration::from_secs(self.config.mesh.outage_wait_secs);
        if allowance.is_zero() || !is_outage(err) {
            return false;
        }
        let now = Instant::now();
        if self.outage.is_none() {
            let since = now.checked_sub(already_down).unwrap_or(now);
            let until = horizon(
                now,
                since,
                allowance,
                self.run_budget.and_then(|budget| budget.deadline),
                self.turn_deadline,
            );
            self.outage = Some(OutageEpisode {
                since,
                since_wall: SystemTime::now()
                    .checked_sub(already_down)
                    .unwrap_or_else(SystemTime::now),
                until,
                attempts: 0,
                reason: reason_of(err),
            });
            self.announce_outage(model);
        }
        let Some(episode) = self.outage.as_mut() else {
            return false;
        };
        episode.reason = reason_of(err);
        let remaining = episode.until.saturating_duration_since(now);
        if remaining.is_zero() {
            return false;
        }
        episode.attempts += 1;
        let wait = delay(episode.attempts, err.retry_after(), wall_clock_jitter()).min(remaining);
        let next_try = SystemTime::now() + wait;
        let event = PresenterEvent::ProviderOutage {
            model: model.to_string(),
            since: unix_secs(episode.since_wall),
            next_try: unix_secs(next_try),
            attempt: episode.attempts,
            until: unix_secs(episode.since_wall + (episode.until - episode.since)),
            reason: episode.reason.clone(),
        };
        tracing::warn!(%model, attempt = episode.attempts, wait_secs = wait.as_secs(), error = %err, "provider outage: waiting to re-send the request");
        self.presenter.emit(event);
        self.presenter.emit(PresenterEvent::ModelSearch {
            model: model.to_string(),
            retrying: true,
        });
        tokio::time::sleep(wait).await;
        true
    }

    fn announce_outage(&mut self, model: &str) {
        let Some(episode) = self.outage.as_ref() else {
            return;
        };
        let until = episode.since_wall + (episode.until - episode.since);
        let text = format!(
            "{}{model}: provider down since {} ({}), retrying every <=5 min until {}. Esc to stop.",
            forge_store::OUTAGE_ENTER_MARK,
            clock(episode.since_wall),
            episode.reason,
            clock(until),
        );
        self.record_outage_note(&text);
    }

    fn record_outage_note(&mut self, text: &str) {
        self.presenter
            .emit(PresenterEvent::Warning(text.to_string()));
        let seq = self.next_seq();
        let _ = self.store.add_ui_note(&self.id, seq, Role::System, text);
        self.transcript.push(Message::system(text).ui_only());
    }

    /// The model answered: close the episode, if one is open.
    pub(crate) fn outage_recovered(&mut self, model: &str) {
        let Some(episode) = self.outage.take() else {
            return;
        };
        let down = Instant::now().saturating_duration_since(episode.since);
        self.record_outage_note(&format!(
            "▶ {model} back after {}, turn resumed",
            span(down)
        ));
        self.presenter.emit(PresenterEvent::ProviderRecovered {
            model: model.to_string(),
            down_secs: down.as_secs(),
        });
    }

    /// The error that ends a turn whose provider stayed down for the whole allowance, or `None`
    /// when no wait ever started (the caller keeps its own error).
    pub(crate) fn provider_down_error(
        &mut self,
        model: &str,
        err: &forge_provider::ProviderError,
    ) -> Option<CoreError> {
        let episode = self.outage.take()?;
        let down = Instant::now().saturating_duration_since(episode.since);
        Some(CoreError::ProviderDown {
            model: model.to_string(),
            message: format!(
                "{model}: provider still down after {} — gave up waiting \
                 (mesh.outage_wait_secs). Send `continue` once it is back. Last error: {err}",
                span(down)
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn the_schedule_doubles_from_fifteen_seconds_and_caps_at_five_minutes() {
        let at = |attempt| delay(attempt, None, 0.5).as_secs();
        assert_eq!(
            (1..=8).map(at).collect::<Vec<_>>(),
            [15, 30, 60, 120, 240, 300, 300, 300]
        );
    }

    #[test]
    fn jitter_is_twenty_percent_either_way_and_never_passes_the_cap() {
        assert_eq!(delay(1, None, 0.0), secs(12));
        assert_eq!(delay(1, None, 1.0), secs(18));
        assert_eq!(delay(9, None, 1.0), secs(300));
        assert_eq!(delay(u32::MAX, None, 1.0), secs(300));
    }

    #[test]
    fn retry_after_is_clamped_to_fifteen_through_three_hundred_seconds() {
        let ra = |n| delay(1, Some(secs(n)), 0.5);
        assert_eq!((ra(1), ra(45), ra(86_400)), (secs(15), secs(45), secs(300)));
    }

    #[test]
    fn the_horizon_is_the_earliest_of_the_allowance_and_both_deadlines() {
        let now = Instant::now();
        let soon = std::time::Instant::now() + secs(100);
        let later = std::time::Instant::now() + secs(5_000);
        let at = |run, turn| horizon(now, now, secs(1_000), run, turn).duration_since(now);
        assert_eq!(at(None, None), secs(1_000));
        assert!(at(Some(soon), None) <= secs(100));
        assert!(at(None, Some(soon)) <= secs(100));
        assert_eq!(at(Some(later), Some(later)), secs(1_000));
    }

    #[test]
    fn only_outages_are_waited_for() {
        use forge_provider::ProviderError as E;
        let busy = E::Overloaded {
            status: 503,
            reason: "no_capacity".into(),
            message: String::new(),
            retry_after: None,
        };
        assert!(is_outage(&busy));
        assert!(is_outage(&E::Unavailable("could not connect".into())));
        assert!(!is_outage(&E::Auth("bad key".into())));
        assert!(!is_outage(&E::Capability("no tools".into())));
        assert!(!is_outage(&E::RateLimited {
            message: String::new(),
            retry_after: None
        }));
        assert!(!is_outage(&E::Unavailable(
            "maximum context length exceeded".into()
        )));
    }

    #[test]
    fn spans_read_like_the_note() {
        assert_eq!(span(secs(45)), "45s");
        assert_eq!(span(secs(12 * 60 + 5)), "12m");
        assert_eq!(span(secs(4 * 3600 + 9 * 60 + 30)), "4h09m");
    }
}
