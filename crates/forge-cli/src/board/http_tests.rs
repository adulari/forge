//! The board's client against a REAL HTTP/WebSocket server on an ephemeral port, rather than
//! fixture strings. What these pin down is the part fixtures cannot: that the URLs the board
//! builds are the ones a token-scoped router actually answers, that a wrong token reads as a
//! rejected token, that a daemon's `{"error": …}` reaches the user verbatim, and that a burst of
//! `fleet_changed` frames collapses into one refetch.
//!
//! `forge serve`'s own `daemon_router` is not reachable from here — `DaemonState` has private
//! fields (`base`, `mock`, `project_roots`, `voice`, `anywhere_enable`) that only `serve.rs` can
//! fill — so the fixture is a stand-in router serving the same routes and the same shapes. Route
//! and payload agreement with the daemon is covered where it belongs: in `serve.rs`'s own tests,
//! and by `wire.rs`'s `Deserialize` views of the daemon's `Serialize` types.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use axum::Router;
use forge_tui::board::{BoardEvent, ConnState, ToastLevel};
use tokio::sync::mpsc;

use super::client::{fetch_fleet, fetch_past, fleet_refresher, post_action};
use super::sockets::fleet_watcher;
use super::Ev;

const TOKEN: &str = "tok";

#[derive(Default)]
struct Fixture {
    /// How many times `GET /api/sessions` was served — the debounce assertion's counter.
    fleet_hits: AtomicUsize,
    /// `fleet_changed` frames every accepted `/ws/fleet` client is sent immediately.
    fleet_frames: usize,
}

/// Bind an ephemeral port and serve the daemon's routes. Returns the base URL; the server task is
/// detached and dies with the test's runtime.
async fn serve(fixture: Arc<Fixture>) -> String {
    let app = Router::new()
        .route(
            &format!("/{TOKEN}/api/sessions"),
            get(sessions).post(create),
        )
        .route(&format!("/{TOKEN}/api/sessions/past"), get(past))
        .route(
            &format!("/{TOKEN}/api/sessions/{{id}}/interrupt"),
            post(interrupt),
        )
        .route(&format!("/{TOKEN}/ws/fleet"), get(fleet_ws))
        .with_state(fixture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn sessions(State(fixture): State<Arc<Fixture>>) -> Response {
    fixture.fleet_hits.fetch_add(1, Ordering::SeqCst);
    json(serde_json::json!([{
        "id": "sess-1",
        "title": "fix the parser",
        "cwd": "/repo",
        "busy": true,
        "waiting": false,
        "model": "anthropic::claude-opus-5",
        "cost_usd": 0.25,
        "context_tokens": 12_000,
        "read_only": false,
        "terminal": false,
        "a_field_from_a_newer_daemon": 1
    }]))
}

async fn past() -> Response {
    json(serde_json::json!([{
        "id": "old-1",
        "title": "last week",
        "cwd": "/repo",
        "archived": false,
        "message_count": 40
    }]))
}

async fn create() -> Response {
    json(serde_json::json!({ "id": "sess-new", "title": "", "cwd": "/repo" }))
}

/// The daemon's real refusal for a session whose driver is winding down.
async fn interrupt() -> Response {
    (
        axum::http::StatusCode::CONFLICT,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "error": "session driver is no longer accepting input (it is shutting down)"
        })
        .to_string(),
    )
        .into_response()
}

async fn fleet_ws(State(fixture): State<Arc<Fixture>>, ws: WebSocketUpgrade) -> Response {
    let frames = fixture.fleet_frames;
    ws.on_upgrade(move |socket| push_fleet_frames(socket, frames))
}

