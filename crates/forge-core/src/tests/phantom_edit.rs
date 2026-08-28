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
