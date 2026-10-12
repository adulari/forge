use super::*;

#[test]
fn completion_claims_file_change_only_matches_applied_changes() {
    for claim in [
        "I've created the requested file.",
        "I have added the workflow step.",
        "I've updated the file.",
        "I've modified the configuration.",
        "Created the file and wired it in.",
        "The file now contains the shared command.",
        "Changes applied.",
    ] {
        assert!(
            completion_claims_file_change(claim),
            "past-tense mutation claim was missed: {claim}"
        );
    }

    for non_claim in [
        "Here is what the diff would look like.",
        "I will create the requested file.",
        "You could add this workflow step.",
        "This would create the file.",
        "```diff\n+new line\n-old line\n```",
    ] {
        assert!(
            !completion_claims_file_change(non_claim),
            "non-applied change was treated as an edit claim: {non_claim}"
        );
    }
}

struct PhantomEditProvider {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for PhantomEditProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "I've created the file and updated the workflow.".into(),
            tool_calls: vec![],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn completion_with_empty_tasks_and_phantom_edits_is_nudged_before_acceptance() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(PhantomEditProvider {
            calls: Arc::clone(&calls),
        }),
        Arc::new(FixedRouter {
            model: "direct::phantom-edit".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        Config::default(),
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();

    session
        .run_turn("create a file and update CI")
        .await
        .unwrap();

    assert!(calls.load(std::sync::atomic::Ordering::Relaxed) > 1);
    let warnings = events.lock().unwrap();
    assert!(
        warnings.iter().any(|event| matches!(
            event,
            PresenterEvent::Warning(message)
                if message.contains("no mutating tool ran")
        )),
        "phantom-edit nudge was not emitted: {warnings:?}"
    );
    assert!(warnings.iter().any(|event| matches!(
        event,
        PresenterEvent::Warning(message)
            if message.contains("did NOT apply")
    )));
}

const ISSUE_PROMPT: &str = "Modify ONLY memory_service/obsidian.py. Replace section_keywords with \
    two explicit mappings: group_sections and perspective_sections. Update ONLY the section-building \
    logic in generate_daily_timeline.";
const ISSUE_REPLY: &str = "Implemented deterministic section handling in \
    `memory_service/obsidian.py`:\n- Added explicit `group_sections` and `perspective_sections` \
    mappings.\n- Replaced the previous keyword-based section building logic.";

struct FixedTextProvider {
    text: &'static str,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for FixedTextProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: self.text.into(),
            tool_calls: vec![],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

async fn run_text_turn(
    text: &'static str,
    prompt: &str,
    stale_done_tasks: bool,
) -> (usize, Vec<String>) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(FixedTextProvider {
            text,
            calls: Arc::clone(&calls),
        }),
        Arc::new(FixedRouter {
            model: "direct::phantom-edit".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        Config::default(),
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    if stale_done_tasks {
        session.tasks = vec![forge_types::TodoItem {
            title: "earlier work".into(),
            status: forge_types::TodoStatus::Done,
            assignee: None,
        }];
    }
    session.run_turn(prompt).await.unwrap();
    let warnings = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            PresenterEvent::Warning(message) => Some(message.clone()),
            _ => None,
        })
        .collect();
    (calls.load(std::sync::atomic::Ordering::Relaxed), warnings)
}

#[tokio::test]
async fn issue_1428_toolless_bulleted_summary_is_not_accepted_silently() {
    for stale in [false, true] {
        let (calls, warnings) = run_text_turn(ISSUE_REPLY, ISSUE_PROMPT, stale).await;
        let (control, _) = run_text_turn("ok", "say ok", stale).await;
        assert!(calls > control, "no re-drive (stale tasks: {stale})");
        assert!(
            warnings.iter().any(|w| w.contains("empty diff")),
            "stale={stale}: {warnings:?}"
        );
    }
}

#[tokio::test]
async fn issue_1428_claim_without_a_change_directive_is_nudged_with_stale_done_tasks() {
    for stale in [false, true] {
        let (calls, warnings) = run_text_turn(
            ISSUE_REPLY,
            "The section handling in memory_service/obsidian.py should be deterministic.",
            stale,
        )
        .await;
        let (control, _) = run_text_turn("ok", "say ok", stale).await;
        assert!(calls > control, "no re-drive (stale tasks: {stale})");
        assert!(
            warnings.iter().any(|w| w.contains("no mutating tool ran")),
            "stale={stale}: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("did NOT apply")),
            "stale={stale}: not loud on exhaustion: {warnings:?}"
        );
    }
}

#[tokio::test]
async fn analysis_prompt_answered_without_tools_is_accepted_cleanly() {
    for stale in [false, true] {
        let (calls, warnings) = run_text_turn(
            "`section_keywords` maps substrings to headings; the first match wins.",
            "Explain how section_keywords works in memory_service/obsidian.py",
            stale,
        )
        .await;
        let (control, _) = run_text_turn("ok", "say ok", stale).await;
        assert_eq!(
            calls, control,
            "analysis turn was re-driven (stale: {stale})"
        );
        assert!(
            !warnings
                .iter()
                .any(|w| w.contains("mutating tool") || w.contains("empty diff")),
            "{warnings:?}"
        );
    }
}
