//! What a pinned turn tells the user, and does, when its model is rate limited.

use super::*;

/// Consecutive token throttles on one request before waiting is judged not to be enough and the
/// context is compacted. The first is treated as a transient burst; a second means the retry just
/// sent the same oversized context again.
pub(crate) const TOKEN_THROTTLES_BEFORE_COMPACTING: u32 = 2;

/// Compactions one request may spend on token throttles. Each one cuts the usable window to 55%,
/// so three leave about a sixth of it; past that the context is not what is being throttled.
pub(crate) const MAX_THROTTLE_COMPACTIONS: u32 = 3;

/// Token throttles seen by one request's retries, and the compactions spent answering them.
#[derive(Default)]
pub(crate) struct TokenThrottles {
    consecutive: u32,
    compactions: u32,
}

impl TokenThrottles {
    /// Record one rate-limit error; true when this retry should compact first. A throttle that is
    /// not about tokens resets the run, so only an unbroken streak counts.
    pub(crate) fn observe(&mut self, err: &forge_provider::ProviderError) -> bool {
        self.consecutive = if err.is_token_throttle() {
            self.consecutive + 1
        } else {
            0
        };
        let compact = self.consecutive >= TOKEN_THROTTLES_BEFORE_COMPACTING
            && self.compactions < MAX_THROTTLE_COMPACTIONS;
        self.compactions += u32::from(compact);
        compact
    }
}

/// Tell the user a pinned model is being retried after a rate limit, compacting first when the
/// throttle is on tokens and has repeated. `(delay, attempt, max_attempts)` describe the backoff.
pub(crate) async fn announce_backoff(
    session: &mut Session,
    active_model: &str,
    err: &forge_provider::ProviderError,
    (delay, attempt, max_attempts): (std::time::Duration, u32, u32),
    compacting: bool,
) {
    tracing::warn!(model = %active_model, attempt, error = %err, "pinned model rate limited");
    let label = rate_limited_label(active_model, err);
    let secs = delay.as_secs().max(1);
    let warning = if compacting {
        format!(
            "{active_model}: {label} — repeated token throttles, so the context was compacted; \
             retrying pinned model in {secs}s (attempt {attempt}/{max_attempts}); {}",
            quota_hint(active_model)
        )
    } else {
        format!(
            "{active_model}: {label} — retrying pinned model in {secs}s \
             (attempt {attempt}/{max_attempts})"
        )
    };
    session.presenter.emit(PresenterEvent::Warning(warning));
    if compacting {
        shrink_context(session, active_model).await;
    }
}

/// Most retries a pinned turn spends on an overloaded provider, whatever the budget says: a
/// gateway answering `Retry-After: 1` would otherwise allow hundreds of attempts.
pub(crate) const OVERLOAD_MAX_ATTEMPTS: u32 = 40;
const OVERLOAD_MIN_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
const OVERLOAD_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// How long to wait before retrying an overloaded provider: its `Retry-After` clamped to
/// 1s..=120s (`0` or a huge value is the server's problem, not a schedule), else the blind
/// schedule.
pub(crate) fn overload_delay(
    attempt: u32,
    retry_after: Option<std::time::Duration>,
    jitter: f64,
) -> std::time::Duration {
    match retry_after {
        Some(ra) => ra.clamp(OVERLOAD_MIN_WAIT, OVERLOAD_MAX_WAIT),
        None => crate::pinned_backoff_delay(attempt, None, jitter),
    }
}

/// `2m10s` / `45s` / `10m`: whole seconds, for a status line.
pub(crate) fn short_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    match (secs / 60, secs % 60) {
        (0, s) => format!("{s}s"),
        (m, 0) => format!("{m}m"),
        (m, s) => format!("{m}m{s}s"),
    }
}

