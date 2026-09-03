//! A minimal Chrome DevTools Protocol client.
//!
//! CDP is a JSON-RPC-ish protocol over one WebSocket: commands carry a caller-chosen `id` and come
//! back as `{id, result}` or `{id, error}`, interleaved with unsolicited `{method, params}` events.
//! Because responses and events share the socket, a naive "send then read the next frame" client
//! mis-reads an event as a response the moment the page does anything on its own — which, on a
//! real site, is immediately. So a single reader task owns the socket, matches responses to
//! waiting callers by id, and forwards everything else to the event stream.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

/// Per-command ceiling. A CDP call that never answers (a page blocked on a modal dialog, a target
/// that died mid-call) must fail the tool call, not wedge the agent loop forever.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// One event pushed by the browser.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// A connected CDP session.
pub struct CdpClient {
    outgoing: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: AtomicU64,
    /// Kept so the reader/writer tasks are aborted when the client drops.
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for CdpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpClient").finish_non_exhaustive()
    }
}

impl Drop for CdpClient {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl CdpClient {
    /// Connect to a target's WebSocket endpoint. Events are forwarded to `events`.
    pub async fn connect(ws_url: &str, events: mpsc::UnboundedSender<CdpEvent>) -> Result<Self> {
        let (stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .with_context(|| format!("connect to CDP endpoint {ws_url}"))?;
        let (mut sink, mut source) = stream.split();

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (outgoing, mut outbox) = mpsc::unbounded_channel::<String>();

        let writer = tokio::spawn(async move {
            while let Some(text) = outbox.recv().await {
                if sink
                    .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let reader_pending = Arc::clone(&pending);
        let reader = tokio::spawn(async move {
            while let Some(Ok(message)) = source.next().await {
                let text = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                match route(&value) {
                    Routed::Response { id, outcome } => {
                        let waiter = reader_pending
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&id);
                        if let Some(waiter) = waiter {
                            let _ = waiter.send(outcome);
                        }
                    }
                    Routed::Event(event) => {
                        // A dropped receiver means the session is shutting down; the socket does
                        // not need to die with it.
                        let _ = events.send(event);
                    }
                    Routed::Ignored => {}
                }
            }
            // Socket closed: fail every caller still waiting instead of leaving them to time out
            // one by one.
            let waiters: Vec<_> = reader_pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drain()
                .collect();
            for (_, waiter) in waiters {
                let _ = waiter.send(Err("the browser closed the DevTools connection".into()));
            }
        });

        Ok(Self {
            outgoing,
            pending,
            next_id: AtomicU64::new(1),
            tasks: vec![reader, writer],
        })
    }

    /// Send a command and wait for its result.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, tx);

        let frame = json!({ "id": id, "method": method, "params": params }).to_string();
        if self.outgoing.send(frame).is_err() {
            self.pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            bail!("the DevTools connection is closed");
        }

        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(message))) => Err(anyhow!("{method} failed: {message}")),
            Ok(Err(_)) => Err(anyhow!("{method}: the DevTools connection dropped")),
            Err(_) => {
                self.pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&id);
                Err(anyhow!(
                    "{method} did not answer within {}s",
                    CALL_TIMEOUT.as_secs()
                ))
            }
        }
    }
}

/// What one inbound frame is.
#[derive(Debug, PartialEq)]
enum Routed {
    Response {
        id: u64,
        outcome: Result<Value, String>,
    },
    Event(CdpEvent),
    Ignored,
}

impl PartialEq for CdpEvent {
    fn eq(&self, other: &Self) -> bool {
        self.method == other.method && self.params == other.params
    }
}

/// Classify an inbound frame. Split out from the reader so the id/event distinction — the thing a
/// naive client gets wrong — is testable without a browser.
fn route(value: &Value) -> Routed {
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown CDP error");
            let data = error.get("data").and_then(Value::as_str);
            let full = match data {
                Some(data) => format!("{message} ({data})"),
                None => message.to_string(),
            };
            return Routed::Response {
                id,
                outcome: Err(full),
            };
        }
        return Routed::Response {
            id,
            outcome: Ok(value.get("result").cloned().unwrap_or(Value::Null)),
        };
    }
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        return Routed::Event(CdpEvent {
            method: method.to_string(),
            params: value.get("params").cloned().unwrap_or(Value::Null),
        });
    }
    Routed::Ignored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_response_and_an_event_are_told_apart() {
        // The bug this prevents: responses and events share one socket, so a client that reads
        // "the next frame" as its answer mis-reads the first event a live page emits — which on a
        // real site arrives before the command has even been answered.
        let response = route(&json!({"id": 7, "result": {"frameId": "F1"}}));
        assert_eq!(
            response,
            Routed::Response {
                id: 7,
                outcome: Ok(json!({"frameId": "F1"})),
            }
        );

        let event = route(&json!({
            "method": "Network.requestWillBeSent",
            "params": {"requestId": "R1"}
        }));
        assert_eq!(
            event,
            Routed::Event(CdpEvent {
                method: "Network.requestWillBeSent".into(),
                params: json!({"requestId": "R1"}),
            })
        );
    }

    #[test]
    fn a_cdp_error_carries_its_data_not_just_its_message() {
        // CDP puts the actionable half in `data` — "No node with given id" arrives as a bare
        // "Internal error" without it.
        let routed = route(&json!({
            "id": 3,
            "error": {"code": -32000, "message": "Internal error", "data": "No node with given id"}
        }));
        match routed {
            Routed::Response { id, outcome } => {
                assert_eq!(id, 3);
                let message = outcome.expect_err("an error frame is an error");
                assert!(message.contains("Internal error"), "{message}");
                assert!(message.contains("No node with given id"), "{message}");
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn a_result_less_response_is_still_a_success() {
        // Several CDP commands (Network.enable, Page.enable) answer with a bare `{id}`.
        assert_eq!(
            route(&json!({"id": 1})),
            Routed::Response {
                id: 1,
                outcome: Ok(Value::Null),
            }
        );
    }

    #[test]
    fn a_frame_that_is_neither_is_ignored_rather_than_guessed_at() {
        assert_eq!(route(&json!({"sessionId": "S1"})), Routed::Ignored);
    }
}
