//! Local WebSocket bridge streams owned by one relay connection.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use forge_anywhere_protocol::bridge::{
    BridgeRequest, BridgeResponse, FrameDirection, WebSocketFrame, WebSocketFrameKind,
};
use futures::{SinkExt, StreamExt};
use reqwest::Url;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::{bridge_error, safe_path_segment};

/// Frames held per stream before the local socket stops being read (terminal output is never
/// dropped, so a slow relay still pushes back on it).
const MAX_QUEUED_FRAMES: usize = 256;

pub(super) struct StreamHandle {
    pub(super) owner_device_id: [u8; 16],
    pub(super) commands: mpsc::Sender<LocalSocketCommand>,
}

pub(super) enum LocalSocketCommand {
    Data { bytes: Vec<u8>, text: bool },
    Close,
}

pub(super) enum LocalSocketEvent {
    Data {
        stream_id: [u8; 16],
        owner_device_id: [u8; 16],
        bytes: Vec<u8>,
        text: bool,
    },
    Closed {
        stream_id: [u8; 16],
        owner_device_id: [u8; 16],
    },
}

pub(super) async fn open_stream(
    local_base_url: &str,
    request: &BridgeRequest,
    owner_device_id: [u8; 16],
    streams: &mut HashMap<[u8; 16], StreamHandle>,
    local_events: &mpsc::Sender<LocalSocketEvent>,
) -> BridgeResponse {
    match open_stream_inner(
        local_base_url,
        request,
        owner_device_id,
        streams,
        local_events,
    )
    .await
    {
        Ok(()) => BridgeResponse {
            request_id: request.request_id,
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            body_blob: None,
        },
        Err(error) => bridge_error(request.request_id, 400, &format!("{error:#}")),
    }
}