/// What a pinned turn tells the user before sleeping on a provider outage:
/// "m: provider busy (503 no provider route), retrying in 20s (attempt 3, 2m10s of 10m budget
/// used)". A wait of a few seconds or more is always announced, so an unattended run's
/// stream-json shows why it is quiet; shorter ones are shown on the first retry and every fifth,
/// so a `Retry-After: 1` gateway does not flood the chat. The provider's whole answer goes to the
/// log.
pub(crate) fn announce_outage_wait(
    session: &mut Session,
    active_model: &str,
    err: &forge_provider::ProviderError,
    wait: OutageWait,
) {
    tracing::warn!(model = %active_model, attempt = wait.attempt, error = %err, "pinned model provider unavailable");
    let long = wait.delay >= std::time::Duration::from_secs(3);
    if !long && wait.attempt != 1 && !wait.attempt.is_multiple_of(5) {
        return;
    }
    let what = match err {
        forge_provider::ProviderError::Overloaded { status, reason, .. } => {
            format!("provider busy ({status} {reason})")
        }
        forge_provider::ProviderError::Unavailable(message) => {
            let message: String = message.chars().take(90).collect();
            format!("provider unreachable ({})", message.trim())
        }
        other => other.to_string(),
    };
    session.presenter.emit(PresenterEvent::Warning(format!(
        "{active_model}: {what}, retrying in {}s (attempt {}, {} of {} budget used)",
        wait.delay.as_secs().max(1),
        wait.attempt,
        short_duration(wait.waited),
        short_duration(wait.budget),
    )));
}

/// One backoff step of a pinned outage wait: how long this sleep is, which attempt it is, and how
/// much of the budget the sleeps so far (including this one) have spent.
#[derive(Clone, Copy)]
pub(crate) struct OutageWait {
    pub(crate) delay: std::time::Duration,
    pub(crate) attempt: u32,
    pub(crate) waited: std::time::Duration,
    pub(crate) budget: std::time::Duration,
}

#[cfg(test)]
mod overload_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn retry_after_is_clamped_to_a_sane_range() {
        let d = |ra: Option<u64>| overload_delay(1, ra.map(Duration::from_secs), 0.5);
        assert_eq!(d(Some(0)), Duration::from_secs(1));
        assert_eq!(d(Some(1)), Duration::from_secs(1));
        assert_eq!(d(Some(30)), Duration::from_secs(30));
        assert_eq!(d(Some(86_400)), Duration::from_secs(120));
        assert!(
            d(None) >= Duration::from_secs(3),
            "blind schedule when none"
        );
    }

    #[test]
    fn durations_read_as_minutes_and_seconds() {
        let d = |s| short_duration(Duration::from_secs(s));
        assert_eq!(
            (d(45).as_str(), d(130).as_str(), d(600).as_str()),
            ("45s", "2m10s", "10m")
        );
    }

    #[test]
    fn the_attempt_cap_bounds_the_worst_case_regardless_of_budget() {
        let worst = Duration::from_secs(1) * OVERLOAD_MAX_ATTEMPTS;
        assert!(
            worst <= Duration::from_secs(120),
            "{worst:?} of floor-rate retries"
        );
    }
}

/// The provider's reason for a failure, short enough for a status line: `Bedrock: Too many tokens,
/// please wait before trying again.` The full text goes to the log, never the chat.
pub(crate) fn provider_reason(model: &str, err: &forge_provider::ProviderError) -> String {
    const LIMIT: usize = 60;
    let message = match err {
        forge_provider::ProviderError::RateLimited { message, .. } => message.trim(),
        _ => return String::new(),
    };
    let message = message
        .strip_prefix("rate limited:")
        .unwrap_or(message)
        .trim();
    if message.is_empty() {
        return String::new();
    }
    let shown = if message.chars().count() <= LIMIT {
        message.to_string()
    } else {
        let cut: String = message.chars().take(LIMIT).collect();
        let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
        format!("{}…", cut.trim_end_matches([',', ';', ':', '.']))
    };
    match provider_label(model) {
        label if label.is_empty() => shown,
        label => format!("{label}: {shown}"),
    }
}

