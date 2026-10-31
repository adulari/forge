//! The fleet socket (`WS /ws/fleet`) and the signals the daemon shares with every open socket:
//! how many apps are attached, and whether the daemon is about to go away.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;

use crate::remote;
use crate::serve::DaemonState;

/// Counters and flags every socket handler reads. Owned by the session registry.
pub(crate) struct HostSignals {
    /// Flips to `true` once the daemon is about to exit, so each open socket can tell its client
    /// the host is going away (close code 1001) instead of just vanishing.
    going_away: tokio::sync::watch::Sender<bool>,
    /// Fleet sockets open right now; with the per-session counts this says whether any app is
    /// connected, which `[power] keep_awake = "while_remote"` keys on.
    fleet_clients: Arc<AtomicUsize>,
    /// Bumped on every project-notes write (`serve_notes`), so a second device refetches them.
    notes_changed: tokio::sync::watch::Sender<u64>,
}

impl HostSignals {
    pub(crate) fn new() -> Self {
        Self {
            going_away: tokio::sync::watch::channel(false).0,
            fleet_clients: Arc::new(AtomicUsize::new(0)),
            notes_changed: tokio::sync::watch::channel(0).0,
        }
    }

    pub(crate) fn notify_notes(&self) {
        self.notes_changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub(crate) fn notes_rx(&self) -> tokio::sync::watch::Receiver<u64> {
        self.notes_changed.subscribe()
    }

    pub(crate) fn announce_going_away(&self) {
        self.going_away.send_replace(true);
    }

    pub(crate) fn going_away_rx(&self) -> tokio::sync::watch::Receiver<bool> {
        self.going_away.subscribe()
    }

    pub(crate) fn fleet_client_count(&self) -> usize {
        self.fleet_clients.load(Ordering::Relaxed)
    }
}

/// Counts one attached fleet socket for as long as it lives.
struct FleetClient(Arc<AtomicUsize>);

impl FleetClient {
    fn attach(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for FleetClient {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) async fn fleet_ws_handler(
    State(state): State<Arc<DaemonState>>,
    ws: WebSocketUpgrade,
) -> Response {
    let fleet_rx = state.registry.subscribe_fleet();
    let going_away = state.registry.signals.going_away_rx();
    let notes_rx = state.registry.signals.notes_rx();
    let attached = FleetClient::attach(&state.registry.signals.fleet_clients);
    ws.on_upgrade(move |socket| async move {
        let _attached = attached;
        pump_fleet_ws(socket, fleet_rx, notes_rx, going_away).await;
    })
}

async fn pump_fleet_ws(
    mut socket: WebSocket,
    mut fleet_rx: tokio::sync::watch::Receiver<u64>,
    mut notes_rx: tokio::sync::watch::Receiver<u64>,
    mut going_away: tokio::sync::watch::Receiver<bool>,
) {
    let initial = *fleet_rx.borrow();
    if socket
        .send(WsMessage::Text(
            serde_json::json!({ "kind": "fleet_changed", "revision": initial })
                .to_string()
                .into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(25));
    keepalive.tick().await;
    loop {
        tokio::select! {
            changed = fleet_rx.changed() => {
                if changed.is_err() { break; }
                let revision = *fleet_rx.borrow_and_update();
                let frame = serde_json::json!({ "kind": "fleet_changed", "revision": revision }).to_string();
                if socket.send(WsMessage::Text(frame.into())).await.is_err() { break; }
            }
            changed = notes_rx.changed() => {
                if changed.is_err() { break; }
                let revision = *notes_rx.borrow_and_update();
                let frame = serde_json::json!({ "kind": "notes_changed", "revision": revision }).to_string();
                if socket.send(WsMessage::Text(frame.into())).await.is_err() { break; }
            }
            _ = keepalive.tick() => {
                if socket.send(WsMessage::Ping(Vec::new().into())).await.is_err() { break; }
            }
            _ = remote::wait_going_away(&mut going_away) => {
                let _ = socket.send(remote::host_restarting_close()).await;
                break;
            }
            // A fleet socket only receives, so without reading it a vanished client would go
            // unnoticed until the next change or ping.
            incoming = socket.recv() => {
                if matches!(incoming, None | Some(Err(_)) | Some(Ok(WsMessage::Close(_)))) {
                    break;
                }
            }
        }
    }
}
