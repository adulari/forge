//! The loop-progress guard through a real turn: a model that keeps saying the same thing while
//! re-reading what it has already seen is nudged once, then stopped cleanly; a model that says the
//! same thing while every step edits something new is left alone.

use super::*;

fn loop_notices(events: &Mutex<Vec<PresenterEvent>>) -> (Vec<String>, Vec<String>) {
    let events = events.lock().unwrap();
    let pick = |warn: bool| {
        events
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(t) if warn => Some(t.clone()),
                PresenterEvent::Error(t) if !warn => Some(t.clone()),
                _ => None,
            })
            .collect()
    };
    (pick(true), pick(false))
}

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
            reasoning_items: Vec::new(),
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
            reasoning_items: Vec::new(),
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
        session.transcript.iter().any(|m| m.role == Role::System
            && (m.content == crate::stall_guard::STALL_NUDGE
                || m.content == crate::loop_progress::STAGNATION_NUDGE)),
        "the nudge reached the model"
    );
    let (warnings, errors) = loop_notices(&events);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("stopping to avoid a loop")),
        "{warnings:?}"
    );
    assert!(errors.is_empty(), "a loop halt is not a crash: {errors:?}");
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
    assert_eq!(
        n, 7,
        "nudge on the 4th statement, halt two stale steps later (6 calls), plus the one \
         tool-less finalization completion; not the step cap: {n} calls"
    );
    assert!(
        session
            .transcript
            .iter()
            .any(|m| m.role == Role::System && m.content == crate::stall_guard::STALL_NUDGE),
        "the nudge reached the model"
    );
    let (warnings, errors) = loop_notices(&events);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("stopping to avoid a loop")),
        "{warnings:?}"
    );
    assert!(errors.is_empty(), "a loop halt is not a crash: {errors:?}");
}

/// Answers with the stall sentence plus a different read every step, but gives a real summary
/// when asked for one with no tools advertised (what the finalizer does).
struct StallsUntilAskedToFinish {
    inner: SameSentenceReader,
}

#[async_trait::async_trait]
impl Provider for StallsUntilAskedToFinish {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if !tools.is_empty() {
            return self.inner.complete(model, messages, tools, on_event).await;
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "Cargo.toml is the largest file.".into(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn halting_session(provider: Arc<dyn Provider>) -> Session {
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_halted_turn_that_did_work_is_finalized_from_the_gathered_evidence() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut session = halting_session(Arc::new(StallsUntilAskedToFinish {
        inner: SameSentenceReader { calls },
    }));
    let out = session.run_turn("which file is largest?").await.unwrap();
    assert_eq!(out.text, "Cargo.toml is the largest file.");
}

#[tokio::test]
async fn a_halted_turn_whose_finalizer_also_fails_still_reports_something() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut session = halting_session(Arc::new(SameSentenceReader { calls }));
    let out = session.run_turn("why the 429?").await.unwrap();
    assert!(!out.text.trim().is_empty(), "empty result after a halt");
}

/// Seven consecutive successful edits that all open with the same sentence: the old text-only guard
/// nudged and then halted this with an Error. Every step changes a different file, so none of it is
/// a loop.
struct SteadyEditor {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    edits: usize,
}

#[async_trait::async_trait]
impl Provider for SteadyEditor {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, tool_calls) = if n < self.edits {
            (
                "Now let me update the module so the next case is covered as well.".to_string(),
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "write_file".into(),
                    args: serde_json::json!({"path": format!("case_{n}.txt"), "content": format!("case {n}\n")}),
                }],
            )
        } else {
            (format!("Updated {} files.", self.edits), vec![])
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn a_productive_run_of_edits_with_one_opening_sentence_is_never_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(SteadyEditor {
            calls: Arc::clone(&calls),
            edits: 12,
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("add the cases").await.unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 13);
    assert!(dir.path().join("case_11.txt").exists());
    let (warnings, errors) = loop_notices(&events);
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("same sentence") || w.contains("avoid a loop")),
        "{warnings:?}"
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert!(
        !session.transcript.iter().any(|m| m.role == Role::System
            && (m.content == crate::stall_guard::STALL_NUDGE
                || m.content == crate::loop_progress::STAGNATION_NUDGE)),
        "no loop nudge in a productive turn"
    );
}
