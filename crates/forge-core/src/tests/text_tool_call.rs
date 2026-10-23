//! A model that wrote its tool call as text on a request that carried no tools: the retry carries
//! the tools, the raw markup never becomes the answer, and a prompt that names a file never starts
//! without tools in the first place.

use super::*;

const CUT_OFF_DSML: &str =
    "I'll check the file.\n\n<｜DSML｜ calls>\n<｜DSML｜ invoke name=\"read_file\">\n\
<｜DSML｜ parameter name=\"path\" string=\"true\">src/li";

/// Records how many tools each request carried and replays a scripted list of replies, repeating
/// the last one.
struct ScriptedProvider {
    replies: Vec<(String, Vec<forge_types::ToolCall>)>,
    tools_seen: Arc<Mutex<Vec<usize>>>,
}

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let mut seen = self.tools_seen.lock().unwrap();
        let (content, tool_calls) = self.replies[seen.len().min(self.replies.len() - 1)].clone();
        seen.push(tools.len());
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

async fn run(
    prompt: &str,
    replies: Vec<(String, Vec<forge_types::ToolCall>)>,
) -> (forge_types::LoopOutcome, Vec<usize>, Vec<String>) {
    let tools_seen = Arc::new(Mutex::new(Vec::new()));
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
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(ScriptedProvider {
            replies,
            tools_seen: Arc::clone(&tools_seen),
        }),
        Arc::new(FixedRouter {
            model: "ollama::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    let outcome = session.run_turn(prompt).await.unwrap();
    let warnings = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(w) => Some(w.clone()),
            _ => None,
        })
        .collect();
    let seen = tools_seen.lock().unwrap().clone();
    (outcome, seen, warnings)
}

#[tokio::test]
async fn text_call_on_a_toolless_request_gets_the_tools_on_the_retry() {
    let (outcome, tools, warnings) = run(
        "tell me a joke",
        vec![
            (CUT_OFF_DSML.into(), vec![]),
            ("Here is the answer.".into(), vec![]),
        ],
    )
    .await;
    assert_eq!(tools.len(), 2, "one nudge, one retry: {tools:?}");
    assert_eq!(tools[0], 0, "a joke request starts without tools");
    assert!(tools[1] > 0, "the retry must carry tools: {tools:?}");
    assert!(
        warnings.iter().any(|w| w.contains("tool call as text")),
        "{warnings:?}"
    );
    assert_eq!(outcome.text, "Here is the answer.");
}

#[tokio::test]
async fn a_model_that_keeps_writing_the_markup_never_shows_it_as_the_answer() {
    let (outcome, tools, warnings) =
        run("tell me a joke", vec![(CUT_OFF_DSML.into(), vec![])]).await;
    assert_eq!(tools.len(), 3, "two nudges then the end: {tools:?}");
    assert!(
        warnings.iter().any(|w| w.contains("never executed")),
        "{warnings:?}"
    );
    assert!(!outcome.text.contains("DSML"), "{:?}", outcome.text);
    assert_eq!(outcome.text, "I'll check the file.");
}

#[tokio::test]
async fn a_recovered_call_on_a_toolless_request_keeps_tools_for_the_next_step() {
    let call = forge_types::ToolCall {
        id: "recovered_0".into(),
        name: "read_file".into(),
        args: serde_json::json!({"path": "Cargo.toml", "limit": 3}),
    };
    let (outcome, tools, _) = run(
        "tell me a joke",
        vec![
            ("Reading.".into(), vec![call]),
            ("It has three lines.".into(), vec![]),
        ],
    )
    .await;
    assert_eq!(tools.len(), 2, "{tools:?}");
    assert_eq!(tools[0], 0);
    assert!(tools[1] > 0, "tool history needs the tools: {tools:?}");
    assert_eq!(outcome.text, "It has three lines.");
}

#[tokio::test]
async fn a_prompt_naming_a_file_starts_with_tools() {
    let (_, tools, _) = run(
        "Which function in `grid/ranges.py` rejects a reversed range?",
        vec![("It is _check_order.".into(), vec![])],
    )
    .await;
    assert_eq!(tools.len(), 1);
    assert!(tools[0] > 0, "{tools:?}");
}
