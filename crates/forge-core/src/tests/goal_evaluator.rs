//! The `/goal` evaluator side call: what it is shown, what it costs, and how it degrades.

use super::*;
use crate::goal_evaluator::GOAL_EVALUATOR_MARKER;

const MODEL: &str = "ollama::goal-eval";

/// Answers the evaluator with a fixed reply (or an error) and records what it was asked.
struct EvaluatorProvider {
    reply: Result<String, ()>,
    seen: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl Provider for EvaluatorProvider {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if messages
            .first()
            .is_some_and(|m| m.content.starts_with(GOAL_EVALUATOR_MARKER))
        {
            let input = messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            self.seen.lock().unwrap().push((model.to_string(), input));
        }
        let content = self
            .reply
            .clone()
            .map_err(|_| forge_provider::ProviderError::Unavailable("upstream 503".into()))?;
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls: vec![],
            usage: forge_types::Usage {
                input_tokens: 400,
                output_tokens: 60,
                ..forge_types::Usage::default()
            },
            quotas: Vec::new(),
        })
    }
}

type Seen = Arc<Mutex<Vec<(String, String)>>>;
type Events = Arc<Mutex<Vec<PresenterEvent>>>;
type Harness = (Session, Arc<Store>, Events, Seen, tempfile::TempDir);

fn session_with(reply: Result<&str, ()>, config: Config) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter {
        attended: true,
        ..CapturePresenter::default()
    };
    let events = capture.events.clone();
    let seen: Seen = Arc::default();
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(EvaluatorProvider {
            reply: reply.map(str::to_string),
            seen: seen.clone(),
        }),
        Arc::new(FixedRouter {
            model: MODEL.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.set_catalog(Some(ModelCatalog::new(vec![MODEL.into()])));
    (session, store, events, seen, dir)
}

fn request() -> GoalEvalRequest {
    GoalEvalRequest {
        goal: "add add/list/done commands to todo.py".into(),
        started_unix: 0,
        iteration: 2,
        previous_remaining: vec!["add the done command".into()],
        last_gate_failure: None,
    }
}

fn progress_lines(events: &[PresenterEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Progress(line) => Some(line.clone()),
            _ => None,
        })
        .collect()
}

fn warnings(events: &[PresenterEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(w) => Some(w.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_verdict_is_parsed_shown_and_priced() {
    let reply = "{\"achieved\": false, \"remaining\": [\"add the done command\", \"test list\"], \
                 \"next_step\": \"add the done command\", \"confidence\": 0.7, \"reason\": \
                 \"done is missing\"}";
    let (mut session, store, events, seen, _dir) = session_with(Ok(reply), Config::default());
    session.tasks = vec![forge_types::TodoItem {
        title: "add the add command".into(),
        status: forge_types::TodoStatus::Done,
        assignee: None,
    }];

    let verdict = session.evaluate_goal(&request()).await.expect("a verdict");

    assert!(!verdict.achieved);
    assert_eq!(verdict.remaining.len(), 2);
    assert_eq!(verdict.model, MODEL);
    assert_eq!(
        progress_lines(&events.lock().unwrap()),
        vec!["🎯 2 left · next: add the done command".to_string()]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (model, input) = &seen[0];
    assert_eq!(model, MODEL);
    for needle in [
        "add add/list/done commands to todo.py",
        "- [done] add the add command",
        "not a git repository",
        "- add the done command",
    ] {
        assert!(input.contains(needle), "missing {needle:?}");
    }
    let usage = store
        .usage_by_provider_for_session(session.session_id())
        .unwrap();
    assert_eq!(
        usage.len(),
        1,
        "the evaluator call is recorded like other side calls"
    );
    assert_eq!(usage[0].provider, "ollama");
}

#[tokio::test]
async fn malformed_output_falls_back_with_one_warning() {
    let (mut session, store, events, _seen, _dir) =
        session_with(Ok("The goal looks mostly done."), Config::default());
    assert!(session.evaluate_goal(&request()).await.is_none());
    let events = events.lock().unwrap();
    assert!(progress_lines(&events).is_empty());
    let warnings = warnings(&events);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("no usable verdict"), "{warnings:?}");
    assert_eq!(
        store
            .usage_by_provider_for_session(session.session_id())
            .unwrap()
            .len(),
        1,
        "a malformed reply still cost tokens and is still recorded"
    );
}

#[tokio::test]
async fn a_failed_call_falls_back_without_stalling() {
    let (mut session, _store, events, _seen, _dir) = session_with(Err(()), Config::default());
    assert!(session.evaluate_goal(&request()).await.is_none());
    assert!(warnings(&events.lock().unwrap())
        .iter()
        .any(|w| w.contains("goal evaluator failed")));
}

#[tokio::test]
async fn switched_off_it_makes_no_call_and_says_nothing() {
    let mut config = Config::default();
    config.goal.evaluator = false;
    let (mut session, _store, events, seen, _dir) = session_with(Ok("{}"), config);
    assert!(!session.goal_evaluator_enabled());
    let before = events.lock().unwrap().len();
    assert!(session.evaluate_goal(&request()).await.is_none());
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(events.lock().unwrap().len(), before);
}

#[tokio::test]
async fn a_configured_evaluator_model_is_used() {
    let mut config = Config::default();
    config.goal.evaluator_model = Some("ollama::judge".into());
    let reply = "{\"achieved\": true, \"remaining\": [], \"reason\": \"all three commands work\"}";
    let (mut session, _store, events, seen, _dir) = session_with(Ok(reply), config);
    let verdict = session.evaluate_goal(&request()).await.unwrap();
    assert!(verdict.achieved);
    assert_eq!(seen.lock().unwrap()[0].0, "ollama::judge");
    assert_eq!(
        progress_lines(&events.lock().unwrap()),
        vec!["🎯 goal achieved: all three commands work".to_string()]
    );
}

#[tokio::test]
async fn a_foreign_subscription_evaluator_model_is_not_spent() {
    let mut config = Config::default();
    config.goal.evaluator_model = Some("codex-oauth::gpt-5.6-sol".into());
    let reply = "{\"achieved\": false, \"remaining\": [\"x\"], \"next_step\": \"x\"}";
    let (mut session, _store, _events, seen, _dir) = session_with(Ok(reply), config);
    session.evaluate_goal(&request()).await.unwrap();
    assert_eq!(
        seen.lock().unwrap()[0].0,
        MODEL,
        "the side-call model serves instead of a plan the session is not running on"
    );
}
