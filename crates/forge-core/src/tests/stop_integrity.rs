//! Re-drive passes ride out a transient provider error instead of failing the turn.

use super::harness_profile_tests::{config, Run};
use super::*;
use forge_config::HarnessProfile;

/// Fails the first `failures` calls with `error`, then answers.
struct Flaky {
    failures: usize,
    error: fn() -> forge_provider::ProviderError,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for Flaky {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n < self.failures {
            return Err((self.error)());
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "recovered".into(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn unavailable() -> forge_provider::ProviderError {
    forge_provider::ProviderError::Unavailable("502 bad gateway".into())
}

async fn redrive_pass(
    failures: usize,
    error: fn() -> forge_provider::ProviderError,
) -> (Result<String, CoreError>, usize) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let provider = Flaky {
        failures,
        error,
        calls: Arc::clone(&calls),
    };
    let mut run = Run::new(
        Arc::new(provider),
        config(HarnessProfile::Guarded),
        test_workspace(),
    );
    run.session.transcript.push(Message::user("finish the job"));
    // `None` is what every guard and hook re-drive passes: no routing decision, no failover.
    let result = run
        .session
        .run_model_loop(
            "direct::scripted".into(),
            &[],
            None,
            false,
            None,
            std::time::Duration::from_secs(60),
        )
        .await
        .map(|o| o.final_text);
    (result, calls.load(std::sync::atomic::Ordering::SeqCst))
}

#[tokio::test]
async fn a_redrive_pass_retries_a_transient_error_on_the_same_model() {
    let (result, calls) = redrive_pass(2, unavailable).await;
    assert_eq!(result.unwrap(), "recovered");
    assert_eq!(calls, 3, "two retries, then the answer");
}

#[tokio::test]
async fn a_redrive_pass_gives_up_after_two_retries() {
    let (result, calls) = redrive_pass(3, unavailable).await;
    assert!(result.is_err());
    assert_eq!(calls, 3);
}

#[tokio::test]
async fn a_redrive_pass_does_not_retry_a_request_the_provider_rejects() {
    let (result, calls) = redrive_pass(1, || {
        forge_provider::ProviderError::Request("400 bad request".into())
    })
    .await;
    assert!(result.is_err());
    assert_eq!(calls, 1);
}
