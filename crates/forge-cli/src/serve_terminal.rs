//! `forge serve` terminal sessions.
//!
//! A terminal is owned by the daemon, not by one WebSocket. Clients attach to a stable
//! `(session, terminal)` key, receive a bounded history snapshot, and can disconnect/reconnect
//! without killing the PTY. Up to [`MAX_TERMINALS_PER_SESSION`] shells may exist per Forge session.
//!
//! Wire protocol:
//! - `WS {base}/ws/terminal?session=<id>&terminal=term-1[&cols=&rows=&restart=true]`
//! - client text: `input`, `resize`, `clear`, or `close` tagged JSON frames
//! - server binary: raw PTY bytes
//! - server text: bounded `status` and `cleared` control frames
//!
//! Security: the client never supplies a cwd. The live Forge session determines the worktree/cwd,
//! and terminal ids are validated as short opaque labels. The daemon token already grants agent
//! command execution on the same machine, so this endpoint adds no broader privilege.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::serve::{err_response, json_response, DaemonState};

const READ_CHUNK_BYTES: usize = 8 * 1024;
const OUTPUT_CHANNEL_CHUNKS: usize = 64;
const COMMAND_CHANNEL_CHUNKS: usize = 64;
const BROADCAST_CHANNEL_EVENTS: usize = 256;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_HISTORY_BYTES: usize = 2 * 1024 * 1024;
const MAX_TERMINALS_PER_SESSION: usize = 8;
const MAX_TERMINAL_ID_BYTES: usize = 64;
const MAX_COLS: u16 = 1_000;
const MAX_ROWS: u16 = 1_000;

fn default_terminal_id() -> String {
    "term-1".to_string()
}

#[derive(serde::Deserialize)]
pub(crate) struct TerminalQuery {
    #[serde(default)]
    session: String,
    #[serde(default = "default_terminal_id")]
    terminal: String,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    restart: bool,
}

#[derive(serde::Deserialize)]
pub(crate) struct TerminalListQuery {
    #[serde(default)]
    session: String,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalFrame {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Clear,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum TerminalStatus {
    Running,
    Exited,
}

impl TerminalStatus {
    fn as_u8(self) -> u8 {
        match self {
            Self::Running => 1,
            Self::Exited => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Running,
            _ => Self::Exited,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalServerFrame {
    Status { status: TerminalStatus },
    Cleared,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct TerminalSummary {
    terminal_id: String,
    status: TerminalStatus,
    clients: usize,
    updated_at_ms: u64,
}

#[derive(Clone, Eq)]
struct TerminalKey {
    session_id: String,
    terminal_id: String,
}

impl PartialEq for TerminalKey {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id && self.terminal_id == other.terminal_id
    }
}

impl Hash for TerminalKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.session_id.hash(state);
        self.terminal_id.hash(state);
    }
}

struct TerminalHistory {
    bytes: VecDeque<u8>,
    sequence: u64,
}

impl TerminalHistory {
    fn new() -> Self {
        Self {
            bytes: VecDeque::new(),
            sequence: 0,
        }
    }

    fn append(&mut self, bytes: &[u8]) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.bytes.extend(bytes.iter().copied());
        while self.bytes.len() > MAX_HISTORY_BYTES {
            self.bytes.pop_front();
        }
        self.sequence
    }

    fn clear(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.bytes.clear();
        self.sequence
    }

    fn snapshot(&self) -> (u64, Vec<u8>) {
        (self.sequence, self.bytes.iter().copied().collect())
    }
}

#[derive(Clone)]
enum TerminalEvent {
    Output { sequence: u64, bytes: Vec<u8> },
    Cleared { sequence: u64 },
    Status(TerminalStatus),
}

enum TerminalCommand {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Kill,
}

struct TerminalHandle {
    key: TerminalKey,
    commands: mpsc::Sender<TerminalCommand>,
    events: broadcast::Sender<TerminalEvent>,
    history: Arc<Mutex<TerminalHistory>>,
    status: Arc<AtomicU8>,
    clients: AtomicUsize,
    updated_at_ms: AtomicU64,
}

impl TerminalHandle {
    fn status(&self) -> TerminalStatus {
        TerminalStatus::from_u8(self.status.load(Ordering::Acquire))
    }

    fn touch(&self) {
        self.updated_at_ms.store(now_ms(), Ordering::Release);
    }

    async fn clear(&self) {
        let sequence = self.history.lock().await.clear();
        self.touch();
        let _ = self.events.send(TerminalEvent::Cleared { sequence });
    }

    fn summary(&self) -> TerminalSummary {
        TerminalSummary {
            terminal_id: self.key.terminal_id.clone(),
            status: self.status(),
            clients: self.clients.load(Ordering::Acquire),
            updated_at_ms: self.updated_at_ms.load(Ordering::Acquire),
        }
    }
}

#[derive(Default)]
pub(crate) struct TerminalRegistry {
    terminals: Mutex<HashMap<TerminalKey, Arc<TerminalHandle>>>,
}

impl TerminalRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    async fn ensure(
        &self,
        key: TerminalKey,
        cwd: String,
        cols: u16,
        rows: u16,
    ) -> Result<Arc<TerminalHandle>, String> {
        let mut terminals = self.terminals.lock().await;
        if let Some(existing) = terminals.get(&key) {
            return Ok(existing.clone());
        }
        let count = terminals
            .keys()
            .filter(|candidate| candidate.session_id == key.session_id)
            .count();
        if count >= MAX_TERMINALS_PER_SESSION {
            return Err(format!(
                "a session may have at most {MAX_TERMINALS_PER_SESSION} terminals"
            ));
        }
        let handle = spawn_terminal(key.clone(), cwd, cols, rows)?;
        terminals.insert(key, handle.clone());
        Ok(handle)
    }

    async fn close(&self, key: &TerminalKey) -> bool {
        let handle = self.terminals.lock().await.remove(key);
        if let Some(handle) = handle {
            let _ = handle.commands.send(TerminalCommand::Kill).await;
            true
        } else {
            false
        }
    }

    pub(crate) async fn close_session(&self, session_id: &str) {
        let handles = {
            let mut terminals = self.terminals.lock().await;
            let keys: Vec<_> = terminals
                .keys()
                .filter(|key| key.session_id == session_id)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| terminals.remove(&key))
                .collect::<Vec<_>>()
        };
        for handle in handles {
            let _ = handle.commands.send(TerminalCommand::Kill).await;
        }
    }

