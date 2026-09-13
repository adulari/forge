//! The board's WebSocket half: the fleet invalidation stream (`/ws/fleet`) and one per-session
//! snapshot stream (`/ws?session=<id>&rev=<n>`) for every card the board is watching.
//!
//! The per-session sockets carry traffic in both directions — snapshots down, `RemoteInput` JSON
//! up — which is what makes the board a real client rather than a dashboard: answering a
//! permission prompt from a card goes over the exact same socket `forge attach` would use, with
//! the same `prompt_seq` echoed back, so a stale keypress can never approve a newer prompt.

use std::collections::HashMap;
use std::time::Duration;

use forge_tui::board::{BoardEvent, ConnState, LiveSnapshot, ToastLevel};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::client::ws_url;
use super::{set_conn, short_id, Ev};

/// Reconnect delays for the fleet socket, in seconds. The last one repeats forever: a daemon that
/// is down comes back eventually, and the board should notice without being restarted.
const FLEET_BACKOFF: [u64; 4] = [1, 2, 5, 10];
/// A session socket is far less patient than the fleet socket: the session it belongs to is
/// probably gone (archived, or the daemon restarted), and the fleet refresh is the authority on
/// that. After this many failed reconnects the card goes quiet instead of retrying forever.
const SESSION_RETRIES: u32 = 3;
const SESSION_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Whether a frame from `/ws/fleet` is the invalidation signal (the daemon also sends pings, and
/// may add other control frames later).
pub(crate) fn is_fleet_changed(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v["kind"].as_str().map(|k| k == "fleet_changed"))
        .unwrap_or(false)
}

/// Decode a server-to-client frame from a session socket.
///
/// Snapshot fields all default so the board can watch an older daemon — which also means the
/// daemon's `{"keepalive":true}` would deserialize into an empty, idle snapshot and blank the card
/// every twenty seconds. A real snapshot always carries its session identity, so that is the test,
/// exactly as `forge attach` does it.
pub(crate) fn decode_snapshot(text: &str) -> Option<LiveSnapshot> {
    serde_json::from_str::<LiveSnapshot>(text)
        .ok()
        .filter(|snap| !snap.session_id.is_empty())
}

