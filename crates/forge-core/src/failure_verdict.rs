//! Why a turn ran out of models, in words a person can act on.
//!
//! Split out of [`crate::model_request`]: the failover loop owns dispatch and retry policy, while
//! this owns the classification of a failed attempt and the verdict the user finally reads.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptFailure {
    NoCredentials,
    RateLimited,
    Capability,
    Network,
    Down,
}

pub(crate) fn classify_attempt_failure(error: &forge_provider::ProviderError) -> AttemptFailure {
    match error {
        forge_provider::ProviderError::Auth(_) => AttemptFailure::NoCredentials,
        forge_provider::ProviderError::RateLimited { .. } => AttemptFailure::RateLimited,
        forge_provider::ProviderError::Capability(_)
        | forge_provider::ProviderError::NoModelAccess(_)
        | forge_provider::ProviderError::Request(_) => AttemptFailure::Capability,
        forge_provider::ProviderError::Unavailable(message) => {
            let message = message.to_ascii_lowercase();
            if [
                "connect",
                "connection",
                "network",
                "dns",
                "timed out",
                "timeout",
                "no data",
            ]
            .iter()
            .any(|needle| message.contains(needle))
            {
                AttemptFailure::Network
            } else {
                AttemptFailure::Down
            }
        }
    }
}

/// Whether this machine has ANY way to reach a model. Neither `available()` (binary on PATH) nor
/// `routable()` (Unknown stays routable) answers it: both are true for an installed, logged-OUT
/// claude, which is exactly the machine that needs the setup guidance.
pub(crate) fn any_credentials_available() -> bool {
    forge_config::known_key_providers().any(forge_config::has_api_key)
        || forge_provider::any_bridge_logged_in()
}

pub(crate) fn failure_verdict(failures: &[AttemptFailure], has_credentials: bool) -> String {
    let count = |kind| failures.iter().filter(|failure| **failure == kind).count();
    // A zero-credential install is the actionable cause whatever the attempts said: a first-run
    // seed chain can be one KEYLESS candidate (a local ollama that is not running) failing as
    // "provider unavailable", which skipped this guidance and named an adapter instead.
    if !failures.is_empty() && !has_credentials {
        return format!(
            "{NO_CREDENTIALS_GUIDANCE}\nAttempts this turn: {}.",
            attempt_summary(failures)
        );
    }
    if !failures.is_empty() && count(AttemptFailure::NoCredentials) == failures.len() {
        return format!(
            "No usable model: none of your configured providers have credentials.\n\
             {NO_CREDENTIALS_GUIDANCE}\n\
             Set one of ANTHROPIC_API_KEY / OPENAI_API_KEY / GROQ_API_KEY, or log in to the \
             Claude or Codex CLI."
        );
    }
    if !failures.is_empty() && count(AttemptFailure::RateLimited) == failures.len() {
        return "No usable model: every attempted model was rate-limited.".to_string();
    }
    format!(
        "No usable model: attempted providers failed for mixed reasons ({}).",
        attempt_summary(failures)
    )
}

/// "2 rate-limited, 1 provider unavailable" — the failure mix, counted by kind.
pub(crate) fn attempt_summary(failures: &[AttemptFailure]) -> String {
    let count = |kind| failures.iter().filter(|failure| **failure == kind).count();
    [
        (AttemptFailure::NoCredentials, "missing credentials"),
        (AttemptFailure::RateLimited, "rate-limited"),
        (AttemptFailure::Capability, "capability errors"),
        (AttemptFailure::Network, "network errors"),
        (AttemptFailure::Down, "provider unavailable"),
    ]
    .into_iter()
    .filter_map(|(kind, label)| {
        let n = count(kind);
        (n > 0).then(|| format!("{n} {label}"))
    })
    .collect::<Vec<_>>()
    .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_for_all_missing_credentials_is_actionable_and_mentions_no_rate_limit() {
        let verdict = failure_verdict(
            &[AttemptFailure::NoCredentials, AttemptFailure::NoCredentials],
            true,
        );
        assert!(verdict.contains("none of your configured providers have credentials"));
        assert!(verdict.contains("forge setup"));
        assert!(
            verdict.contains(NO_CREDENTIALS_GUIDANCE),
            "reuses the shared guidance so it cannot drift from the unroutable path"
        );
        assert!(!verdict.contains("rate-limit"));
    }

    #[test]
    fn verdict_for_all_rate_limited_says_exactly_that() {
        let verdict = failure_verdict(
            &[AttemptFailure::RateLimited, AttemptFailure::RateLimited],
            true,
        );
        assert_eq!(
            verdict,
            "No usable model: every attempted model was rate-limited."
        );
    }

    #[test]
    fn verdict_for_mixed_chain_reports_each_observed_cause() {
        let verdict = failure_verdict(
            &[
                AttemptFailure::NoCredentials,
                AttemptFailure::RateLimited,
                AttemptFailure::Network,
            ],
            true,
        );
        assert!(verdict.contains("1 missing credentials"));
        assert!(verdict.contains("1 rate-limited"));
        assert!(verdict.contains("1 network errors"));
    }

    /// A first-time install: no key, no login. One keyless candidate failing as "provider
    /// unavailable" used to skip the setup guidance and name an adapter instead.
    #[test]
    fn a_zero_credential_install_is_told_how_to_set_up_whatever_failed() {
        let verdict = failure_verdict(&[AttemptFailure::Down], false);
        assert!(
            verdict.contains(NO_CREDENTIALS_GUIDANCE),
            "leads with the setup guidance: {verdict}"
        );
        assert!(
            verdict.contains("1 provider unavailable"),
            "keeps what actually happened for diagnosis: {verdict}"
        );
        assert!(
            !verdict.contains("mixed reasons"),
            "does not bury the cause behind a failure-mix summary: {verdict}"
        );
    }

    #[test]
    fn a_credentialed_install_still_gets_the_failure_mix() {
        let verdict = failure_verdict(&[AttemptFailure::Down, AttemptFailure::RateLimited], true);
        assert!(verdict.contains("mixed reasons"), "{verdict}");
        assert!(!verdict.contains(NO_CREDENTIALS_GUIDANCE), "{verdict}");
    }
}
