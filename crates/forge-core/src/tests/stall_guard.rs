//! The narration-stall guard through a real turn: a model that keeps saying the same thing while
//! reading something slightly different every step is nudged once, then stopped.

use super::*;

struct SameSentenceReader {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for SameSentenceReader {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            content: "You're right — I looped. Answering the question from evidence, then \
                      finishing the revert I left half-done."
                .into(),
            tool_calls: vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "Cargo.toml", "limit": 5 + n}),
            }],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

struct CyclingReader {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    pattern: Vec<String>,
}

#[async_trait::async_trait]
impl Provider for CyclingReader {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let text = self.pattern[n % self.pattern.len()].clone();
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            content: text,
            tool_calls: vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "Cargo.toml", "limit": 5 + n}),
            }],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

/// The live 40-step distinct-text pattern (see `stall_guard::tests`): only 5 of the 39 adjacent
/// pairs are identical, so a model saying this never repeats consecutively — it must be caught by
/// the frequency-over-a-window check, not the consecutive one.
#[tokio::test]
async fn a_model_cycling_among_a_few_phrasings_is_nudged_then_stopped() {
    const TEXTS: [(char, &str); 15] = [
        (
            'A',
            "Reviewing the retry logic before making any further changes here.",
        ),
        (
            'B',
            "Checking the queue depth to understand the current backlog size.",
        ),
        (
            'C',
            "Reading the config file to confirm the timeout value used.",
        ),
        ('D', "Looking at the worker thread to see where it blocks."),
        (
            'E',
            "Inspecting the response headers to find the rate limit source.",
        ),
        (
            'F',
            "Tracing the error path through the client before the retry.",
        ),
        (
            'G',
            "Comparing the two branches to spot the behavioral difference found.",
        ),
        (
            'H',
            "Verifying the schema migration applied correctly to the test database.",
        ),
        (
            'I',
            "Walking through the call stack to locate the actual failure.",
        ),
        (
            'J',
            "Auditing the recent commits for anything touching this shared module.",
        ),
        (
            'K',
            "Measuring the latency spread across the last few sample runs.",
        ),
        (
            'L',
            "Confirming the feature flag state before touching production traffic.",
        ),
        (
            'M',
            "Scanning the logs for any related warning near the crash.",
        ),
        (
            'N',
            "Profiling the hot path to rule out a performance regression.",
        ),
        (
            'O',
            "Summarizing findings so far before deciding on the next step.",
        ),
    ];
    const PATTERN: &str = "ABACADEFFEDGCHAIACCBADIIGJDCAKBGBBLMMNEO";
    let sequence: Vec<String> = PATTERN
        .chars()
        .map(|c| TEXTS.iter().find(|(k, _)| *k == c).unwrap().1.to_string())
        .collect();

    let store = Arc::new(Store::open_in_memory().unwrap());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    let mut session = Session::start(
        store,
        Arc::new(CyclingReader {
            calls: Arc::clone(&calls),
            pattern: sequence.clone(),
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session
        .run_turn("why does this keep happening?")
        .await
        .unwrap();
    let n = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        n < sequence.len(),
        "guard never fired: ran through the whole {}-step synthetic pattern without stopping",
        sequence.len()
    );
    assert!(
        session
            .transcript
            .iter()
            .any(|m| m.role == Role::System && m.content == crate::stall_guard::STALL_NUDGE),
        "the nudge reached the model"
    );
    let errors: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Error(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("repeating the same statement")),
        "{errors:?}"
    );
}

#[tokio::test]
async fn a_model_repeating_its_opening_while_reading_different_things_is_nudged_then_stopped() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    let mut session = Session::start(
        store,
        Arc::new(SameSentenceReader {
            calls: Arc::clone(&calls),
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("why the 429?").await.unwrap();
    let n = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        (7..=9).contains(&n),
        "nudge on the 5th statement, halt two repeats later, not the step cap: {n} calls"
    );
    assert!(
        session
            .transcript
            .iter()
            .any(|m| m.role == Role::System && m.content == crate::stall_guard::STALL_NUDGE),
        "the nudge reached the model"
    );
    let errors: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Error(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("repeating the same statement")),
        "{errors:?}"
    );
}
