//! A pinned model throttled on TOKENS: say why, and send less once waiting has not helped.

use super::*;

/// Throttles the first `fail_first` calls with `message`, then answers.
struct ThrottleThenOk {
    calls: std::sync::atomic::AtomicUsize,
    fail_first: usize,
    message: &'static str,
}

#[async_trait::async_trait]
impl Provider for ThrottleThenOk {
    async fn complete(
        &self,
        model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n < self.fail_first {
            return Err(forge_provider::ProviderError::RateLimited {
                message: self.message.into(),
                retry_after: Some(std::time::Duration::from_millis(10)),
            });
        }
        on_event(StreamEvent::Text(model.into()));
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: model.into(),
            tool_calls: vec![],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

async fn run(message: &'static str, fail_first: usize) -> (String, Vec<String>, Session) {
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let config = Config::default();
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(ThrottleThenOk {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_first,
            message,
        }),
        Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    let answer = session.run_turn("fix the bug").await.unwrap();
    let warnings = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(w) => Some(w.clone()),
            _ => None,
        })
        .filter(|w| !w.contains("empty diff"))
        .collect();
    (answer.to_string(), warnings, session)
}

#[tokio::test]
async fn the_second_consecutive_token_throttle_compacts_before_waiting_again() {
    let (answer, warnings, _session) =
        run("Too many tokens, please wait before trying again.", 3).await;
    assert_eq!(answer, "pin::model", "the pinned model still answers");
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings[0].contains("rate limited (pin: Too many tokens"),
        "the provider's reason is shown: {}",
        warnings[0]
    );
    assert!(
        !warnings[0].contains("compacted"),
        "one throttle is a burst, not yet a reason to compact: {}",
        warnings[0]
    );
    assert!(
        warnings[1].contains("context was compacted"),
        "{}",
        warnings[1]
    );
    assert!(
        warnings[1].contains("tokens-per-minute quota"),
        "the warning names the real remedy: {}",
        warnings[1]
    );
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("exceeded the context window")),
        "a throttle is not reported as an overflow: {warnings:?}"
    );
}

#[tokio::test]
async fn a_request_rate_throttle_keeps_plain_backoff() {
    let (answer, warnings, session) =
        run("Too many requests, please wait before trying again.", 3).await;
    assert_eq!(answer, "pin::model");
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    for w in &warnings {
        assert!(w.contains("rate limited (pin: Too many requests"), "{w}");
        assert!(!w.contains("compacted"), "{w}");
    }
    assert!(
        session.overflow_window_cap.is_none(),
        "no compaction ran, so no window cap was armed"
    );
}
