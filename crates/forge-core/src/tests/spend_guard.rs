//! A runaway turn used to be invisible: the day/week/month caps are off by default and are read
//! once, when a turn starts. One resumed session reached $186 across ~1,800 calls of a 214k-token
//! average prompt with no guard ever speaking, and every side call (compaction included) was
//! recorded at $0.

use super::*;
use crate::spend_guard::{level, Level};
use forge_config::PriceOverride;

const MODEL: &str = "ollama::spend-test";

/// $1.00 per call for the 1,000 input tokens `EndlessToolProvider::new(1000)` reports.
fn priced_config() -> Config {
    let mut config = Config::default();
    config.mesh.max_steps = 12;
    config.mesh.max_steps_unattended = 12;
    config.mesh.max_turn_input_tokens = 0;
    config.mesh.pricing.insert(
        MODEL.into(),
        PriceOverride {
            input_per_1k: 1.0,
            output_per_1k: 0.0,
            cache_read_per_1k: None,
        },
    );
    config
}

fn session_with(
    store: &Arc<Store>,
    provider: impl Provider + 'static,
    config: Config,
) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let capture = CapturePresenter {
        attended: true,
        ..CapturePresenter::default()
    };
    let events = capture.events.clone();
    let mut session = Session::start(
        Arc::clone(store),
        Arc::new(provider),
        Arc::new(FixedRouter {
            model: MODEL.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    session.set_catalog(Some(ModelCatalog::new(vec![MODEL.into()])));
    (session, events)
}

fn warnings(events: &[PresenterEvent], needle: &str) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains(needle)))
        .count()
}

#[test]
fn thresholds_warn_then_stop_and_zero_disables() {
    assert_eq!(level(5.0, 25.0, true, 4.99), Level::Ok);
    assert_eq!(level(5.0, 25.0, true, 5.0), Level::Warn);
    assert_eq!(level(5.0, 25.0, true, 25.0), Level::Stop);
    assert_eq!(
        level(5.0, 0.0, true, 1e6),
        Level::Warn,
        "no cap, still warns"
    );
    assert_eq!(level(0.0, 0.0, true, 1e6), Level::Ok, "0 switches both off");
    assert_eq!(
        level(5.0, 25.0, false, 400.0),
        Level::Warn,
        "hard_stop = false keeps the cap but only ever warns"
    );
}

#[tokio::test]
async fn a_turn_is_ended_at_its_dollar_cap_after_one_warning() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = priced_config();
    config.mesh.budget.turn_warn_usd = 2.0;
    config.mesh.budget.turn_cap_usd = 4.0;
    config.mesh.budget.session_warn_usd = 0.0;
    config.mesh.budget.session_cap_usd = 0.0;
    let (provider, _calls) = EndlessToolProvider::new(1000);
    let (mut session, events) = session_with(&store, provider, config);

    session.run_turn("keep reading").await.unwrap();

    let events = events.lock().unwrap();
    assert_eq!(
        loop_steps(&events),
        4,
        "the fourth $1.00 call reaches the $4.00 cap; the step cap (12) is nowhere near"
    );
    assert_eq!(
        warnings(&events, "this turn has spent"),
        1,
        "one warning per turn, not one per call"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        PresenterEvent::Error(w)
            if w.contains("turn spend cap") && w.contains("$4.00") && w.contains("turn_cap_usd")
    )));
}

#[tokio::test]
async fn the_next_turn_gets_a_fresh_allowance() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = priced_config();
    config.mesh.budget.turn_warn_usd = 0.0;
    config.mesh.budget.turn_cap_usd = 3.0;
    config.mesh.budget.session_warn_usd = 0.0;
    config.mesh.budget.session_cap_usd = 0.0;
    let (provider, _calls) = EndlessToolProvider::new(1000);
    let (mut session, events) = session_with(&store, provider, config);

    session.run_turn("keep reading").await.unwrap();
    session.run_turn("continue").await.unwrap();

    assert_eq!(
        loop_steps(&events.lock().unwrap()),
        6,
        "`continue` is the confirmation: three more calls, not zero"
    );
}

#[tokio::test]
async fn a_session_over_its_cap_refuses_new_turns() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = priced_config();
    config.mesh.budget.turn_warn_usd = 0.0;
    config.mesh.budget.turn_cap_usd = 0.0;
    config.mesh.budget.session_warn_usd = 1.0;
    config.mesh.budget.session_cap_usd = 2.5;
    let (provider, _calls) = EndlessToolProvider::new(1000);
    let (mut session, events) = session_with(&store, provider, config);

    session.run_turn("keep reading").await.unwrap();
    let after_first = loop_steps(&events.lock().unwrap());
    assert_eq!(after_first, 3, "$3.00 is the first total at or past $2.50");

    session.run_turn("one more").await.unwrap();

    let events = events.lock().unwrap();
    assert_eq!(
        loop_steps(&events),
        after_first,
        "no provider call once the session is at its cap"
    );
    assert_eq!(warnings(&events, "this session has spent"), 1);
    assert!(events.iter().any(|e| matches!(
        e,
        PresenterEvent::Error(w) if w.contains("session spend cap")
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        PresenterEvent::Done {
            stop_reason: StopReason::BudgetExhausted,
            ..
        }
    )));
}

#[tokio::test]
async fn without_hard_stop_the_caps_only_warn() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = priced_config();
    config.mesh.max_steps = 6;
    config.mesh.max_steps_unattended = 6;
    config.mesh.budget.hard_stop = false;
    config.mesh.budget.turn_warn_usd = 1.0;
    config.mesh.budget.turn_cap_usd = 2.0;
    config.mesh.budget.session_cap_usd = 0.0;
    let (provider, _calls) = EndlessToolProvider::new(1000);
    let (mut session, events) = session_with(&store, provider, config);

    session.run_turn("keep reading").await.unwrap();

    let events = events.lock().unwrap();
    assert_eq!(loop_steps(&events), 6, "ran to the step cap");
    assert!(warnings(&events, "this turn has spent") >= 1);
}

struct UsageProvider;

#[async_trait::async_trait]
impl Provider for UsageProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "nothing durable".into(),
            tool_calls: vec![],
            usage: forge_types::Usage {
                input_tokens: 1000,
                ..forge_types::Usage::default()
            },
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn side_calls_are_priced_into_the_session_total() {
    let mut config = priced_config();
    config.mesh.auto_memory = true;
    config.mesh.models.insert(
        TaskTier::Trivial.as_str().to_string(),
        forge_config::OneOrMany::Many(vec![MODEL.into()]),
    );
    let store = Arc::new(Store::open_in_memory().unwrap());
    let (mut session, _events) = session_with(&store, UsageProvider, config);

    let handle = session
        .capture_memories("remember this", "noted")
        .await
        .expect("a usable side-call model exists");
    handle.await.unwrap();

    let cost = store.session_cost(&session.id).unwrap();
    assert!(
        (cost - 1.0).abs() < 1e-9,
        "1,000 input tokens at $1.00/1k must reach the session total, got {cost}"
    );
}