    async fn list(&self, session_id: &str) -> Vec<TerminalSummary> {
        let handles = self
            .terminals
            .lock()
            .await
            .iter()
            .filter(|(key, _)| key.session_id == session_id)
            .map(|(_, handle)| handle.clone())
            .collect::<Vec<_>>();
        let mut summaries = handles
            .into_iter()
            .map(|handle| handle.summary())
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| terminal_id_order(&left.terminal_id, &right.terminal_id));
        summaries
    }
}

fn terminal_id_order(left: &str, right: &str) -> std::cmp::Ordering {
    fn numeric_suffix(value: &str) -> Option<u64> {
        value.strip_prefix("term-")?.parse().ok()
    }
    match (numeric_suffix(left), numeric_suffix(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn valid_terminal_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_TERMINAL_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn login_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn clamp_size(cols: Option<u16>, rows: Option<u16>) -> (u16, u16) {
    (
        cols.unwrap_or(80).clamp(1, MAX_COLS),
        rows.unwrap_or(24).clamp(1, MAX_ROWS),
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn spawn_terminal(
    key: TerminalKey,
    cwd: String,
    cols: u16,
    rows: u16,
) -> Result<Arc<TerminalHandle>, String> {
    let size = PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = native_pty_system()
        .openpty(size)
        .map_err(|error| format!("failed to open a pty: {error}"))?;

    let mut command = CommandBuilder::new(login_shell());
    command.cwd(&cwd);
    command.env("TERM", "xterm-256color");
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("failed to start a shell in {cwd}: {error}"))?;
    drop(pair.slave);

    let mut reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            return Err(format!("failed to attach terminal reader: {error}"));
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = child.kill();
            return Err(format!("failed to attach terminal writer: {error}"));
        }
    };
    let mut killer = child.clone_killer();
    let master = pair.master;

    let (output_tx, mut output_rx) = mpsc::channel::<Vec<u8>>(OUTPUT_CHANNEL_CHUNKS);
    let reader_task = tokio::task::spawn_blocking(move || {
        let mut buffer = [0u8; READ_CHUNK_BYTES];
        loop {
            match std::io::Read::read(&mut reader, &mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if output_tx.blocking_send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(COMMAND_CHANNEL_CHUNKS);
    let writer_task = tokio::task::spawn_blocking(move || {
        let mut writer = writer;
        while let Some(chunk) = input_rx.blocking_recv() {
            use std::io::Write;
            if writer.write_all(&chunk).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let (exit_tx, mut exit_rx) = mpsc::channel::<()>(1);
    let wait_task = tokio::task::spawn_blocking(move || {
        let _ = child.wait();
        let _ = exit_tx.blocking_send(());
    });

    let (commands, mut command_rx) = mpsc::channel(COMMAND_CHANNEL_CHUNKS);
    let (events, _) = broadcast::channel(BROADCAST_CHANNEL_EVENTS);
    let history = Arc::new(Mutex::new(TerminalHistory::new()));
    let status = Arc::new(AtomicU8::new(TerminalStatus::Running.as_u8()));
    let handle = Arc::new(TerminalHandle {
        key,
        commands,
        events: events.clone(),
        history: history.clone(),
        status: status.clone(),
        clients: AtomicUsize::new(0),
        updated_at_ms: AtomicU64::new(now_ms()),
    });
    let runtime_handle = handle.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                output = output_rx.recv() => {
                    let Some(bytes) = output else { break; };
                    let sequence = history.lock().await.append(&bytes);
                    runtime_handle.touch();
                    let _ = events.send(TerminalEvent::Output { sequence, bytes });
                }
                command = command_rx.recv() => {
                    match command {
                        Some(TerminalCommand::Input(bytes)) => {
                            runtime_handle.touch();
                            if input_tx.send(bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(TerminalCommand::Resize { cols, rows }) => {
                            let (cols, rows) = clamp_size(Some(cols), Some(rows));
                            let _ = master.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                        Some(TerminalCommand::Kill) | None => {
                            let _ = killer.kill();
                            break;
                        }
                    }
                }
                _ = exit_rx.recv() => break,
            }
        }

        status.store(TerminalStatus::Exited.as_u8(), Ordering::Release);
        runtime_handle.touch();
        let _ = events.send(TerminalEvent::Status(TerminalStatus::Exited));
        drop(input_tx);
        drop(master);
        reader_task.abort();
        writer_task.abort();
        wait_task.abort();
        let _ = killer.kill();
    });

    Ok(handle)
}

pub(crate) async fn terminal_list(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<TerminalListQuery>,
) -> Response {
    if state.registry.get(&params.session).await.is_none() {
        return err_response(axum::http::StatusCode::NOT_FOUND, "session not found");
    }
    json_response(&state.terminals.list(&params.session).await)
}

pub(crate) async fn terminal_ws(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(session) = state.registry.get(&params.session).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "session not found");
    };
    if !valid_terminal_id(&params.terminal) {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "invalid terminal id");
    }
    let cwd = session
        .worktree
        .clone()
        .unwrap_or_else(|| session.cwd.clone());
    if cwd.is_empty() {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "session has no working directory",
        );
    }

    let key = TerminalKey {
        session_id: params.session,
        terminal_id: params.terminal,
    };
    let (cols, rows) = clamp_size(params.cols, params.rows);
    if params.restart {
        state.terminals.close(&key).await;
    }
    let terminal = match state.terminals.ensure(key.clone(), cwd, cols, rows).await {
        Ok(terminal) => terminal,
        Err(error) => return err_response(axum::http::StatusCode::BAD_REQUEST, &error),
    };
    let registry = state.terminals.clone();
    ws.on_upgrade(move |socket| attach_terminal(socket, terminal, registry, key))
}

struct TerminalClientGuard(Arc<TerminalHandle>);

impl TerminalClientGuard {
    fn new(handle: Arc<TerminalHandle>) -> Self {
        handle.clients.fetch_add(1, Ordering::AcqRel);
        handle.touch();
        Self(handle)
    }
}

impl Drop for TerminalClientGuard {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::AcqRel);
        self.0.touch();
    }
}

async fn send_server_frame(socket: &mut WebSocket, frame: TerminalServerFrame) -> bool {
    let Ok(json) = serde_json::to_string(&frame) else {
        return false;
    };
    socket.send(WsMessage::Text(json.into())).await.is_ok()
}

async fn send_history(socket: &mut WebSocket, handle: &TerminalHandle) -> Option<u64> {
    let (sequence, history) = handle.history.lock().await.snapshot();
    if !history.is_empty()
        && socket
            .send(WsMessage::Binary(history.into()))
            .await
            .is_err()
    {
        return None;
    }
    Some(sequence)
}

async fn attach_terminal(
    mut socket: WebSocket,
    terminal: Arc<TerminalHandle>,
    registry: Arc<TerminalRegistry>,
    key: TerminalKey,
) {
    let _client = TerminalClientGuard::new(terminal.clone());
    let mut events = terminal.events.subscribe();
    let initial_status = terminal.status();
    if !send_server_frame(
        &mut socket,
        TerminalServerFrame::Status {
            status: initial_status,
        },
    )
    .await
    {
        return;
    }
    let Some(mut delivered_sequence) = send_history(&mut socket, &terminal).await else {
        return;
    };
    if initial_status == TerminalStatus::Exited {
        let _ = socket.send(WsMessage::Close(None)).await;
        return;
    }

    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Ok(TerminalEvent::Output { sequence, bytes }) => {
                        if sequence <= delivered_sequence {
                            continue;
                        }
                        delivered_sequence = sequence;
                        if socket.send(WsMessage::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(TerminalEvent::Cleared { sequence }) => {
                        delivered_sequence = sequence;
                        if !send_server_frame(&mut socket, TerminalServerFrame::Cleared).await {
                            break;
                        }
                    }
                    Ok(TerminalEvent::Status(status)) => {
                        if !send_server_frame(&mut socket, TerminalServerFrame::Status { status }).await {
                            break;
                        }
                        if status == TerminalStatus::Exited {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if !send_server_frame(&mut socket, TerminalServerFrame::Cleared).await {
                            break;
                        }
                        let Some(sequence) = send_history(&mut socket, &terminal).await else {
                            break;
                        };
                        delivered_sequence = sequence;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => {
                        match serde_json::from_str::<TerminalFrame>(&text) {
                            Ok(TerminalFrame::Input { data }) => {
                                if data.len() <= MAX_INPUT_BYTES
                                    && terminal
                                        .commands
                                        .send(TerminalCommand::Input(data.into_bytes()))
                                        .await
                                        .is_err()
                                {
                                    break;
                                }
                            }
                            Ok(TerminalFrame::Resize { cols, rows }) => {
                                if terminal
                                    .commands
                                    .send(TerminalCommand::Resize { cols, rows })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Ok(TerminalFrame::Clear) => terminal.clear().await,
                            Ok(TerminalFrame::Close) => {
                                registry.close(&key).await;
                                break;
                            }
                            Err(_) => {}
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }

    let _ = socket.send(WsMessage::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn key(session_id: &str, terminal_id: &str) -> TerminalKey {
        TerminalKey {
            session_id: session_id.to_string(),
            terminal_id: terminal_id.to_string(),
        }
    }

    async fn wait_for_history(handle: &TerminalHandle, needle: &[u8]) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (_, history) = handle.history.lock().await.snapshot();
                if history.windows(needle.len()).any(|window| window == needle) {
                    return history;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("terminal emitted expected output")
    }

    #[test]
    fn sizes_are_clamped_to_sane_geometry() {
        assert_eq!(clamp_size(None, None), (80, 24));
        assert_eq!(clamp_size(Some(0), Some(0)), (1, 1));
        assert_eq!(clamp_size(Some(9_000), Some(9_000)), (MAX_COLS, MAX_ROWS));
    }

    #[test]
    fn frames_parse_by_kind_tag() {
        let input: TerminalFrame =
            serde_json::from_str(r#"{"kind":"input","data":"ls\n"}"#).unwrap();
        assert!(matches!(input, TerminalFrame::Input { data } if data == "ls\n"));
        let resize: TerminalFrame =
            serde_json::from_str(r#"{"kind":"resize","cols":120,"rows":40}"#).unwrap();
        assert!(matches!(
            resize,
            TerminalFrame::Resize {
                cols: 120,
                rows: 40
            }
        ));
        assert!(matches!(
            serde_json::from_str::<TerminalFrame>(r#"{"kind":"clear"}"#).unwrap(),
            TerminalFrame::Clear
        ));
        assert!(serde_json::from_str::<TerminalFrame>(r#"{"kind":"exec"}"#).is_err());
    }

    #[test]
    fn terminal_ids_are_strict_and_bounded() {
        assert!(valid_terminal_id("term-1"));
        assert!(valid_terminal_id("dev_server"));
        assert!(!valid_terminal_id(""));
        assert!(!valid_terminal_id("../term-1"));
        assert!(!valid_terminal_id(&"x".repeat(MAX_TERMINAL_ID_BYTES + 1)));
    }

    #[test]
    fn history_is_bounded_and_sequence_is_monotonic() {
        let mut history = TerminalHistory::new();
        assert_eq!(history.append(b"one"), 1);
        assert_eq!(history.clear(), 2);
        assert_eq!(history.append(&vec![b'x'; MAX_HISTORY_BYTES + 128]), 3);
        let (sequence, bytes) = history.snapshot();
        assert_eq!(sequence, 3);
        assert_eq!(bytes.len(), MAX_HISTORY_BYTES);
    }

    #[test]
    fn numeric_terminal_ids_sort_for_humans() {
        let mut ids = ["term-10", "term-2", "custom", "term-1"];
        ids.sort_by(|left, right| terminal_id_order(left, right));
        assert_eq!(ids, ["custom", "term-1", "term-2", "term-10"]);
    }

    #[tokio::test]
    async fn daemon_registry_reuses_a_detached_terminal_and_preserves_history() {
        let cwd = tempfile::tempdir().unwrap();
        let registry = TerminalRegistry::new();
        let terminal_key = key("session-a", "term-1");
        let first = registry
            .ensure(
                terminal_key.clone(),
                cwd.path().display().to_string(),
                80,
                24,
            )
            .await
            .unwrap();
        first
            .commands
            .send(TerminalCommand::Input(
                b"echo __FORGE_TERMINAL_HISTORY__\r\n".to_vec(),
            ))
            .await
            .unwrap();
        let history = wait_for_history(&first, b"__FORGE_TERMINAL_HISTORY__").await;
        assert!(history
            .windows(b"__FORGE_TERMINAL_HISTORY__".len())
            .any(|window| window == b"__FORGE_TERMINAL_HISTORY__"));

        // A socket detach only drops its client guard. Re-attaching resolves the same daemon
        // handle, whose bounded history is sent before live events.
        let second = registry
            .ensure(terminal_key, cwd.path().display().to_string(), 120, 40)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let (_, replay) = second.history.lock().await.snapshot();
        assert_eq!(replay, history);
        registry.close_session("session-a").await;
    }

    #[tokio::test]
    async fn terminal_ids_are_isolated_and_close_removes_only_the_target() {
        let cwd = tempfile::tempdir().unwrap();
        let registry = TerminalRegistry::new();
        let first_key = key("session-a", "term-1");
        let second_key = key("session-a", "term-2");
        let first = registry
            .ensure(first_key.clone(), cwd.path().display().to_string(), 80, 24)
            .await
            .unwrap();
        let second = registry
            .ensure(second_key.clone(), cwd.path().display().to_string(), 80, 24)
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        first.clear().await;
        assert_ne!(
            first.history.lock().await.snapshot().0,
            second.history.lock().await.snapshot().0
        );

        assert!(registry.close(&first_key).await);
        let summaries = registry.list("session-a").await;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].terminal_id, "term-2");
        assert!(!registry.close(&first_key).await);
        assert!(registry.close(&second_key).await);
    }

    #[tokio::test]
    async fn registry_enforces_the_per_session_terminal_cap() {
        let cwd = tempfile::tempdir().unwrap();
        let registry = TerminalRegistry::new();
        for index in 1..=MAX_TERMINALS_PER_SESSION {
            registry
                .ensure(
                    key("capped", &format!("term-{index}")),
                    cwd.path().display().to_string(),
                    80,
                    24,
                )
                .await
                .unwrap();
        }
        let error = registry
            .ensure(
                key("capped", "term-overflow"),
                cwd.path().display().to_string(),
                80,
                24,
            )
            .await
            .err()
            .expect("ninth terminal is rejected");
        assert!(error.contains("at most 8 terminals"));
        registry.close_session("capped").await;
    }
}
