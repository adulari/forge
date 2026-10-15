//! `[mesh] vision_fallback`: a request that carries an image and lands on a text-only model, and
//! routing that keeps following the image after the turn that introduced it.

use super::*;
use forge_config::VisionFallback;

const TEXT_ONLY: &str = "ollama::deepseek-v3";
const VISION: &str = "ollama::llava:13b";

#[derive(Default)]
struct Seen {
    /// (model, images on the latest image turn, text of the last user message)
    main: Vec<(String, usize, String)>,
    describes: usize,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Seen>,
}

#[async_trait::async_trait]
impl Provider for Recorder {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        unreachable!("the loop calls complete_with")
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _opts: &CompletionOptions,
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let mut seen = self.seen.lock().unwrap();
        if messages
            .first()
            .is_some_and(|m| m.content.starts_with("You describe images"))
        {
            seen.describes += 1;
            return Ok(forge_provider::ModelResponse {
                content: "A red square labelled HELLO-WORLD".into(),
                ..Default::default()
            });
        }
        if messages
            .first()
            .is_some_and(|m| m.content.starts_with("You are"))
        {
            let images =
                forge_provider::last_image_turn(messages).map_or(0, |i| messages[i].images.len());
            let last = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, Role::User))
                .map(|m| m.content.clone())
                .unwrap_or_default();
            seen.main.push((model.to_string(), images, last));
        }
        Ok(forge_provider::ModelResponse {
            content: "Done.".into(),
            ..Default::default()
        })
    }
}

/// Picks the vision model only when told the request carries an image.
struct VisionRouter;

#[async_trait::async_trait]
impl Router for VisionRouter {
    async fn route(
        &self,
        _prompt: &str,
        has_images: bool,
        _budget: BudgetState,
        _health: &forge_types::ModelHealth,
        _quota: &forge_types::SubscriptionQuota,
        _effort: Option<forge_types::EffortLevel>,
        _project: &forge_types::ProjectContext,
    ) -> forge_mesh::RoutingDecision {
        forge_mesh::RoutingDecision {
            effort: None,
            tier: forge_types::TaskTier::Trivial,
            model: if has_images { VISION } else { TEXT_ONLY }.into(),
            rationale: "test".into(),
            fallbacks: vec![],
            pinned: false,
            unroutable: false,
        }
    }
}

fn session(
    provider: Arc<Recorder>,
    mode: VisionFallback,
) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.vision_fallback = mode;
    let capture = CapturePresenter {
        attended: true,
        ..CapturePresenter::default()
    };
    let events = capture.events.clone();
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(VisionRouter),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.set_catalog(Some(ModelCatalog::new(vec![
        TEXT_ONLY.into(),
        VISION.into(),
    ])));
    (session, events)
}

fn png() -> forge_types::ImageAttachment {
    forge_types::ImageAttachment {
        media_type: "image/png".into(),
        data_base64: "iVBORw0KGgo=".into(),
    }
}

fn warnings(events: &Arc<Mutex<Vec<PresenterEvent>>>, needle: &str) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains(needle)))
        .count()
}

#[tokio::test]
async fn a_follow_up_with_no_new_image_still_routes_to_a_vision_model() {
    let provider = Arc::new(Recorder::default());
    let (mut session, _) = session(provider.clone(), VisionFallback::Warn);
    session.attach_images(vec![png()]);
    session.run_turn("what is in this picture").await.unwrap();
    session.run_turn("and what colour is it").await.unwrap();
    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.main.len(), 2);
    assert_eq!(seen.main[0].0, VISION);
    assert_eq!(
        seen.main[1].0, VISION,
        "the image is still on the wire for turn 2"
    );
    assert_eq!(seen.main[1].1, 1);
}

#[tokio::test]
async fn warn_sends_text_only_and_says_so_once() {
    let provider = Arc::new(Recorder::default());
    let (mut session, events) = session(provider.clone(), VisionFallback::Warn);
    session.pin_model(Some(TEXT_ONLY.into()));
    session.attach_images(vec![png()]);
    session.run_turn("what is in this picture").await.unwrap();
    session.run_turn("again").await.unwrap();
    let seen = provider.seen.lock().unwrap();
    assert!(seen
        .main
        .iter()
        .all(|(m, images, _)| m == TEXT_ONLY && *images == 0));
    assert_eq!(seen.describes, 0);
    assert_eq!(warnings(&events, "can't read images; sent text only"), 1);
    assert_eq!(warnings(&events, "mesh.vision_fallback"), 1);
}

#[tokio::test]
async fn describe_replaces_the_image_with_one_cached_description() {
    let provider = Arc::new(Recorder::default());
    let (mut session, events) = session(provider.clone(), VisionFallback::Describe);
    session.pin_model(Some(TEXT_ONLY.into()));
    session.attach_images(vec![png()]);
    session.run_turn("what does the sign say").await.unwrap();
    session.run_turn("and below it").await.unwrap();
    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.main.len(), 2);
    for (model, images, _) in &seen.main {
        assert_eq!(model, TEXT_ONLY);
        assert_eq!(*images, 0);
    }
    assert_eq!(seen.describes, 1, "described once, not once per request");
    let transcript_text: String = seen.main[0].2.clone();
    assert!(transcript_text.contains("HELLO-WORLD"), "{transcript_text}");
    assert_eq!(warnings(&events, "can't read images"), 0);
}

#[tokio::test]
async fn describe_without_a_vision_model_falls_back_to_the_warning() {
    let provider = Arc::new(Recorder::default());
    let (mut session, events) = session(provider.clone(), VisionFallback::Describe);
    session.set_catalog(Some(ModelCatalog::new(vec![TEXT_ONLY.into()])));
    session.pin_model(Some(TEXT_ONLY.into()));
    session.attach_images(vec![png()]);
    session.run_turn("what does the sign say").await.unwrap();
    assert_eq!(provider.seen.lock().unwrap().describes, 0);
    assert_eq!(warnings(&events, "no vision model was available"), 1);
}

#[tokio::test]
async fn switch_leaves_the_pin_for_a_vision_model_for_that_request() {
    let provider = Arc::new(Recorder::default());
    let (mut session, events) = session(provider.clone(), VisionFallback::Switch);
    session.pin_model(Some(TEXT_ONLY.into()));
    session.attach_images(vec![png()]);
    session.run_turn("what is in this picture").await.unwrap();
    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.main[0].0, VISION);
    assert_eq!(seen.main[0].1, 1, "the vision model receives the image");
    assert_eq!(warnings(&events, "routed this request to"), 1);
}

#[tokio::test]
async fn a_pin_is_kept_when_no_image_is_involved() {
    let provider = Arc::new(Recorder::default());
    let (mut session, events) = session(provider.clone(), VisionFallback::Switch);
    session.pin_model(Some(TEXT_ONLY.into()));
    session.run_turn("plain text question").await.unwrap();
    assert_eq!(provider.seen.lock().unwrap().main[0].0, TEXT_ONLY);
    assert_eq!(warnings(&events, "can't read images"), 0);
}
