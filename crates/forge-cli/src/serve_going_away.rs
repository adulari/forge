//! A daemon that is restarting tells its open sockets so (close code 1001), instead of letting
//! them drop and read as "unreachable".

use std::sync::Arc;

use futures::StreamExt;
use tokio_tungstenite::tungstenite::Message;

use crate::serve::{daemon_router, DaemonState, SessionRegistry};

fn state() -> Arc<DaemonState> {
    Arc::new(DaemonState {
        registry: Arc::new(SessionRegistry::new()),
        terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
        base: "/tok".into(),
        mock: true,
        default_cwd: "/tmp".into(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    })
}

async fn serve(state: Arc<DaemonState>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = daemon_router(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fleet_socket_gets_a_1001_close_when_the_daemon_announces_it_is_going_away() {
    let state = state();
    let addr = serve(state.clone()).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/tok/ws/fleet"))
        .await
        .expect("fleet socket upgrades");
    let first = ws.next().await.expect("a frame").expect("readable");
    assert!(matches!(first, Message::Text(_)), "the opening fleet frame");

    state.registry.signals.announce_going_away();

    let close = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(frame))) => return frame,
                Some(Ok(_)) => continue,
                other => panic!("socket ended without a close frame: {other:?}"),
            }
        }
    })
    .await
    .expect("the close frame arrives");
    let frame = close.expect("a close frame with a code");
    assert_eq!(u16::from(frame.code), 1001);
    assert_eq!(frame.reason.as_str(), "host restarting");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fleet_socket_counts_as_an_attached_client_only_while_open() {
    let state = state();
    let addr = serve(state.clone()).await;
    assert_eq!(state.registry.signals.fleet_client_count(), 0);
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/tok/ws/fleet"))
        .await
        .unwrap();
    let _ = ws.next().await;
    assert_eq!(state.registry.signals.fleet_client_count(), 1);
    drop(ws);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.registry.signals.fleet_client_count() != 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the count never fell back"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