async fn push_fleet_frames(mut socket: WebSocket, frames: usize) {
    for revision in 0..frames {
        let frame = serde_json::json!({ "kind": "fleet_changed", "revision": revision });
        if socket
            .send(WsMessage::Text(frame.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }
    // A frame the board must ignore, proving the refetch is driven by `kind`, not by traffic.
    let _ = socket
        .send(WsMessage::Text(r#"{"keepalive":true}"#.into()))
        .await;
    std::future::pending::<()>().await;
}

fn json(value: serde_json::Value) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

#[tokio::test]
async fn the_fleet_and_past_routes_answer_the_urls_the_board_builds() {
    let base = serve(Arc::new(Fixture::default())).await;
    let http = reqwest::Client::new();

    let fleet = fetch_fleet(&http, &base, TOKEN).await.unwrap();
    assert_eq!(fleet.len(), 1);
    assert_eq!(fleet[0].id, "sess-1");
    assert_eq!(fleet[0].title, "fix the parser");
    assert!(fleet[0].busy);
    assert_eq!(fleet[0].model, "anthropic::claude-opus-5");
    assert!(!fleet[0].read_only, "a drivable session gets a socket");

    let past = fetch_past(&http, &base, TOKEN, 30).await.unwrap();
    assert_eq!(past.len(), 1);
    assert_eq!(past[0].id, "old-1");
    assert_eq!(past[0].message_count, 40);
}

#[tokio::test]
async fn a_wrong_token_reads_as_a_rejected_token_not_a_missing_daemon() {
    // Every route is under `/<token>/`, so a bad token is indistinguishable from a bad path at the
    // HTTP layer — the message has to name the cause the user can act on.
    let base = serve(Arc::new(Fixture::default())).await;
    let http = reqwest::Client::new();
    let error = fetch_fleet(&http, &base, "wrong-token")
        .await
        .expect_err("a 404 is a rejection, not an empty fleet")
        .to_string();
    assert!(error.contains("token"), "{error}");
}

#[tokio::test]
async fn a_dead_daemon_says_how_to_start_one() {
    let http = reqwest::Client::new();
    // Port 1 on loopback refuses instantly; nothing is listening in a test environment.
    let error = fetch_fleet(&http, "http://127.0.0.1:1", TOKEN)
        .await
        .expect_err("a refused connection is an error")
        .to_string();
    assert!(error.contains("forge serve"), "{error}");
}

#[tokio::test]
async fn a_failed_action_reports_the_daemons_own_reason() {
    let base = serve(Arc::new(Fixture::default())).await;
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel();
    post_action(
        reqwest::Client::new(),
        format!("{base}/{TOKEN}/api/sessions/sess-1/interrupt"),
        serde_json::json!({}),
        "interrupted sess-1".into(),
        ev_tx,
        refresh_tx,
    )
    .await;

    match ev_rx.try_recv() {
        Ok(Ev::Board(BoardEvent::Toast(ToastLevel::Error, text))) => assert_eq!(
            text,
            "session driver is no longer accepting input (it is shutting down)"
        ),
        other => panic!("expected the daemon's own reason; got {:?}", other.is_ok()),
    }
    assert!(
        refresh_rx.try_recv().is_ok(),
        "an action always re-reads the fleet, successful or not"
    );
}

#[tokio::test]
async fn an_invalidation_burst_costs_one_refetch() {
    let fixture = Arc::new(Fixture {
        fleet_frames: 5,
        ..Fixture::default()
    });
    let base = serve(fixture.clone()).await;
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let (refresh_tx, refresh_rx) = mpsc::unbounded_channel();

    let watcher = tokio::spawn(fleet_watcher(
        base.clone(),
        TOKEN.into(),
        refresh_tx,
        ev_tx.clone(),
    ));
    let refresher = tokio::spawn(fleet_refresher(
        reqwest::Client::new(),
        base,
        TOKEN.into(),
        refresh_rx,
        ev_tx,
    ));

    // Long enough for the socket, the 300 ms debounce and the refetch; far short of the 15 s
    // fallback poll, so anything counted here came from the burst.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    watcher.abort();
    refresher.abort();

    assert_eq!(
        fixture.fleet_hits.load(Ordering::SeqCst),
        1,
        "five fleet_changed frames are one refetch"
    );

    let mut fleets = 0;
    let mut live = false;
    while let Ok(event) = ev_rx.try_recv() {
        match event {
            Ev::Board(BoardEvent::Fleet(rows)) => {
                assert_eq!(rows.len(), 1);
                fleets += 1;
            }
            Ev::Board(BoardEvent::Connection(ConnState::Live)) => live = true,
            _ => {}
        }
    }
    assert_eq!(fleets, 1);
    assert!(live, "an open fleet socket is the board's Live signal");
}