async fn open_stream_inner(
    local_base_url: &str,
    request: &BridgeRequest,
    owner_device_id: [u8; 16],
    streams: &mut HashMap<[u8; 16], StreamHandle>,
    local_events: &mpsc::Sender<LocalSocketEvent>,
) -> Result<()> {
    if !request.method.eq_ignore_ascii_case("GET")
        || !request.headers.is_empty()
        || !request.body.is_empty()
        || request.body_blob.is_some()
    {
        bail!("WebSocket open request has invalid fields");
    }
    if streams.contains_key(&request.request_id) {
        bail!("WebSocket stream id is already open");
    }
    let mut url = Url::parse(local_base_url).context("parse local WebSocket URL")?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => bail!("local daemon URL must use HTTP or HTTPS"),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("set local WebSocket scheme"))?;
    let base_path = url.path().trim_end_matches('/').to_owned();
    url.set_query(None);
    match request.route {
        forge_anywhere_protocol::bridge::RouteId::WebSocket => {
            if request.parameters.len() != 2 {
                bail!("session WebSocket open request has invalid parameters");
            }
            let session_id = safe_path_segment(&request.parameters[0])?;
            let revision = request.parameters[1]
                .parse::<u64>()
                .context("invalid WebSocket revision")?;
            url.set_path(&format!("{base_path}/ws"));
            url.query_pairs_mut()
                .append_pair("session", session_id)
                .append_pair("rev", &revision.to_string());
        }
        forge_anywhere_protocol::bridge::RouteId::TerminalWebSocket => {
            if request.parameters.len() != 5 {
                bail!("terminal WebSocket open request has invalid parameters");
            }
            let session_id = safe_path_segment(&request.parameters[0])?;
            let terminal_id = safe_path_segment(&request.parameters[1])?;
            if terminal_id.len() > 64 {
                bail!("terminal id is too long");
            }
            let cols = request.parameters[2]
                .parse::<u16>()
                .context("invalid terminal columns")?;
            let rows = request.parameters[3]
                .parse::<u16>()
                .context("invalid terminal rows")?;
            if !(1..=1_000).contains(&cols) || !(1..=1_000).contains(&rows) {
                bail!("terminal geometry is out of range");
            }
            let restart = match request.parameters[4].as_str() {
                "0" => "false",
                "1" => "true",
                _ => bail!("invalid terminal restart flag"),
            };
            url.set_path(&format!("{base_path}/ws/terminal"));
            url.query_pairs_mut()
                .append_pair("session", session_id)
                .append_pair("terminal", terminal_id)
                .append_pair("cols", &cols.to_string())
                .append_pair("rows", &rows.to_string())
                .append_pair("restart", restart);
        }
        _ => bail!("route is not a WebSocket stream"),
    }

    let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .context("open local Forge session WebSocket")?;
    let (commands_tx, mut commands_rx) = mpsc::channel(64);
    let events = local_events.clone();
    let stream_id = request.request_id;
    tokio::spawn(async move {
        let (mut write, mut read) = socket.split();
        // Frames wait here, not in the shared channel, while the relay is slower than the daemon,
        // so a newer full snapshot can replace the ones the phone has not been sent yet.
        let mut queue = FrameQueue::default();
        loop {
            tokio::select! {
                permit = events.reserve(), if !queue.is_empty() => {
                    let Ok(permit) = permit else { break };
                    if let Some((bytes, text)) = queue.pop() {
                        permit.send(LocalSocketEvent::Data { stream_id, owner_device_id, bytes, text });
                    }
                }
                command = commands_rx.recv() => match command {
                    Some(LocalSocketCommand::Data { bytes, text }) => {
                        let message = if text {
                            match String::from_utf8(bytes) {
                                Ok(text) => Message::Text(text.into()),
                                Err(_) => break,
                            }
                        } else {
                            Message::Binary(bytes.into())
                        };
                        if write.send(message).await.is_err() { break; }
                    }
                    Some(LocalSocketCommand::Close) | None => {
                        let _ = write.send(Message::Close(None)).await;
                        break;
                    }
                },
                message = read.next(), if queue.len() < MAX_QUEUED_FRAMES => match message {
                    Some(Ok(Message::Text(text))) => queue.push(text.as_bytes().to_vec(), true),
                    Some(Ok(Message::Binary(bytes))) => queue.push(bytes.to_vec(), false),
                    Some(Ok(Message::Ping(bytes))) => {
                        if write.send(Message::Pong(bytes)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                }
            }
        }
        // Deliver what the daemon already sent before announcing the close.
        while let Some((bytes, text)) = queue.pop() {
            if events
                .send(LocalSocketEvent::Data {
                    stream_id,
                    owner_device_id,
                    bytes,
                    text,
                })
                .await
                .is_err()
            {
                break;
            }
        }
        let _ = events
            .send(LocalSocketEvent::Closed {
                stream_id,
                owner_device_id,
            })
            .await;
    });
    streams.insert(
        stream_id,
        StreamHandle {
            owner_device_id,
            commands: commands_tx,
        },
    );
    Ok(())
}

#[derive(Default)]
pub(super) struct FrameQueue {
    frames: std::collections::VecDeque<(Vec<u8>, bool)>,
}

enum FrameKind {
    Snapshot { resync: bool },
    Keepalive,
    Other,
}

fn classify(bytes: &[u8], text: bool) -> FrameKind {
    if !text {
        return FrameKind::Other;
    }
    if bytes.starts_with(br#"{"keepalive""#) {
        return FrameKind::Keepalive;
    }
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Object(map)) if map.get("revision").is_some_and(|v| v.is_u64()) => {
            FrameKind::Snapshot {
                resync: map.get("resync").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        }
        _ => FrameKind::Other,
    }
}

