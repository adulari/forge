//! Driving a terminal `forge` chat session from the phone and desktop apps.
//!
//! A terminal chat is not in the daemon's session registry: no driver task owns it, no input
//! channel reaches it, and it exits when its terminal does. Until now that made it a second-class
//! fleet row — visible, but read-only, so a permission prompt raised on the laptop could not be
//! answered from the phone.
//!
//! It is, however, already running the whole remote-session protocol for its own `/remote` page:
//! it builds a [`crate::remote::Snapshot`] every frame and injects a
//! [`crate::remote::RemoteInput`] exactly like a local keystroke. So rather than reimplement any
//! of that against the store, the terminal opens a second, token-gated copy of that server on
//! loopback and records its URL in the shared store ([`forge_store::Store::local_session_control_url`]),
//! and this module splices the app's WebSocket onto it frame for frame.
//!
//! The proxy is deliberately protocol-blind: it copies opaque messages both ways. There is nothing
//! here to keep in step as the snapshot grows fields or `RemoteInput` grows variants, and a client
//! attached to a terminal session gets byte-identical behaviour to one attached to a daemon-hosted
//! one — including `?rev=` reconnect replay, which the terminal's own event log answers.

use axum::extract::ws::{Message as AxumMessage, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Build the upstream WebSocket URL for a published control endpoint.
///
/// The stored value is the browser-facing base (`http://127.0.0.1:<port>/<token>`); its WS route
/// is `<base>/ws`, and `?rev=` is forwarded so the client's reconnect replay is answered by the
/// terminal's own event log rather than being downgraded to a full resync.
pub(super) fn upstream_ws_url(base: &str, rev: u64) -> String {
    let ws_base = match base.split_once("://") {
        Some(("https", rest)) => format!("wss://{rest}"),
        Some((_, rest)) => format!("ws://{rest}"),
        None => format!("ws://{base}"),
    };
    format!("{}/ws?rev={rev}", ws_base.trim_end_matches('/'))
}

/// Copy messages between an attached app client and a terminal session's control channel until
/// either side closes.
///
/// Errors are terminal for the connection, never for the daemon: a terminal that exited between
/// the fleet listing and the client's tap simply fails to connect, and the client sees a closed
/// socket and retries — the same thing it already does when a daemon-hosted session ends.
pub(super) async fn pump_proxy(client: WebSocket, upstream_url: String) {
    let upstream = match tokio_tungstenite::connect_async(&upstream_url).await {
        Ok((stream, _)) => stream,
        Err(error) => {
            tracing::debug!(
                url = %upstream_url,
                %error,
                "terminal session control channel refused the proxy connection"
            );
            return;
        }
    };
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut client_tx, mut client_rx) = client.split();

    // Two independent copy loops: the terminal broadcasts a frame roughly every time its TUI
    // repaints, while the client sends only on a tap, so neither direction may be able to block
    // the other.
    let to_upstream = async {
        while let Some(Ok(message)) = client_rx.next().await {
            let forwarded = match message {
                AxumMessage::Text(text) => WsMessage::Text(text.as_str().into()),
                AxumMessage::Binary(bytes) => WsMessage::Binary(bytes),
                AxumMessage::Ping(bytes) => WsMessage::Ping(bytes),
                AxumMessage::Pong(bytes) => WsMessage::Pong(bytes),
                AxumMessage::Close(_) => break,
            };
            if up_tx.send(forwarded).await.is_err() {
                break;
            }
        }
        let _ = up_tx.close().await;
    };
    let to_client = async {
        while let Some(Ok(message)) = up_rx.next().await {
            let forwarded = match message {
                WsMessage::Text(text) => AxumMessage::Text(text.as_str().into()),
                WsMessage::Binary(bytes) => AxumMessage::Binary(bytes),
                WsMessage::Ping(bytes) => AxumMessage::Ping(bytes),
                WsMessage::Pong(bytes) => AxumMessage::Pong(bytes),
                WsMessage::Close(_) => break,
                // Tungstenite answers protocol-level pings itself; a raw frame is not something
                // the client half of this proxy has any use for.
                WsMessage::Frame(_) => continue,
            };
            if client_tx.send(forwarded).await.is_err() {
                break;
            }
        }
        let _ = client_tx.close().await;
    };
    tokio::pin!(to_upstream);
    tokio::pin!(to_client);
    // Whichever half finishes first ends the session: a half-open proxy would leave the client
    // rendering a frozen snapshot with no indication that its input path is gone.
    tokio::select! {
        () = &mut to_upstream => {}
        () = &mut to_client => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upstream_url_keeps_the_path_token_and_forwards_the_replay_cursor() {
        assert_eq!(
            upstream_ws_url("http://127.0.0.1:41234/abc123", 0),
            "ws://127.0.0.1:41234/abc123/ws?rev=0"
        );
        // A reconnect must carry its cursor through, or every app reconnect costs a full resync.
        assert_eq!(
            upstream_ws_url("http://127.0.0.1:41234/abc123", 97),
            "ws://127.0.0.1:41234/abc123/ws?rev=97"
        );
        // A trailing slash is what the PWA `start_url` shape produces; it must not double up.
        assert_eq!(
            upstream_ws_url("http://127.0.0.1:41234/abc123/", 3),
            "ws://127.0.0.1:41234/abc123/ws?rev=3"
        );
        assert!(upstream_ws_url("https://127.0.0.1:41234/tok", 1).starts_with("wss://"));
    }
}
