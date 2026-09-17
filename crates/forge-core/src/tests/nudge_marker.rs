//! Harness-injected continuation nudges must carry a real, wording-independent marker so a
//! client can tell them apart from something the person actually typed (mobile bug report #20:
//! the app rendered the empty-response nudge as "You"). `record`/`nudge_policy` etc. persist
//! these as ordinary `Role::User` rows (a provider needs its next request to end on a legal user
//! turn) — this proves the store-level `nudge` flag survives onto the history page for the exact
//! "empty response" nudge site in `run_model_loop`.

use super::*;

/// Answers empty (no text, no tool call) once — tripping the empty-response nudge — then a real
/// final answer.
struct EmptyOnceProvider {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for EmptyOnceProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let content = if n == 0 { "" } else { "Done." };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            content: content.to_string(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn the_empty_response_nudge_is_persisted_with_the_nudge_marker() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let session = Session::start(
        Arc::clone(&store),
        Arc::new(EmptyOnceProvider {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        Config::default(),
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    let mut session = session;

    session.run_turn("do the task").await.unwrap();

    let page = store.load_history_page(&session.id, None, 20).unwrap();
    let nudges: Vec<_> = page.iter().filter(|r| r.nudge).collect();
    assert_eq!(
        nudges.len(),
        1,
        "exactly the one synthetic empty-response nudge must be marked"
    );
    assert_eq!(
        nudges[0].role,
        Role::User,
        "still a legal user turn for the model"
    );
    assert!(
        nudges[0].content.contains("Your last response was empty"),
        "the marked row is the empty-response nudge: {}",
        nudges[0].content
    );

    // The real prompt that started the turn must NOT be marked — only Forge-authored rows are.
    let real_prompt = page
        .iter()
        .find(|r| r.content == "do the task")
        .expect("the user's own prompt must still be on the page");
    assert!(!real_prompt.nudge, "a real user prompt is never a nudge");
}
