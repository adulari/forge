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
