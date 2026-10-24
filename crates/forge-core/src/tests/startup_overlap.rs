//! Work that used to sit in front of the first prompt now runs beside startup: the pinned model's
//! context-window lookup and the Claude bridge process.

use super::*;
use forge_provider::CheckpointContext;
use std::time::Duration;

type Warmed = Vec<(String, String, Option<forge_types::EffortLevel>)>;

#[derive(Default)]
struct WarmRecorder {
    warmed: Mutex<Warmed>,
}

#[async_trait::async_trait]
impl Provider for WarmRecorder {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        unreachable!("these tests never run a turn")
    }

    async fn prewarm(
        &self,
        model: &str,
        checkpoint: &CheckpointContext,
        effort: Option<forge_types::EffortLevel>,
    ) {
        self.warmed
            .lock()
            .unwrap()
            .push((model.to_string(), checkpoint.session.clone(), effort));
    }
}

fn session(dir: &std::path::Path, provider: Arc<WarmRecorder>) -> (Session, CapturePresenter) {
    let capture = CapturePresenter {
        attended: true,
        ..CapturePresenter::default()
    };
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(FixedRouter {
            model: "claude-cli::haiku".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(capture.clone()),
        Config::default(),
        dir.to_str().unwrap(),
    )
    .unwrap();
    (session, capture)
}

fn warnings(capture: &CapturePresenter) -> Vec<String> {
    capture
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

async fn warmed_within(provider: &WarmRecorder, wait: Duration) -> Warmed {
    let deadline = tokio::time::Instant::now() + wait;
    while tokio::time::Instant::now() < deadline {
        if !provider.warmed.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    provider.warmed.lock().unwrap().clone()
}

const UNKNOWN: &str = "gateway::model";

/// Words enough that the 32k floor would compact them: the window decides what is sent.
fn fill_past_a_quarter_of_the_floor_trigger(session: &mut Session) {
    let words = vec!["context"; 12_000].join(" ");
    session.transcript.push(Message::user(words));
}

#[tokio::test]
async fn a_small_first_request_goes_out_without_waiting_for_the_window_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, capture) = session(dir.path(), Arc::default());
    let (tx, lookup) = crate::WindowLookup::channel();
    session.set_window_lookup(Some(lookup));

    let started = std::time::Instant::now();
    session.settle_window_lookup(UNKNOWN, Some(&[])).await;
    session.settle_window_lookup(UNKNOWN, None).await;
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the floor sizes a first request exactly as the real window would"
    );
    assert!(warnings(&capture).is_empty());

    // The answer is still taken by a later request.
    tx.send(Some(vec![UNKNOWN.to_string()])).unwrap();
    session.settle_window_lookup(UNKNOWN, None).await;
    assert_eq!(warnings(&capture).len(), 1);
}

#[tokio::test]
async fn a_context_the_floor_would_cut_waits_for_the_lookup_and_names_a_window_it_never_learned() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, capture) = session(dir.path(), Arc::default());
    let (tx, lookup) = crate::WindowLookup::channel();
    session.set_window_lookup(Some(lookup));
    fill_past_a_quarter_of_the_floor_trigger(&mut session);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = tx.send(Some(vec![UNKNOWN.to_string()]));
    });

    let started = std::time::Instant::now();
    session.settle_window_lookup(UNKNOWN, None).await;

    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "a context the floor would compact must not be sized before the lookup settled"
    );
    let notice = warnings(&capture).join("\n");
    assert!(
        notice.contains("context window of gateway::model is unknown"),
        "{notice}"
    );
    session.settle_window_lookup(UNKNOWN, None).await;
    assert_eq!(warnings(&capture).len(), 1, "the lookup is consumed once");
}

