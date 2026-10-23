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

/// The error for a pinned model whose fast failure is a refusal, not an outage: the same error
/// twice in a row, or one refusal of an id no discovered catalog lists (startup already warned).
/// One plain line; the provider's own words go to the log.
pub(crate) fn rejected_model_error(
    model: &str,
    error: &forge_provider::ProviderError,
    repeated: bool,
    attended: bool,
) -> CoreError {
    tracing::warn!(model, %error, repeated, "pinned model refused; ending the turn");
    let why = if repeated {
        "its provider rejected it twice"
    } else {
        "its provider rejected it and no catalog lists it"
    };
    model_unavailable(model, why, attended)
}

/// What a pinned model's failure surfaces as. A permanent "this model is not callable" answer
/// (`NoModelAccess`) reads as the plain line; every other error keeps its own text.
pub(crate) fn pinned_failure_error(
    model: &str,
    error: forge_provider::ProviderError,
    attended: bool,
) -> CoreError {
    if !matches!(error, forge_provider::ProviderError::NoModelAccess(_)) {
        return error.into();
    }
    tracing::warn!(model, %error, "pinned model is not available; ending the turn");
    let why = if forge_provider::falls_back_to_local_ollama(model) {
        "no provider knows this id"
    } else {
        "not available on this account"
    };
    model_unavailable(model, why, attended)
}

/// `/model` exists where a person is at the session; a script has no such command and is pointed
/// at the listing instead.
fn model_unavailable(model: &str, why: &str, attended: bool) -> CoreError {
    let way_out = if attended {
        "Run /model to pick another."
    } else {
        "`forge models` lists the usable ids."
    };
    CoreError::ModelUnavailable {
        model: model.to_string(),
        message: format!("model '{model}' isn't available ({why}). {way_out}"),
    }
}

pub(crate) fn in_set_advance_reason(rejected: bool, repeated: bool) -> &'static str {
    match (rejected, repeated) {
        (true, true) => "the provider rejected this model twice in a row",
        (true, false) => "the provider rejected this model, and no discovered catalog lists it",
        (false, _) => "still unreachable after backoff retries",
    }
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
        return "No usable model: every attempted provider rejected its credentials (HTTP 401/403). \
                Re-run `forge auth <provider>` with a valid key, or log in to the Claude or Codex CLI \
                again."
            .to_string();
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
    fn a_dead_model_reads_as_one_plain_line_with_the_way_out() {
        let e = pinned_failure_error(
            "groq::llama-9",
            forge_provider::ProviderError::NoModelAccess(
                "The model `llama-9` does not exist".into(),
            ),
            true,
        );
        let text = e.to_string();
        assert_eq!(
            text,
            "model 'groq::llama-9' isn't available (not available on this account). Run /model to pick another."
        );
        assert!(!text.contains('\n'), "one line: {text}");
        assert!(!text.contains("adapter"), "no adapter jargon: {text}");
        assert!(!text.contains("pin_failover"), "no config jargon: {text}");
    }

    #[test]
    fn an_unrecognised_bare_id_says_nothing_recognises_it() {
        let e = pinned_failure_error(
            "nonexistent-model-xyz",
            forge_provider::ProviderError::NoModelAccess("no local Ollama answered".into()),
            true,
        );
        assert_eq!(
            e.to_string(),
            "model 'nonexistent-model-xyz' isn't available (no provider knows this id). Run /model to pick another."
        );
    }

    #[test]
    fn a_script_is_pointed_at_the_listing_not_at_a_slash_command() {
        let e = pinned_failure_error(
            "groq::llama-9",
            forge_provider::ProviderError::NoModelAccess("gone".into()),
            false,
        );
        assert_eq!(
            e.to_string(),
            "model 'groq::llama-9' isn't available (not available on this account). `forge models` lists the usable ids."
        );
    }

    #[test]
    fn other_pinned_failures_keep_their_own_text() {
        let e = pinned_failure_error(
            "groq::x",
            forge_provider::ProviderError::Capability("no tool support".into()),
            true,
        );
        assert!(matches!(e, CoreError::Provider(_)), "{e:?}");
        assert!(e.to_string().contains("no tool support"));
    }

    #[test]
    fn a_rejection_names_the_model_and_hides_the_provider_text() {
        let provider_error = forge_provider::ProviderError::Unavailable("502 bad gateway".into());
        let twice = rejected_model_error("pin::m", &provider_error, true, true).to_string();
        assert_eq!(
            twice,
            "model 'pin::m' isn't available (its provider rejected it twice). Run /model to pick another."
        );
        assert!(!twice.contains("502"), "details live in the log: {twice}");
        let once = rejected_model_error("pin::m", &provider_error, false, true).to_string();
        assert!(once.contains("no catalog lists it"), "{once}");
    }

    #[test]
    fn verdict_for_rejected_credentials_does_not_claim_none_configured() {
        let verdict = failure_verdict(
            &[AttemptFailure::NoCredentials, AttemptFailure::NoCredentials],
            true,
        );
        assert!(verdict.contains("rejected its credentials"));
        assert!(!verdict.contains("none of your configured providers"));
        assert!(!verdict.contains("rate-limit"));
    }

    #[test]
    fn verdict_without_any_credentials_leads_with_setup_guidance() {
        let verdict = failure_verdict(&[AttemptFailure::NoCredentials], false);
        assert!(verdict.contains(NO_CREDENTIALS_GUIDANCE));
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