/// Watch `/ws/fleet` and ask for a refresh whenever the daemon says the fleet moved. This is the
/// board's only push channel for membership: without it a new session would take up to the
/// fallback interval to appear.
pub(crate) async fn fleet_watcher(
    base: String,
    token: String,
    refresh: mpsc::UnboundedSender<()>,
    ev: mpsc::UnboundedSender<Ev>,
) {
    let mut state: Option<ConnState> = None;
    let url = match ws_url(&base, &token, "ws/fleet") {
        Ok(url) => url,
        Err(e) => {
            set_conn(&ev, &mut state, ConnState::Offline(e.to_string()));
            return;
        }
    };
    let mut attempt = 0usize;
    loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok((mut socket, _)) => {
                attempt = 0;
                set_conn(&ev, &mut state, ConnState::Live);
                while let Some(message) = socket.next().await {
                    match message {
                        Ok(Message::Text(text)) => {
                            if is_fleet_changed(&text) && refresh.send(()).is_err() {
                                return;
                            }
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
                set_conn(&ev, &mut state, ConnState::Reconnecting);
            }
            Err(e) => {
                // The first failure is a blip; a second one is an outage worth naming.
                if attempt == 0 {
                    set_conn(&ev, &mut state, ConnState::Reconnecting);
                } else {
                    set_conn(
                        &ev,
                        &mut state,
                        ConnState::Offline(super::client::truncate(&e.to_string(), 90)),
                    );
                }
            }
        }
        let wait = FLEET_BACKOFF[attempt.min(FLEET_BACKOFF.len() - 1)];
        attempt = attempt.saturating_add(1);
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

/// One live socket per watched session, opened and closed to follow `BoardApp::watch_ids`.
pub(crate) struct SocketPool {
    base: String,
    token: String,
    ev: mpsc::UnboundedSender<Ev>,
    open: HashMap<String, Socket>,
}

struct Socket {
    input: mpsc::UnboundedSender<String>,
    task: tokio::task::JoinHandle<()>,
}

impl SocketPool {
    pub(crate) fn new(base: String, token: String, ev: mpsc::UnboundedSender<Ev>) -> Self {
        Self {
            base,
            token,
            ev,
            open: HashMap::new(),
        }
    }

    /// Open a socket for every id that gained one and drop the sockets of ids that went away.
    /// A task that gave up (it sent `SessionClosed` after its retries) is dropped too, so the
    /// board's next fleet refresh — which lifts the closed mark — gets a fresh attempt instead of
    /// a dead entry that looks open.
    pub(crate) fn sync(&mut self, ids: &[String]) {
        self.open.retain(|id, socket| {
            let keep = ids.iter().any(|want| want == id) && !socket.task.is_finished();
            if !keep {
                socket.task.abort();
            }
            keep
        });
        for id in ids {
            if self.open.contains_key(id) {
                continue;
            }
            let (input, rx) = mpsc::unbounded_channel();
            let task = tokio::spawn(session_socket(
                self.base.clone(),
                self.token.clone(),
                id.clone(),
                self.ev.clone(),
                rx,
            ));
            self.open.insert(id.clone(), Socket { input, task });
        }
    }

    pub(crate) fn open_ids(&self) -> Vec<String> {
        self.open.keys().cloned().collect()
    }

    /// Send one `RemoteInput` frame. A card whose socket is gone must SAY so — silently dropping a
    /// permission answer would look like the daemon ignored the user.
    pub(crate) fn send(&self, id: &str, json: &serde_json::Value) {
        let sent = self
            .open
            .get(id)
            .is_some_and(|socket| socket.input.send(json.to_string()).is_ok());
        if !sent {
            let _ = self.ev.send(Ev::Board(BoardEvent::Toast(
                ToastLevel::Error,
                format!("no live connection to {}", short_id(id)),
            )));
        }
    }

    pub(crate) fn shutdown(&mut self) {
        for (_, socket) in self.open.drain() {
            socket.task.abort();
        }
    }
}

/// One session's stream. Reconnects from the last revision it saw, so a blip replays only what was
/// missed instead of the whole bounded window.
async fn session_socket(
    base: String,
    token: String,
    id: String,
    ev: mpsc::UnboundedSender<Ev>,
    mut input: mpsc::UnboundedReceiver<String>,
) {
    let mut revision: u64 = 0;
    let mut failures: u32 = 0;
    loop {
        let url = match ws_url(&base, &token, &format!("ws?session={id}&rev={revision}")) {
            Ok(url) => url,
            Err(_) => break,
        };
        if let Ok((socket, _)) = tokio_tungstenite::connect_async(&url).await {
            let (mut tx, mut rx) = socket.split();
            loop {
                tokio::select! {
                    message = rx.next() => match message {
                        Some(Ok(Message::Text(text))) => {
                            let Some(snapshot) = decode_snapshot(&text) else { continue };
                            // A frame that arrived is proof the session is alive; the retry
                            // budget is for a socket that never comes back, not for a long one.
                            failures = 0;
                            if let Some(next) = snapshot.revision {
                                revision = next;
                            }
                            if ev.send(Ev::Board(BoardEvent::Snapshot(id.clone(), snapshot))).is_err() {
                                return;
                            }
                        }
                        Some(Ok(_)) => {}
                        _ => break,
                    },
                    line = input.recv() => match line {
                        // The pool dropped this session: stop, and say nothing (the card is gone).
                        None => return,
                        Some(line) => {
                            if tx.send(Message::Text(line.into())).await.is_err() {
                                break;
                            }
                        }
                    },
                }
            }
        }
        failures += 1;
        if failures > SESSION_RETRIES {
            let _ = ev.send(Ev::Board(BoardEvent::SessionClosed(id)));
            return;
        }
        tokio::time::sleep(SESSION_RETRY_DELAY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_invalidation_frame_triggers_a_refetch() {
        assert!(is_fleet_changed(r#"{"kind":"fleet_changed","revision":7}"#));
        assert!(!is_fleet_changed(r#"{"kind":"something_else"}"#));
        assert!(!is_fleet_changed(r#"{"keepalive":true}"#));
        assert!(!is_fleet_changed("not json at all"));
    }

    #[test]
    fn a_keepalive_is_not_an_empty_snapshot() {
        // Every field defaults, so this parses — and would blank the card if it were accepted.
        assert!(decode_snapshot(r#"{"keepalive":true}"#).is_none());
        assert!(decode_snapshot("}{").is_none());
    }

    #[test]
    fn a_real_frame_parses_with_the_fields_the_board_renders() {
        let frame = serde_json::json!({
            "protocol": 9,
            "session_id": "abc123",
            "title": "fix parser",
            "cwd": "/repo",
            "model": "sonnet",
            "busy": true,
            "permission_mode": "default",
            "cost_usd": 0.0123,
            "transcript": ["user: hi"],
            "permission_prompt": "run write_file on src/x.rs?",
            "prompt_seq": 4,
            "revision": 12,
            "a_field_from_a_newer_daemon": 99
        })
        .to_string();
        let snap = decode_snapshot(&frame).expect("a daemon frame");
        assert_eq!(snap.session_id, "abc123");
        assert_eq!(snap.revision, Some(12));
        assert_eq!(snap.prompt_seq, 4);
        assert!(snap.waiting(), "a permission prompt blocks the turn");
    }

    #[test]
    fn an_older_daemon_without_revisions_is_still_understood() {
        let snap = decode_snapshot(r#"{"session_id":"s1","busy":true}"#).expect("a legacy frame");
        assert_eq!(snap.revision, None, "resume simply starts from rev 0");
        assert!(snap.busy);
    }

    #[tokio::test]
    async fn an_input_for_a_session_with_no_socket_says_so_instead_of_vanishing() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pool = SocketPool::new("http://h:1".into(), "tok".into(), tx);
        pool.send(
            "0123456789abcdef",
            &serde_json::json!({ "kind": "interrupt" }),
        );
        match rx.try_recv() {
            Ok(Ev::Board(BoardEvent::Toast(level, text))) => {
                assert_eq!(level, ToastLevel::Error);
                assert_eq!(text, "no live connection to 01234567");
            }
            _ => panic!("a dropped input must be reported"),
        }
    }
}