/// `bedrock::global.moonshotai.kimi-k3` -> `Bedrock`; anything else keeps its provider prefix.
fn provider_label(model: &str) -> String {
    match forge_config::provider_of(model) {
        "bedrock" => "Bedrock".to_string(),
        other => other.to_string(),
    }
}

/// `rate limited (Bedrock: Too many tokens, please wait…)`, or plain `rate limited` when the
/// provider gave no reason.
pub(crate) fn rate_limited_label(model: &str, err: &forge_provider::ProviderError) -> String {
    let reason = provider_reason(model, err);
    if reason.is_empty() {
        "rate limited".to_string()
    } else {
        format!("rate limited ({reason})")
    }
}

/// The advice that goes with a compaction done for a token throttle. A retry cannot cure a quota
/// smaller than one request, so the remedy that does is named.
pub(crate) fn quota_hint(model: &str) -> String {
    if forge_config::provider_of(model) == "bedrock" {
        "if this keeps happening, request a higher Bedrock tokens-per-minute quota for this model \
         (AWS Service Quotas)"
            .to_string()
    } else {
        "if this keeps happening, ask the provider for a higher tokens-per-minute quota".to_string()
    }
}

/// Shrink what the next request sends: cap the model's usable window at 55% of what it was, then
/// compact the transcript. Non-destructive and convergent — the stored transcript is untouched and
/// each call multiplies the cap down — so the caller may repeat it a bounded number of times.
pub(crate) async fn shrink_context(session: &mut Session, active_model: &str) {
    let shrunk = (session.effective_context_window(active_model) as u64 * 55 / 100).max(1) as u32;
    session.overflow_window_cap = Some((active_model.to_string(), shrunk));
    // Best-effort LLM compaction: a cleaner summary when the summarize call itself fits. The window
    // cap above is the guarantee that the retry shrinks whether or not compaction runs.
    let _ = session.compact(true).await;
    session.emit_context_gauge(active_model);
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_provider::ProviderError;

    fn limited(message: &str) -> ProviderError {
        ProviderError::RateLimited {
            message: message.into(),
            retry_after: None,
        }
    }

    #[test]
    fn the_label_carries_a_short_provider_reason() {
        let e = limited("Too many tokens, please wait before trying again.");
        assert_eq!(
            rate_limited_label("bedrock::global.moonshotai.kimi-k3", &e),
            "rate limited (Bedrock: Too many tokens, please wait before trying again.)"
        );
    }

    #[test]
    fn a_long_reason_is_cut_at_a_word_with_an_ellipsis() {
        let e = limited(
            "Too many tokens for this account in this region right now, so please wait a while \
             before trying your request again",
        );
        let reason = provider_reason("bedrock::m", &e);
        assert!(reason.starts_with("Bedrock: Too many tokens"), "{reason}");
        assert!(reason.ends_with('…'), "{reason}");
        assert!(reason.chars().count() <= "Bedrock: ".len() + 61, "{reason}");
    }

    #[test]
    fn no_reason_leaves_the_plain_label() {
        assert_eq!(
            rate_limited_label("groq::x", &limited("  ")),
            "rate limited"
        );
        let other = ProviderError::Unavailable("down".into());
        assert_eq!(rate_limited_label("groq::x", &other), "rate limited");
    }

    #[test]
    fn other_providers_keep_their_prefix() {
        let e = limited("slow down");
        assert_eq!(
            rate_limited_label("groq::llama", &e),
            "rate limited (groq: slow down)"
        );
    }

    #[test]
    fn the_hint_names_bedrock_quotas_only_for_bedrock() {
        assert!(quota_hint("bedrock::m").contains("Bedrock tokens-per-minute"));
        assert!(!quota_hint("groq::m").contains("Bedrock"));
    }
}
