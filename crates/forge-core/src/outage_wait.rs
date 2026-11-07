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

/// One subagent's wait through a provider outage: the same schedule and allowance as the parent
/// turn, bounded by the parent's run and turn deadlines so a spawned agent never outlives them.
pub(crate) struct ChildOutage {
    until: Instant,
    attempts: u32,
}

impl ChildOutage {
    pub(crate) fn begin(allowance: Duration, deadline: Option<std::time::Instant>) -> Self {
        let now = Instant::now();
        Self {
            until: horizon(now, now, allowance, deadline, None),
            attempts: 0,
        }
    }

    /// How long to sleep before the next probe, or `None` once the horizon has passed.
    pub(crate) fn next_wait(&mut self, err: &forge_provider::ProviderError) -> Option<Duration> {
        let remaining = self.until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        self.attempts += 1;
        Some(delay(self.attempts, err.retry_after(), wall_clock_jitter()).min(remaining))
    }
}

impl Session {
    /// The earlier of the run budget's deadline and the turn deadline: what a child's outage
    /// wait must not outlast.
    pub(crate) fn outage_deadline(&self) -> Option<std::time::Instant> {
        [
            self.run_budget.and_then(|budget| budget.deadline),
            self.turn_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

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
            self.open_episode(model, reason_of(err), already_down, allowance);
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
        tracing::warn!(%model, attempt = episode.attempts, wait_secs = wait.as_secs(), error = %err, "provider outage: waiting to re-send the request");
        self.sleep_probe(model, wait).await;
        true
    }

    /// The turn-start router found nothing routable. When that is only because every candidate is
    /// benched for a transient reason (an outage or overload failed it moments ago), sleep until
    /// the soonest of those benches ends (at most five minutes at a time) under the same
    /// `mesh.outage_wait_secs` horizon, so a `continue` sent while the provider is still down
    /// waits for it instead of ending the turn at once. Returns true after a sleep, and the caller
    /// routes again; false when waiting does not apply (disabled, the candidate is not benched, or
    /// the benches are permanent exclusions or belong to providers without a key) or the horizon
    /// has passed.
    pub(crate) async fn wait_out_benches(&mut self, candidate: &str) -> bool {
        let allowance = Duration::from_secs(self.config.mesh.outage_wait_secs);
        if allowance.is_zero() {
            return false;
        }
        let pin = self.effective_pin();
        let rows = self.store.current_benched_report().unwrap_or_default();
        let transient: Vec<_> = rows
            .iter()
            .filter(|(model, _, reason)| {
                !reason.starts_with("excluded:")
                    && forge_config::has_api_key(forge_config::provider_of(model))
                    && !forge_config::is_model_disabled(model, &self.config.mesh.disabled)
                    && pin.as_ref().is_none_or(|p| p.iter().any(|x| x == model))
            })
            .collect();
        let Some((_, _, reason)) = transient.iter().find(|(model, _, _)| model == candidate) else {
            return false;
        };
        let soonest = transient
            .iter()
            .map(|(_, until, _)| *until)
            .min()
            .unwrap_or(0);
        let reason = format!(
            "benched: {}",
            reason.chars().take(50).collect::<String>().trim()
        );
        let now = Instant::now();
        if self.outage.is_none() {
            self.open_episode(candidate, reason, Duration::ZERO, allowance);
        }
        let Some(episode) = self.outage.as_mut() else {
            return false;
        };
        let remaining = episode.until.saturating_duration_since(now);
        if remaining.is_zero() {
            return false;
        }
        episode.attempts += 1;
        let until_bench = Duration::from_secs(
            u64::try_from(soonest - chrono::Utc::now().timestamp()).unwrap_or(0) + 1,
        );
        let wait = until_bench.min(MAX_WAIT).min(remaining);
        tracing::warn!(%candidate, wait_secs = wait.as_secs(), "every candidate is benched: waiting for the soonest bench to end");
        self.sleep_probe(candidate, wait).await;
        true
    }

    fn open_episode(
        &mut self,
        model: &str,
        reason: String,
        already_down: Duration,
        allowance: Duration,
    ) {
        let now = Instant::now();
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
            reason,
        });
        self.announce_outage(model);
    }

    async fn sleep_probe(&mut self, model: &str, wait: Duration) {
        let Some(episode) = self.outage.as_ref() else {
            return;
        };
        let next_try = SystemTime::now() + wait;
        let event = PresenterEvent::ProviderOutage {
            model: model.to_string(),
            since: unix_secs(episode.since_wall),
            next_try: unix_secs(next_try),
            attempt: episode.attempts,
            until: unix_secs(episode.since_wall + (episode.until - episode.since)),
            reason: episode.reason.clone(),
        };
        self.presenter.emit(event);
        self.presenter.emit(PresenterEvent::ModelSearch {
            model: model.to_string(),
            retrying: true,
        });
        tokio::time::sleep(wait).await;
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

    #[tokio::test(start_paused = true)]
    async fn a_child_wait_stops_at_the_allowance_and_at_the_parent_deadline() {
        use forge_provider::ProviderError as E;
        let down = E::Unavailable("could not connect".into());
        let mut within = ChildOutage::begin(secs(100), None);
        let first = within.next_wait(&down).expect("inside the allowance");
        assert!(first <= secs(100));
        tokio::time::sleep(secs(101)).await;
        assert!(within.next_wait(&down).is_none());

        let soon = std::time::Instant::now() + secs(20);
        let mut bounded = ChildOutage::begin(secs(8 * 3600), Some(soon));
        assert!(bounded.next_wait(&down).expect("before the deadline") <= secs(20));
    }

    #[test]
    fn spans_read_like_the_note() {
        assert_eq!(span(secs(45)), "45s");
        assert_eq!(span(secs(12 * 60 + 5)), "12m");
        assert_eq!(span(secs(4 * 3600 + 9 * 60 + 30)), "4h09m");
    }
}
