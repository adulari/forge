//! A turn that produced nothing must be reported as a FAILED turn.
//!
//! Reproduced twice through `forge serve`: a session was sent a detailed task, went busy, made
//! real tool calls (reads and greps, visible in the `message` table), returned EMPTY assistant
//! content for every one, then went idle with a clean working tree. Nothing was logged and the
//! fleet API said only `"busy": false` — "did the work" and "did literally nothing" were the same
//! signal from every outside surface.

use super::*;

/// Replays a scripted `(content, tool_calls)` sequence, repeating the last entry forever so
/// nudge/retry paths can keep asking without running the script off its end.
struct ScriptedProvider {
    script: Vec<(String, Vec<forge_types::ToolCall>)>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl ScriptedProvider {
    fn new(script: Vec<(&str, Vec<forge_types::ToolCall>)>) -> Self {
        Self {
            script: script
                .into_iter()
                .map(|(text, calls)| (text.to_string(), calls))
                .collect(),
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let step = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (content, tool_calls) = self
            .script
            .get(step)
            .or_else(|| self.script.last())
            .cloned()
            .unwrap_or_default();
        Ok(forge_provider::ModelResponse {
            content,
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn call(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        args,
    }
}

fn session_with(
    script: Vec<(&str, Vec<forge_types::ToolCall>)>,
    workspace: &std::path::Path,
) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let session = Session::start(
        store,
        Arc::new(ScriptedProvider::new(script)),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(workspace),
        Box::new(capture),
        Config::default(),
        workspace.to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    (session, events)
}

fn done_reason(events: &Mutex<Vec<PresenterEvent>>) -> Option<StopReason> {
    events
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match event {
            PresenterEvent::Done { stop_reason, .. } => Some(*stop_reason),
            _ => None,
        })
}

#[tokio::test]
async fn a_turn_with_no_content_and_no_mutation_is_reported_as_failed() {
    let (mut session, events) = session_with(vec![("", vec![])], test_workspace());

    let outcome = session.run_turn("do the detailed task").await.unwrap();

    assert_eq!(
        outcome.stop_reason,
        StopReason::NoOutput,
        "an empty turn must not be reported as a final answer"
    );
    assert_eq!(
        done_reason(&events),
        Some(StopReason::NoOutput),
        "the presenter (and through it every remote surface) must see the same outcome"
    );
    assert_eq!(StopReason::NoOutput.outcome(), "failed");
    assert!(!StopReason::NoOutput.is_success());
}

#[tokio::test]
async fn reading_and_grepping_without_ever_answering_is_still_a_failed_turn() {
    // The exact reproduction: real, successful, read-only tool calls followed by empty assistant
    // content. Inspection is not output — nothing was said and nothing changed.
    let (mut session, events) = session_with(
        vec![
            (
                "",
                vec![call(
                    "list_dir",
                    serde_json::json!({ "path": test_workspace().to_str().unwrap() }),
                )],
            ),
            ("", vec![]),
        ],
        test_workspace(),
    );

    let outcome = session
        .run_turn("summarize how this crate is laid out")
        .await
        .unwrap();

    assert_eq!(
        outcome.stop_reason,
        StopReason::NoOutput,
        "tool activity alone is not a completed turn"
    );
    assert!(
        events.lock().unwrap().iter().any(|event| matches!(
            event,
            PresenterEvent::Error(message) if message.contains("produced NO answer")
        )),
        "a no-op turn must be surfaced loudly, not accepted in silence"
    );
}

#[tokio::test]
async fn a_normal_turn_is_still_reported_as_successful() {
    let (mut session, events) = session_with(
        vec![("Here is the answer you asked for.", vec![])],
        test_workspace(),
    );

    let outcome = session.run_turn("what does this crate do?").await.unwrap();

    assert_eq!(outcome.stop_reason, StopReason::FinalAnswer);
    assert_eq!(done_reason(&events), Some(StopReason::FinalAnswer));
    assert_eq!(StopReason::FinalAnswer.outcome(), "success");
    assert!(StopReason::FinalAnswer.is_success());
}

#[tokio::test]
async fn an_empty_reply_after_a_successful_write_is_not_a_no_op_turn() {
    // The rule is "no assistant content AND no successful mutating tool call". A turn that
    // actually changed something did work, however tersely it reported it.
    let dir = std::env::temp_dir().join(format!("forge-no-op-turn-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("written.txt");

    let (mut session, _events) = session_with(
        vec![
            (
                "",
                vec![call(
                    "write_file",
                    serde_json::json!({
                        "path": target.to_str().unwrap(),
                        "content": "applied by the turn\n",
                    }),
                )],
            ),
            ("", vec![]),
        ],
        &dir,
    );
    session.set_mode(PermissionMode::AcceptEdits);

    let outcome = session.run_turn("write the file").await.unwrap();

    assert!(target.exists(), "the write must actually have landed");
    assert_ne!(
        outcome.stop_reason,
        StopReason::NoOutput,
        "a turn that successfully mutated state did not produce nothing"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A presenter that declines every question, standing in for the two ways plan approval ends
/// without a build: the user chose Cancel, or the surface cannot ask at all (a headless
/// `forge serve` session answers [`forge_types::NO_ANSWER`]).
#[derive(Default)]
struct DecliningPresenter {
    events: Arc<Mutex<Vec<PresenterEvent>>>,
}

impl Presenter for DecliningPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> forge_types::ConfirmOutcome {
        forge_types::ConfirmOutcome::Deny
    }
    fn ask(&mut self, _q: &str, _options: &[forge_types::QChoice], _allow_other: bool) -> String {
        forge_types::NO_ANSWER.to_string()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
}

#[tokio::test]
async fn a_declined_plan_is_output_even_though_planning_mode_can_mutate_nothing() {
    // A plan card IS the product of a planning turn, and Plan temper forbids mutation by design,
    // so "no text and no mutation" would condemn every plan turn whose model let the card speak
    // for itself. Declined approval falls through to the classifier, so this is the path that
    // must not be called a no-op.
    let workspace = test_workspace();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let presenter = DecliningPresenter::default();
    let events = presenter.events.clone();
    let mut session = Session::start(
        store,
        Arc::new(ScriptedProvider::new(vec![
            (
                "",
                vec![call(
                    "present_plan",
                    serde_json::json!({
                        "title": "Split the remote plumbing out",
                        "steps": [{ "title": "Land the core turn outcome" }],
                    }),
                )],
            ),
            ("", vec![]),
        ])),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(workspace),
        Box::new(presenter),
        Config::default(),
        workspace.to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    session.set_mode(PermissionMode::Plan);

    let outcome = session.run_turn("plan the work").await.unwrap();

    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, PresenterEvent::PlanProposed(_))),
        "the turn must actually have presented a plan"
    );
    assert_ne!(
        outcome.stop_reason,
        StopReason::NoOutput,
        "a presented plan is output, even when approval was declined"
    );
}

#[tokio::test]
async fn a_failed_write_does_not_count_as_a_mutation() {
    // A denied or failing write changed nothing, so it must not launder an otherwise silent turn
    // into a success. Read-only temper denies the write; the reply is still empty.
    let (mut session, _events) = session_with(
        vec![
            (
                "",
                vec![call(
                    "write_file",
                    serde_json::json!({
                        "path": test_workspace().join("never-written.txt").to_str().unwrap(),
                        "content": "x",
                    }),
                )],
            ),
            ("", vec![]),
        ],
        test_workspace(),
    );
    session.set_mode(PermissionMode::Plan);

    let outcome = session.run_turn("write the file").await.unwrap();

    assert!(
        !test_workspace().join("never-written.txt").exists(),
        "the write must have been denied"
    );
    assert_eq!(
        outcome.stop_reason,
        StopReason::NoOutput,
        "an attempted-but-failed mutation is not evidence of work"
    );
}