impl FrameQueue {
    pub(super) fn len(&self) -> usize {
        self.frames.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub(super) fn pop(&mut self) -> Option<(Vec<u8>, bool)> {
        self.frames.pop_front()
    }

    /// Latest snapshot wins: unsent older snapshots and keepalives are dropped when a newer
    /// snapshot arrives, and the survivor is flagged `resync` because its revision no longer
    /// follows the last frame the phone saw. Every other frame, and every close, is kept in order.
    pub(super) fn push(&mut self, bytes: Vec<u8>, text: bool) {
        if self.frames.is_empty() {
            self.frames.push_back((bytes, text));
            return;
        }
        match classify(&bytes, text) {
            FrameKind::Keepalive => {}
            FrameKind::Other => self.frames.push_back((bytes, text)),
            FrameKind::Snapshot { resync } => {
                let mut dropped_snapshot = false;
                self.frames.retain(
                    |(queued, queued_text)| match classify(queued, *queued_text) {
                        FrameKind::Snapshot { .. } => {
                            dropped_snapshot = true;
                            false
                        }
                        FrameKind::Keepalive => false,
                        FrameKind::Other => true,
                    },
                );
                let bytes = if dropped_snapshot && !resync {
                    mark_resync(&bytes).unwrap_or(bytes)
                } else {
                    bytes
                };
                self.frames.push_back((bytes, text));
            }
        }
    }
}

fn mark_resync(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    value
        .as_object_mut()?
        .insert("resync".into(), serde_json::Value::Bool(true));
    serde_json::to_vec(&value).ok()
}

/// Close a stale stream only when the controller has not already closed it.
pub(super) fn stale_stream_close(frame: &WebSocketFrame) -> Option<WebSocketFrame> {
    (frame.kind == WebSocketFrameKind::Data).then(|| WebSocketFrame {
        stream_id: frame.stream_id,
        direction: FrameDirection::HostToController,
        kind: WebSocketFrameKind::Close,
        text: false,
        bytes: Vec::new(),
        bytes_blob: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: WebSocketFrameKind) -> WebSocketFrame {
        WebSocketFrame {
            stream_id: [7; 16],
            direction: FrameDirection::ControllerToHost,
            kind,
            text: false,
            bytes: b"{}".to_vec(),
            bytes_blob: None,
        }
    }

    fn snapshot(revision: u64) -> Vec<u8> {
        format!(r#"{{"revision":{revision},"resync":false,"transcript":["row"]}}"#).into_bytes()
    }

    fn texts(queue: &mut FrameQueue) -> Vec<String> {
        std::iter::from_fn(|| queue.pop())
            .map(|(bytes, _)| String::from_utf8(bytes).unwrap())
            .collect()
    }

    #[test]
    fn newest_snapshot_replaces_waiting_ones_and_is_flagged_resync() {
        let mut queue = FrameQueue::default();
        queue.push(snapshot(1), true);
        queue.push(snapshot(2), true);
        queue.push(br#"{"keepalive":true}"#.to_vec(), true);
        queue.push(snapshot(3), true);
        let out = texts(&mut queue);
        assert_eq!(out.len(), 1);
        let value: serde_json::Value = serde_json::from_str(&out[0]).unwrap();
        assert_eq!(value["revision"], 3);
        assert_eq!(value["resync"], true);
    }

    #[test]
    fn a_lone_snapshot_keeps_its_contiguous_flag_and_keepalives_yield_to_traffic() {
        let mut queue = FrameQueue::default();
        queue.push(snapshot(1), true);
        queue.push(br#"{"keepalive":true}"#.to_vec(), true);
        let out = texts(&mut queue);
        assert_eq!(out, vec![String::from_utf8(snapshot(1)).unwrap()]);
    }

    #[test]
    fn non_snapshot_frames_are_never_dropped_or_reordered() {
        let mut queue = FrameQueue::default();
        queue.push(snapshot(1), true);
        queue.push(b"\x01terminal".to_vec(), false);
        queue.push(br#"{"event":"x"}"#.to_vec(), true);
        queue.push(snapshot(2), true);
        let out = texts(&mut queue);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], "\u{1}terminal");
        assert_eq!(out[1], r#"{"event":"x"}"#);
        assert!(out[2].contains(r#""revision":2"#));
    }

    #[test]
    fn forgotten_stream_data_is_closed_but_a_close_is_not_echoed() {
        let close = stale_stream_close(&frame(WebSocketFrameKind::Data)).expect("data is answered");
        assert_eq!(close.stream_id, [7; 16]);
        assert_eq!(close.direction, FrameDirection::HostToController);
        assert_eq!(close.kind, WebSocketFrameKind::Close);
        assert!(!close.text);
        assert!(close.bytes.is_empty());
        assert_eq!(close.bytes_blob, None);
        assert!(stale_stream_close(&frame(WebSocketFrameKind::Close)).is_none());
    }
}
