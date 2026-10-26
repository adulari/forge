//! Live checks of a model source: the badge a card earns before it counts. Reuses the same calls
//! `forge doctor` makes (a model listing for a key or a subscription login, a tiny turn for a CLI
//! bridge). The result never carries the provider's raw error text, which can echo a key.

use std::time::Duration;

use forge_provider::{CliKind, ProviderError};

use crate::job::{ActionResult, JobHandle, Success};
use crate::state::VerifyStatus;

const LISTING_TIMEOUT: Duration = Duration::from_secs(8);
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(30);

/// The outcome of one check, before it becomes a job result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub status: VerifyStatus,
    pub detail: String,
}

impl Verdict {
    fn new(status: VerifyStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
        }
    }
}

/// Check `source` live and report progress on `job`.
pub fn verify_source(source: &str, job: &JobHandle) -> Result<Success, String> {
    job.progress(format!("Checking {source}"), None);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start the checker: {e}"))?;
    let verdict = runtime.block_on(check(source));
    job.output(&verdict.detail);
    Ok(Success::with(
        verdict.detail.clone(),
        ActionResult::Verified {
            source: source.to_string(),
            status: verdict.status,
            detail: Some(verdict.detail),
        },
    ))
}

async fn check(source: &str) -> Verdict {
    match source {
        "claude-cli" => bridge(CliKind::ClaudeCode).await,
        "codex-cli" => bridge(CliKind::Codex).await,
        "agy-cli" => bridge(CliKind::Antigravity).await,
        "codex-oauth" => listing(forge_provider::list_codex_oauth_models().await, "ChatGPT"),
        "xai-oauth" => listing(forge_provider::list_xai_oauth_models().await, "xAI"),
        "ollama" => ollama().await,
        key => {
            let call = forge_provider::list_models(key);
            match tokio::time::timeout(LISTING_TIMEOUT, call).await {
                Ok(result) => listing(result, &crate::providers::display_name(key)),
                Err(_) => Verdict::new(
                    VerifyStatus::Unchecked,
                    "Saved. The provider did not answer in time, so it is not checked yet.",
                ),
            }
        }
    }
}

/// Turn a model-listing result into a verdict. Only an explicit authentication refusal counts
/// against the source: plenty of providers list nothing while chat works fine.
pub fn listing(result: Result<Vec<String>, ProviderError>, name: &str) -> Verdict {
    match result {
        Ok(models) if !models.is_empty() => Verdict::new(
            VerifyStatus::Verified,
            format!("{name} answered with {} models.", models.len()),
        ),
        Ok(_) => Verdict::new(
            VerifyStatus::Unchecked,
            format!("{name} answered but listed no models, so it is not checked yet."),
        ),
        Err(error) => match auth_rejection(&error) {
            Some(code) => Verdict::new(
                VerifyStatus::Rejected,
                format!("{name} refused the key ({code}). Check it and try again."),
            ),
            None => Verdict::new(
                VerifyStatus::Unchecked,
                "Saved. Forge could not check it live, which is normal offline.",
            ),
        },
    }
}

/// 401/403 and unambiguous "invalid key" wording are an authentication refusal; a refused
/// connection, a 5xx or a missing listing endpoint are not.
pub fn auth_rejection(error: &ProviderError) -> Option<u16> {
    let lower = error.to_string().to_lowercase();
    let code = if lower.contains("403") || lower.contains("forbidden") {
        403
    } else {
        401
    };
    if matches!(error, ProviderError::Auth(_)) {
        return Some(code);
    }
    let names_status = lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("403")
        || lower.contains("forbidden");
    let names_bad_key = lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("incorrect api key")
        || lower.contains("authentication_error")
        || lower.contains("invalid authentication");
    (names_status || names_bad_key).then_some(code)
}

async fn bridge(kind: CliKind) -> Verdict {
    use forge_provider::Provider as _;
    if !kind.available() {
        return Verdict::new(VerifyStatus::Rejected, "The command is not installed.");
    }
    let provider = forge_provider::CliProvider::new(kind)
        .with_harness(false)
        .with_timeout(BRIDGE_TIMEOUT);
    let model = kind.default_model_id();
    let messages = [forge_types::Message::user("Reply with the single word: ok")];
    let mut sink = |_event: forge_provider::StreamEvent| {};
    let call = provider.complete(&model, &messages, &[], &mut sink);
    match tokio::time::timeout(BRIDGE_TIMEOUT + Duration::from_secs(2), call).await {
        Ok(Ok(reply)) if !reply.content.trim().is_empty() => {
            Verdict::new(VerifyStatus::Verified, "It launched and answered.")
        }
        Ok(Ok(_)) => Verdict::new(
            VerifyStatus::Unchecked,
            "It launched but returned no text. Try again, or sign in again.",
        ),
        Ok(Err(error)) => match auth_rejection(&error) {
            Some(_) => Verdict::new(
                VerifyStatus::Rejected,
                "It is installed but not signed in. Sign in, then check again.",
            ),
            None => Verdict::new(
                VerifyStatus::Unchecked,
                "It is installed, but a test turn did not complete. Forge will retry when you use it.",
            ),
        },
        Err(_) => Verdict::new(
            VerifyStatus::Unchecked,
            "It did not answer in 30 seconds. It may need you to sign in.",
        ),
    }
}

async fn ollama() -> Verdict {
    let client = forge_provider::bundled_http_client();
    let reply = client
        .get("http://127.0.0.1:11434/api/tags")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    match reply {
        Ok(response) if response.status().is_success() => {
            let body = response.text().await.unwrap_or_default();
            let count = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("models").and_then(|m| m.as_array()).map(Vec::len))
                .unwrap_or(0);
            if count > 0 {
                Verdict::new(
                    VerifyStatus::Verified,
                    format!("Ollama is running with {count} model(s)."),
                )
            } else {
                Verdict::new(
                    VerifyStatus::Rejected,
                    "Ollama is running but has no models yet.",
                )
            }
        }
        _ => Verdict::new(
            VerifyStatus::Unchecked,
            "Ollama is not running right now. Forge starts it when it needs it.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_listing_verifies_the_source() {
        let v = listing(Ok(vec!["a".into(), "b".into()]), "Groq");
        assert_eq!(v.status, VerifyStatus::Verified);
        assert!(v.detail.contains("2 models"));
    }

    #[test]
    fn an_empty_listing_proves_nothing() {
        assert_eq!(listing(Ok(vec![]), "Groq").status, VerifyStatus::Unchecked);
    }

    #[test]
    fn only_an_authentication_refusal_rejects_the_source() {
        let rejected = listing(Err(ProviderError::Auth("bad".into())), "Groq");
        assert_eq!(rejected.status, VerifyStatus::Rejected);
        let rejected = listing(
            Err(ProviderError::Request("HTTP 401 Unauthorized".into())),
            "Groq",
        );
        assert_eq!(rejected.status, VerifyStatus::Rejected);
        let offline = listing(
            Err(ProviderError::Request("connection refused".into())),
            "Groq",
        );
        assert_eq!(offline.status, VerifyStatus::Unchecked);
    }

    #[test]
    fn a_verdict_never_echoes_the_providers_error_text() {
        let echoed =
            ProviderError::Request("Incorrect API key provided: sk-live-abcdef123456".into());
        let v = listing(Err(echoed), "OpenAI");
        assert_eq!(v.status, VerifyStatus::Rejected);
        assert!(!v.detail.contains("sk-live"), "{}", v.detail);
    }
}