#[tokio::test]
async fn a_window_notice_is_not_stacked_on_a_pin_startup_already_called_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, capture) = session(dir.path(), Arc::default());
    session.set_unverified_models(vec!["nonexistent-model-xyz".to_string()]);
    let (tx, lookup) = crate::WindowLookup::channel();
    tx.send(Some(vec!["nonexistent-model-xyz".to_string()]))
        .unwrap();
    session.set_window_lookup(Some(lookup));
    session
        .settle_window_lookup("nonexistent-model-xyz", None)
        .await;
    assert!(
        warnings(&capture).is_empty(),
        "one warning about a model the person is about to replace is enough: {:?}",
        warnings(&capture)
    );

    // A model startup did not flag still gets its window notice.
    let (tx, lookup) = crate::WindowLookup::channel();
    tx.send(Some(vec!["gateway::model".to_string()])).unwrap();
    session.set_window_lookup(Some(lookup));
    session.settle_window_lookup(UNKNOWN, None).await;
    assert_eq!(warnings(&capture).len(), 1);
}

#[tokio::test]
async fn a_lookup_that_found_every_window_or_died_says_nothing_and_never_hangs() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, capture) = session(dir.path(), Arc::default());
    fill_past_a_quarter_of_the_floor_trigger(&mut session);

    let (tx, lookup) = crate::WindowLookup::channel();
    tx.send(Some(Vec::new())).unwrap();
    session.set_window_lookup(Some(lookup));
    session.settle_window_lookup(UNKNOWN, None).await;

    let (tx, lookup) = crate::WindowLookup::channel();
    drop(tx);
    session.set_window_lookup(Some(lookup));
    tokio::time::timeout(
        Duration::from_secs(2),
        session.settle_window_lookup(UNKNOWN, None),
    )
    .await
    .expect("a lookup whose task died must not hold the first request");

    assert!(warnings(&capture).is_empty());
}

#[tokio::test]
async fn a_new_session_pinned_to_one_claude_bridge_model_warms_that_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(WarmRecorder::default());
    let (mut session, _capture) = session(dir.path(), Arc::clone(&provider));
    session.pin_model(Some("claude-cli::haiku".to_string()));
    session.set_effort(Some(forge_types::EffortLevel::High));

    session.prewarm_pinned_bridge();

    let warmed = warmed_within(&provider, Duration::from_secs(2)).await;
    assert_eq!(
        warmed,
        [(
            "claude-cli::haiku".to_string(),
            session.id().to_string(),
            Some(forge_types::EffortLevel::High)
        )],
        "the warm-up must name the conversation and the rung its first turn will send"
    );
}

#[tokio::test]
async fn nothing_is_warmed_when_the_first_turn_would_not_use_it() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(WarmRecorder::default());
    let (mut session, _capture) = session(dir.path(), Arc::clone(&provider));

    session.prewarm_pinned_bridge();
    session.pin_model(Some("inferhub::cb/deepseek-v4.1-flash".to_string()));
    session.prewarm_pinned_bridge();
    session.pin_model(Some("claude-cli::haiku,claude-cli::sonnet".to_string()));
    session.prewarm_pinned_bridge();
    session.pin_model(Some("claude-cli::haiku".to_string()));
    session.transcript.push(Message::user("an earlier turn"));
    session.prewarm_pinned_bridge();

    assert!(
        warmed_within(&provider, Duration::from_millis(200))
            .await
            .is_empty(),
        "unpinned, non-bridge, set-pinned and resumed conversations are not warmed"
    );
}

#[tokio::test]
async fn a_prompt_is_held_back_only_while_the_whole_pin_is_known_uncallable() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _capture) = session(dir.path(), Arc::default());
    session.pin_model(Some("nonexistent-model-xyz".to_string()));
    assert_eq!(session.unavailable_pin(), None, "nothing is known yet");
    session.set_unavailable_models(vec!["nonexistent-model-xyz".to_string()]);
    assert_eq!(session.unavailable_pin(), Some("nonexistent-model-xyz"));
    session.pin_model(Some("nonexistent-model-xyz, groq::llama".to_string()));
    assert_eq!(
        session.unavailable_pin(),
        None,
        "a set with a callable member still sends"
    );
    session.pin_model(Some("groq::llama".to_string()));
    assert_eq!(session.unavailable_pin(), None, "picking another lifts it");
    session.pin_model(None);
    assert_eq!(session.unavailable_pin(), None, "no pin, nothing to refuse");
}
