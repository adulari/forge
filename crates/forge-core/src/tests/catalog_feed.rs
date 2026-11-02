//! A session follows a background-refreshed catalog without touching its pin, and tells the user
//! about model-set changes only when they are next active.

use super::*;
use forge_mesh::{LiveCatalog, ModelCatalog};

fn session_with(catalog: ModelCatalog) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let presenter = CapturePresenter::default();
    let events = presenter.events.clone();
    let config = Config::default();
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(presenter),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    session.set_catalog(Some(catalog));
    (session, events)
}

fn notes(events: &Arc<Mutex<Vec<PresenterEvent>>>) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Progress(text) if text.starts_with("models updated") => {
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
}

fn catalog(ids: &[&str]) -> ModelCatalog {
    ModelCatalog::new(ids.iter().map(|s| s.to_string()).collect())
}

#[test]
fn a_published_catalog_replaces_the_snapshot_silently_and_keeps_the_pin() {
    let feed = LiveCatalog::default();
    let (mut session, events) = session_with(catalog(&["groq::a"]));
    session.set_catalog_feed(feed.clone());
    session.pin_model(Some("groq::a".into()));

    assert!(!session.sync_catalog_feed(), "nothing published yet");

    feed.publish(catalog(&[
        "groq::a",
        "modelgateway::glm",
        "modelgateway::kimi",
    ]));
    assert!(session.sync_catalog_feed());
    let ids = session.catalog().unwrap().models();
    assert!(
        ids.iter().any(|m| m == "modelgateway::glm"),
        "adopted immediately"
    );
    assert_eq!(session.pinned_model(), Some(&["groq::a".to_string()][..]));
    assert!(notes(&events).is_empty(), "adoption itself never speaks");

    assert!(!session.sync_catalog_feed(), "already adopted");
}

#[test]
fn a_catalog_published_before_the_session_attached_is_not_announced() {
    let feed = LiveCatalog::default();
    feed.publish(catalog(&["groq::a"]));
    let (mut session, events) = session_with(catalog(&["groq::a"]));
    session.set_catalog_feed(feed);
    assert!(!session.sync_catalog_feed());
    assert_eq!(session.announce_catalog_changes(), None);
    assert!(notes(&events).is_empty());
}

#[test]
fn a_re_ranked_catalog_with_the_same_models_is_silent() {
    let feed = LiveCatalog::default();
    let (mut session, events) = session_with(catalog(&["groq::a"]));
    session.set_catalog_feed(feed.clone());
    feed.publish(catalog(&["groq::a"]));
    assert!(session.sync_catalog_feed());
    assert_eq!(session.announce_catalog_changes(), None);
    assert!(notes(&events).is_empty());
}

#[test]
fn flapping_listings_produce_no_notes_and_no_net_change() {
    let feed = LiveCatalog::default();
    let base = ["sambanova::a", "sambanova::b", "inferhub::c"];
    let (mut session, events) = session_with(catalog(&base));
    session.set_catalog_feed(feed.clone());
    for _ in 0..100 {
        feed.publish(catalog(&["inferhub::c"]));
        session.sync_catalog_feed();
        feed.publish(catalog(&base));
        session.sync_catalog_feed();
    }
    assert_eq!(session.announce_catalog_changes(), None);
    assert!(notes(&events).is_empty());
}

#[test]
fn flapping_that_ends_on_a_net_change_is_one_line_with_the_net_delta() {
    let feed = LiveCatalog::default();
    let (mut session, _events) = session_with(catalog(&["sambanova::a"]));
    session.set_catalog_feed(feed.clone());
    for _ in 0..50 {
        feed.publish(catalog(&["sambanova::a", "sambanova::b", "sambanova::c"]));
        session.sync_catalog_feed();
        feed.publish(catalog(&["sambanova::a"]));
        session.sync_catalog_feed();
    }
    feed.publish(catalog(&["sambanova::a", "sambanova::b"]));
    session.sync_catalog_feed();
    assert_eq!(
        session.announce_catalog_changes().as_deref(),
        Some("models updated: +1 sambanova")
    );
    assert_eq!(session.announce_catalog_changes(), None, "told once");
}

#[tokio::test]
async fn a_real_addition_is_announced_once_at_the_next_turn_not_while_idle() {
    let feed = LiveCatalog::default();
    let (mut session, events) = session_with(catalog(&["groq::a"]));
    session.set_catalog_feed(feed.clone());

    feed.publish(catalog(&[
        "groq::a",
        "modelgateway::glm",
        "modelgateway::kimi",
    ]));
    session.sync_catalog_feed();
    assert!(notes(&events).is_empty(), "idle: nothing is emitted");

    session.run_turn("hello").await.unwrap();
    assert_eq!(notes(&events), vec!["models updated: +2 modelgateway"]);

    session.run_turn("again").await.unwrap();
    assert_eq!(notes(&events).len(), 1, "never repeated");
}
