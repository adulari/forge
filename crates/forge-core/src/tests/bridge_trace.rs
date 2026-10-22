//! A bridged CLI runs its tools and model calls inside one `complete()`. The session writes them
//! down from the stream events, so metrics, replay and export see what the agent did.

use super::*;

/// Streams the events of a CLI agent loop that made `model_calls` model calls and ran two tools
/// (the second fails), then answers. Later calls just answer.
struct LoopingBridge {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    model_calls: u64,
}

#[async_trait::async_trait]
impl Provider for LoopingBridge {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_provider::StreamEvent;
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            on_event(StreamEvent::ToolStarted {
                name: "mcp__forge__read_file".into(),
                args: r#"{"path":"src/lib.rs"}"#.into(),
            });
            on_event(StreamEvent::ToolFinished {
                name: "mcp__forge__read_file".into(),
                ok: true,
                summary: "fn main".into(),
            });
            on_event(StreamEvent::ToolStarted {
                name: "Bash".into(),
                args: r#"{"command":"cargo test"}"#.into(),
            });
            on_event(StreamEvent::ToolFinished {
                name: "Bash".into(),
                ok: false,
                summary: "1 failed".into(),
            });
            on_event(StreamEvent::ModelCalls(self.model_calls));
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "Understood; here is a reply that is long enough to count as an answer."
                .to_string(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage {
                input_tokens: 100,
                output_tokens: 10,
                ..Default::default()
            },
            quotas: Vec::new(),
        })
    }
}

/// The session, its store and how many times the provider was asked: tool progress makes the loop
/// ask again, and only the first answer carries the CLI's events.
fn bridge_session(
    model: &str,
    model_calls: u64,
) -> (Session, Arc<Store>, Arc<std::sync::atomic::AtomicUsize>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let session = Session::start(
        Arc::clone(&store),
        Arc::new(LoopingBridge {
            calls: Arc::clone(&asked),
            model_calls,
        }),
        Arc::new(FixedRouter {
            model: model.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    (session, store, asked)
}

#[tokio::test]
async fn a_bridged_turn_records_the_tools_and_model_calls_its_cli_made() {
    let (mut session, store, asked) = bridge_session("claude-cli::sonnet", 5);
    session.run_turn("fix the bug").await.unwrap();
    let asked = asked.load(std::sync::atomic::Ordering::SeqCst) as u64;

    assert_eq!(store.session_tool_call_count(&session.id).unwrap(), 2);
    let usage = store.session_usage_by_model(&session.id).unwrap();
    let calls: u64 = usage.iter().map(|row| row.calls).sum();
    assert_eq!(
        calls,
        5 + (asked - 1),
        "the first usage row stands for the CLI's five model calls, any later row for one"
    );

    let replay = store.load_replay(&session.id).unwrap();
    let answer = replay
        .iter()
        .find(|e| !e.tool_calls.is_empty())
        .expect("the assistant entry carries the bridged tool calls");
    let names: Vec<&str> = answer.tool_calls.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["mcp__forge__read_file", "Bash"]);
    assert_eq!(answer.tool_calls[0].args["path"], "src/lib.rs");
}

#[tokio::test]
async fn the_trace_is_per_turn_and_only_kept_for_a_bridge_model() {
    let (mut session, store, _) = bridge_session("claude-cli::sonnet", 3);
    session.run_turn("first").await.unwrap();
    session.run_turn("second").await.unwrap();
    assert_eq!(
        store.session_tool_call_count(&session.id).unwrap(),
        2,
        "the second turn streamed nothing, so it adds nothing"
    );

    let (mut direct, store, asked) = bridge_session("mock::mock", 3);
    direct.run_turn("fix the bug").await.unwrap();
    let asked = asked.load(std::sync::atomic::Ordering::SeqCst) as u64;
    assert_eq!(store.session_tool_call_count(&direct.id).unwrap(), 0);
    let calls: u64 = store
        .session_usage_by_model(&direct.id)
        .unwrap()
        .iter()
        .map(|row| row.calls)
        .sum();
    assert_eq!(
        calls, asked,
        "a non-bridge model's usage rows are one call each"
    );
}
