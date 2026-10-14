//! A session follows a background-refreshed catalog without touching its pin.

use super::*;
use forge_mesh::{LiveCatalog, ModelCatalog};

fn session_with(catalog: ModelCatalog) -> Session {
    let mut session = fresh_session(
        Arc::new(Store::open_in_memory().unwrap()),
        Config::default(),
    );
    session.set_catalog(Some(catalog));
    session
}

#[test]
fn a_published_catalog_replaces_the_snapshot_once_and_keeps_the_pin() {
    let feed = LiveCatalog::default();
    let mut session = session_with(ModelCatalog::new(vec!["groq::a".into()]));
    session.set_catalog_feed(feed.clone());
    session.pin_model(Some("groq::a".into()));

    assert_eq!(session.sync_catalog_feed(), None, "nothing published yet");

    feed.publish(ModelCatalog::new(vec![
        "groq::a".into(),
        "modelgateway::glm".into(),
        "modelgateway::kimi".into(),
    ]));
    assert_eq!(
        session.sync_catalog_feed().as_deref(),
        Some("catalog refreshed: +2 modelgateway")
    );
    let ids = session.catalog().unwrap().models();
    assert!(ids.iter().any(|m| m == "modelgateway::glm"));
    assert_eq!(session.pinned_model(), Some(&["groq::a".to_string()][..]));

    assert_eq!(session.sync_catalog_feed(), None, "already adopted");
}

#[test]
fn a_catalog_published_before_the_session_attached_is_not_announced() {
    let feed = LiveCatalog::default();
    feed.publish(ModelCatalog::new(vec!["groq::a".into()]));
    let mut session = session_with(ModelCatalog::new(vec!["groq::a".into()]));
    session.set_catalog_feed(feed);
    assert_eq!(session.sync_catalog_feed(), None);
}

#[test]
fn a_re_ranked_catalog_with_the_same_models_is_adopted_silently() {
    let feed = LiveCatalog::default();
    let mut session = session_with(ModelCatalog::new(vec!["groq::a".into()]));
    session.set_catalog_feed(feed.clone());
    feed.publish(ModelCatalog::new(vec!["groq::a".into()]));
    assert_eq!(session.sync_catalog_feed(), None);
}
