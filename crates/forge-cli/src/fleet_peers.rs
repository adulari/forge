//! Session-to-session messaging for the fleet: who is out there, and how a message gets to them.
//!
//! Everything a transport needs lives here so the direct tool path (`DaemonFleetMessaging`), the
//! bridge's HTTP mirror (`mcp_serve::fleet`), `forge send` and the Anywhere relay all apply the
//! same rules: one address resolution, one set of guards (no self-send, a per-pair rate limit,
//! and a conversation-depth guard against two agents ping-ponging), one wire framing
//! (`forge_core::fleet::peer_message_text`), and one delivery path per recipient kind —
//! a daemon-hosted driver's input queue, or a `forge chat` terminal's published control channel.
//!
//! Other machines are reached through [`RemoteHosts`], which the daemon installs only while
//! Forge Anywhere is on; with it off every function here is purely local.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use forge_core::fleet::{
    describe_peers, resolve_target, FleetHostStatus, FleetPeer, FleetSessionInfo, ListFilter,
    MessageMode, ResolveError, SendReceipt, SessionListing, AUTO_REPLY_LIMIT, CHAIN_HARD_LIMIT,
    MAX_MESSAGE_BYTES, RATE_PER_MINUTE,
};
use forge_store::Store;
use futures::SinkExt as _;

use crate::serve::{
    deliver_pending_fleet_messages, err_response, json_response, DaemonState, SessionRegistry,
};

/// What a remote host should do with a message that came from a session on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteMessage {
    pub text: String,
    pub mode: MessageMode,
    /// `<this host>/<session id>`, the key the receiving host's guards track.
    pub sender_id: String,
    /// `<this host>/<session title>`.
    pub sender_label: String,
}

/// One other machine's sessions, or why they are not available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteHostSessions {
    pub status: FleetHostStatus,
    pub sessions: Vec<FleetSessionInfo>,
}

/// The user's other machines, reached over the end-to-end encrypted Anywhere relay.
#[async_trait::async_trait]
pub(crate) trait RemoteHosts: Send + Sync {
    /// This machine's Anywhere host name, used to label what it sends.
    fn local_name(&self) -> String;
    /// Every other host with its sessions; an unreachable host comes back with `online: false`.
    async fn list(&self) -> Vec<RemoteHostSessions>;
    /// Deliver `message` into session `session_id` on `host`.
    async fn send(
        &self,
        host: &str,
        session_id: &str,
        message: &RemoteMessage,
    ) -> Result<(), String>;
}

/// Who a message is from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sender {
    /// `"session"` (an agent) or `"cli"` (a human at a shell).
    pub kind: &'static str,
    pub id: Option<String>,
    pub label: String,
}

impl Sender {
    pub(crate) fn cli() -> Self {
        Self {
            kind: "cli",
            id: None,
            label: "cli".into(),
        }
    }
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

fn state_word(busy: bool, waiting: bool) -> &'static str {
    if waiting {
        "waiting"
    } else if busy {
        "busy"
    } else {
        "idle"
    }
}

/// Sessions on this machine: the daemon's own drivers plus `forge chat` terminals that published
/// presence. A session id never appears twice (the daemon-driven row wins).
pub(crate) async fn local_sessions(
    store: &Arc<Store>,
    registry: &SessionRegistry,
    self_id: Option<&str>,
) -> Vec<FleetSessionInfo> {
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for handle in registry.all().await {
        seen.insert(handle.session_id.clone());
        let snap = handle.snapshot_rx.borrow().snapshot.clone();
        rows.push(FleetSessionInfo {
            id: handle.session_id.clone(),
            title: handle.title(),
            cwd: handle.cwd.clone(),
            model: snap.model,
            state: state_word(
                snap.busy,
                snap.permission_prompt.is_some() || snap.question.is_some(),
            )
            .into(),
            host: None,
            terminal: false,
            can_receive: true,
            is_me: self_id == Some(handle.session_id.as_str()),
        });
    }
    let presence = store.clone();
    let terminals = tokio::task::spawn_blocking(move || presence.local_live_sessions())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    for t in terminals {
        if !seen.insert(t.id.clone()) {
            continue;
        }
        rows.push(FleetSessionInfo {
            is_me: self_id == Some(t.id.as_str()),
            id: t.id,
            title: t.title.unwrap_or_default(),
            cwd: t.cwd,
            model: t.model.unwrap_or_default(),
            state: state_word(t.busy, false).into(),
            host: None,
            terminal: true,
            can_receive: t.control_url.is_some(),
        });
    }
    rows.sort_by(|a, b| a.cwd.cmp(&b.cwd).then(a.id.cmp(&b.id)));
    rows
}

fn remote_hosts(registry: &SessionRegistry) -> Option<Arc<dyn RemoteHosts>> {
    registry
        .remote_hosts
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

impl SessionRegistry {
    /// Install (or, with `None`, remove) the Anywhere transport to the user's other machines.
    pub(crate) fn set_remote_hosts(&self, hosts: Option<Arc<dyn RemoteHosts>>) {
        *self
            .remote_hosts
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = hosts;
    }
}

/// The full listing for `list_sessions`: this machine, then (unless `local_only`) other hosts.
pub(crate) async fn list_all(
    store: &Arc<Store>,
    registry: &SessionRegistry,
    self_id: Option<&str>,
    filter: &ListFilter,
) -> SessionListing {
    let mut sessions = local_sessions(store, registry, self_id).await;
    if let Some(cwd) = &filter.cwd {
        sessions.retain(|s| forge_core::fleet::cwd_matches(&s.cwd, cwd));
    }
    let mut hosts = Vec::new();
    if !filter.local_only {
        if let Some(remote) = remote_hosts(registry) {
            for host in remote.list().await {
                hosts.push(host.status);
                sessions.extend(host.sessions);
            }
        }
    }
    SessionListing { sessions, hosts }
}

fn peers_of(sessions: &[FleetSessionInfo]) -> Vec<FleetPeer> {
    sessions
        .iter()
        .map(|s| FleetPeer {
            id: s.id.clone(),
            title: (!s.title.is_empty()).then(|| s.title.clone()),
            host: s.host.clone(),
        })
        .collect()
}

fn candidates(peers: &[FleetPeer], address: &str) -> Vec<FleetPeer> {
    let tail = address.rsplit('/').next().unwrap_or(address);
    peers
        .iter()
        .filter(|p| p.title.as_deref() == Some(tail) || p.id.starts_with(tail))
        .cloned()
        .collect()
}

/// Resolve `address` and deliver. Local sessions are tried first, and only a miss (or a
/// `host/name` address) reaches for the network.
pub(crate) async fn send_from(
    store: &Arc<Store>,
    registry: &SessionRegistry,
    sender: &Sender,
    address: &str,
    mode: MessageMode,
    text: &str,
) -> Result<SendReceipt, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("message must not be empty".into());
    }
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(format!(
            "message is {} bytes, exceeds the 16KB limit",
            text.len()
        ));
    }
    let local = local_sessions(store, registry, sender.id.as_deref()).await;
    let mut sessions = local.clone();
    let mut peers = peers_of(&local);
    let mut resolved = resolve_target(&peers, address).cloned();
    let mut hosts_seen = false;
    if matches!(resolved, Err(ResolveError::NotFound)) {
        if let Some(remote) = remote_hosts(registry) {
            for host in remote.list().await {
                hosts_seen = true;
                sessions.extend(host.sessions);
            }
            peers = peers_of(&sessions);
            resolved = resolve_target(&peers, address).cloned();
        }
    }
    let peer = match resolved {
        Ok(p) => p,
        Err(ResolveError::NotFound) => {
            let scope = if hosts_seen {
                "Live sessions (all machines)"
            } else {
                "Live sessions"
            };
            return Err(format!(
                "no session matches '{address}'. {scope}: [{}]",
                describe_peers(&peers)
            ));
        }
        Err(e @ ResolveError::Ambiguous(_)) => {
            return Err(format!(
                "{e} for '{address}'. Candidates: [{}] — use a longer id prefix or `host/name`",
                describe_peers(&candidates(&peers, address))
            ));
        }
    };
    if sender.id.as_deref() == Some(peer.id.as_str()) && peer.host.is_none() {
        return Err("you cannot message your own session".into());
    }
    let label = peer.title.clone().unwrap_or_else(|| "unnamed".into());
    match &peer.host {
        None => {
            let info = sessions
                .iter()
                .find(|s| s.id == peer.id && s.host.is_none());
            let no_reply =
                submit_local(store, registry, sender, &peer.id, info, mode, text).await?;
            Ok(SendReceipt {
                label,
                id: peer.id,
                host: None,
                mode: mode.as_str().into(),
                no_reply,
            })
        }
        Some(host) => {
            let remote = remote_hosts(registry).ok_or("Forge Anywhere is not available")?;
            let me = remote.local_name();
            let sender_id = sender
                .id
                .as_deref()
                .map_or_else(|| format!("{me}/cli"), |id| format!("{me}/{id}"));
            remote
                .send(
                    host,
                    &peer.id,
                    &RemoteMessage {
                        text: text.to_string(),
                        mode,
                        sender_id,
                        sender_label: format!("{me}/{}", sender.label),
                    },
                )
                .await?;
            Ok(SendReceipt {
                label,
                id: peer.id,
                host: Some(host.clone()),
                mode: mode.as_str().into(),
                no_reply: false,
            })
        }
    }
}

/// Guard, persist and deliver one message to a session on this machine. Returns whether the
/// recipient was told not to reply automatically.
pub(crate) async fn submit_local(
    store: &Arc<Store>,
    registry: &SessionRegistry,
    sender: &Sender,
    target_id: &str,
    info: Option<&FleetSessionInfo>,
    mode: MessageMode,
    text: &str,
) -> Result<bool, String> {
    if sender.id.as_deref() == Some(target_id) {
        return Err("you cannot message your own session".into());
    }
    let live = registry.get(target_id).await.is_some();
    if !live {
        match info {
            None => return Err("no live session with that id".into()),
            Some(i) if i.terminal && !i.can_receive => {
                return Err(format!(
                    "session {} is a watch-only terminal (no control channel), it cannot receive \
                     messages",
                    short(target_id)
                ));
            }
            Some(_) => {}
        }
    }
    let now = chrono::Utc::now().timestamp();
    let mut no_reply = false;
    if let (true, Some(sender_id)) = (sender.kind == "session", sender.id.as_deref()) {
        let activity = store
            .fleet_pair_activity(sender_id, target_id, now)
            .map_err(|e| e.to_string())?;
        if activity.last_minute >= RATE_PER_MINUTE {
            return Err(format!(
                "rate limit: {RATE_PER_MINUTE} messages per minute to the same session — wait \
                 before sending more"
            ));
        }
        let depth = activity.chain + 1;
        if depth > CHAIN_HARD_LIMIT {
            return Err(format!(
                "conversation depth limit: {CHAIN_HARD_LIMIT} consecutive agent messages to this \
                 session with no human in between — stop and let your user decide"
            ));
        }
        no_reply = depth >= AUTO_REPLY_LIMIT;
    }
    store
        .enqueue_fleet_message(
            &forge_types::new_id(),
            sender.kind,
            sender.id.as_deref(),
            &sender.label,
            target_id,
            text,
            mode.as_str(),
        )
        .map_err(|e| e.to_string())?;
    if live {
        deliver_pending_fleet_messages(store, registry, target_id).await;
    } else {
        deliver_to_terminal(store, target_id).await?;
    }
    Ok(no_reply)
}

/// Whether the message at `created_at` is deep enough in an agent-to-agent exchange that its
/// recipient is told not to reply on its own. Recomputed at delivery so a message that waited for
/// a restart is framed exactly as it would have been at send time.
pub(crate) fn frame_no_reply(store: &Store, msg: &forge_store::FleetMessage) -> bool {
    let (true, Some(sender_id)) = (msg.sender_kind == "session", msg.sender_id.as_deref()) else {
        return false;
    };
    store
        .fleet_pair_activity(sender_id, &msg.target_session_id, msg.created_at)
        .is_ok_and(|a| a.chain >= AUTO_REPLY_LIMIT)
}

/// The text a delivered row becomes in the recipient's input queue.
pub(crate) fn delivered_text(store: &Store, msg: &forge_store::FleetMessage) -> String {
    forge_core::fleet::peer_message_text(
        &msg.sender_kind,
        &msg.sender_label,
        msg.sender_id.as_deref(),
        &msg.body,
        frame_no_reply(store, msg),
    )
}

/// The control-channel frame for a delivery: `RemoteInput`'s wire shape (it only derives
/// `Deserialize`, and a test below keeps the two in step).
fn terminal_input(mode: &str, text: String) -> String {
    let kind = if mode == "steer" { "steer" } else { "prompt" };
    serde_json::json!({ "kind": kind, "text": text }).to_string()
}

fn control_ws_url(base: &str) -> String {
    let ws = match base.split_once("://") {
        Some(("https", rest)) => format!("wss://{rest}"),
        Some((_, rest)) => format!("ws://{rest}"),
        None => format!("ws://{base}"),
    };
    format!("{}/ws", ws.trim_end_matches('/'))
}

/// Push every pending row for a `forge chat` terminal session into its published control channel,
/// the same WebSocket the phone app drives it through. Rows are marked delivered either way: a
/// terminal is not in the registry, so nothing would ever flush them later and a stuck row would
/// only eat the sender's pending cap.
async fn deliver_to_terminal(store: &Arc<Store>, target_id: &str) -> Result<(), String> {
    let pending = store
        .pending_fleet_messages_for(target_id)
        .map_err(|e| e.to_string())?;
    let control = store.local_session_control_url(target_id).ok().flatten();
    let mut outcome = Ok(());
    for msg in pending {
        let input = terminal_input(&msg.mode, delivered_text(store, &msg));
        if outcome.is_ok() {
            outcome = match &control {
                None => Err("that terminal session has no control channel".to_string()),
                Some(base) => push_to_terminal(base, &input).await,
            };
        }
        let _ = store.mark_fleet_message_delivered(&msg.id, chrono::Utc::now().timestamp());
    }
    outcome
}

async fn push_to_terminal(base: &str, payload: &str) -> Result<(), String> {
    let url = control_ws_url(base);
    let (mut ws, _) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async(url.as_str()),
    )
    .await
    .map_err(|_| "timed out reaching the terminal session".to_string())?
    .map_err(|e| format!("could not reach the terminal session (is it still running?): {e}"))?;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        payload.to_string().into(),
    ))
    .await
    .map_err(|e| format!("terminal session refused the message: {e}"))?;
    let _ = ws.close(None).await;
    Ok(())
}

/// `FleetMessaging` for a process that is not the daemon — the CLI bridge child and a `forge
/// chat` terminal — over the daemon's `/api/peers/*` routes. The daemon resolves, guards and
/// delivers, so this side cannot drift from the in-process one.
pub(crate) struct HttpFleet {
    /// `(base url, token)` instead of the configured local daemon; tests point it at a scratch
    /// router.
    daemon: Option<(String, String)>,
}

impl HttpFleet {
    pub(crate) fn local() -> Self {
        Self { daemon: None }
    }

    #[cfg(test)]
    pub(crate) fn at(base: String, token: String) -> Self {
        Self {
            daemon: Some((base, token)),
        }
    }

    fn endpoint(&self, path: &str) -> Result<(reqwest::Client, String), String> {
        let (base, token) = match &self.daemon {
            Some(pair) => pair.clone(),
            None => (
                crate::attach::resolve_base_url(None),
                crate::attach::resolve_token(None).map_err(|e| e.to_string())?,
            ),
        };
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok((http, format!("{base}/{token}{path}")))
    }

    /// Whether a daemon answers, so a terminal session only offers the tools when they can work.
    pub(crate) async fn reachable() -> bool {
        let Ok((_, url)) = Self::local().endpoint("/api/peers/sessions?local_only=true") else {
            return false;
        };
        let Ok(http) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(400))
            .build()
        else {
            return false;
        };
        http.get(url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }
}

#[async_trait::async_trait]
impl forge_core::fleet::FleetMessaging for HttpFleet {
    async fn list(&self, me: &str, filter: &ListFilter) -> SessionListing {
        let fetched = async {
            let (http, url) = self.endpoint("/api/peers/sessions")?;
            let mut query = vec![("local_only", filter.local_only.to_string())];
            if !me.is_empty() {
                query.push(("self", me.to_string()));
            }
            if let Some(cwd) = &filter.cwd {
                query.push(("cwd", cwd.clone()));
            }
            let resp = http.get(url).query(&query).send().await.map_err(|e| {
                format!("could not reach the forge serve daemon — is it running? [{e}]")
            })?;
            if !resp.status().is_success() {
                return Err(format!("daemon answered {}", resp.status()));
            }
            resp.json::<SessionListing>()
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        fetched.unwrap_or_default()
    }

    async fn send(
        &self,
        me: &str,
        address: &str,
        mode: MessageMode,
        text: &str,
    ) -> Result<SendReceipt, String> {
        let (http, url) = self.endpoint("/api/peers/message")?;
        let resp = http
            .post(url)
            .json(&serde_json::json!({
                "target": address,
                "text": text,
                "mode": mode.as_str(),
                "sender_id": (!me.is_empty()).then_some(me),
            }))
            .send()
            .await
            .map_err(|e| {
                format!("could not reach the forge serve daemon — is it running? [{e}]")
            })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status.is_success() {
            return serde_json::from_str(&body).map_err(|e| e.to_string());
        }
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or(body);
        Err(format!("daemon rejected the message ({status}): {detail}"))
    }
}

// ---------------------------------------------------------------------------
// HTTP surface
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendFleetMessageReq {
    text: String,
    #[serde(default)]
    mode: Option<String>,
    /// `"cli"` (a `forge send` invocation, the default) or `"session"` (an agent, possibly on
    /// another machine).
    #[serde(default)]
    sender_kind: Option<String>,
    /// Display name for the delivered header; defaults to the sender kind.
    #[serde(default)]
    sender_label: Option<String>,
    /// The sending session's id (`<host>/<id>` from another machine).
    #[serde(default)]
    sender_id: Option<String>,
}

fn sender_of(req: &SendFleetMessageReq) -> Sender {
    let kind = if req.sender_kind.as_deref() == Some("session") {
        "session"
    } else {
        "cli"
    };
    Sender {
        kind,
        id: req.sender_id.clone().filter(|_| kind == "session"),
        label: req
            .sender_label
            .clone()
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| kind.to_string()),
    }
}

/// `POST /api/sessions/{id}/message` — `forge send`, and the message another machine's session
/// delivers over the Anywhere relay. The target is already resolved (an exact id).
pub(crate) async fn send_fleet_message(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    axum::Json(req): axum::Json<SendFleetMessageReq>,
) -> Response {
    if state.registry.is_draining() {
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "daemon is draining for shutdown",
        );
    }
    let text = req.text.trim();
    if text.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "message must not be empty");
    }
    if text.len() > MAX_MESSAGE_BYTES {
        return err_response(StatusCode::BAD_REQUEST, "message exceeds the 16KB limit");
    }
    let mode_raw = req.mode.as_deref().unwrap_or("follow_up");
    let Some(mode) = MessageMode::parse(mode_raw) else {
        return err_response(
            StatusCode::BAD_REQUEST,
            &format!("unknown mode '{mode_raw}' — use follow_up or steer"),
        );
    };
    let sessions = local_sessions(&state.store, &state.registry, None).await;
    let info = sessions.iter().find(|s| s.id == id);
    if info.is_none() {
        return err_response(
            StatusCode::NOT_FOUND,
            "no live fleet session with that id — see GET /api/sessions",
        );
    }
    match submit_local(
        &state.store,
        &state.registry,
        &sender_of(&req),
        &id,
        info,
        mode,
        text,
    )
    .await
    {
        Ok(no_reply) => json_response(
            &serde_json::json!({ "ok": true, "mode": mode.as_str(), "no_reply": no_reply }),
        ),
        Err(error) => err_response(StatusCode::BAD_REQUEST, &error),
    }
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct PeersQuery {
    /// The caller's own session id, to mark its row.
    #[serde(rename = "self")]
    self_id: Option<String>,
    cwd: Option<String>,
    #[serde(default)]
    local_only: bool,
}

/// `GET /api/peers/sessions` — the listing behind `list_sessions` for processes that have no
/// registry of their own (the CLI bridge, a terminal session).
pub(crate) async fn peers_sessions(
    State(state): State<Arc<DaemonState>>,
    axum::Extension(auth): axum::Extension<crate::serve_clients::ClientAuth>,
    Query(q): Query<PeersQuery>,
) -> Response {
    let filter = ListFilter {
        cwd: q.cwd.filter(|c| !c.trim().is_empty()),
        // A client paired to this host sees this host's sessions, not the account's other machines.
        local_only: q.local_only || !auth.is_owner(),
    };
    json_response(&list_all(&state.store, &state.registry, q.self_id.as_deref(), &filter).await)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PeerMessageReq {
    target: String,
    text: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    sender_id: Option<String>,
    #[serde(default)]
    sender_label: Option<String>,
}

/// `POST /api/peers/message` — address resolution (including other machines) and delivery for
/// processes without a registry. The sender's title is read live from the registry when it is a
/// daemon-hosted session, so a rename shows up in its next message.
pub(crate) async fn peers_message(
    State(state): State<Arc<DaemonState>>,
    axum::Json(req): axum::Json<PeerMessageReq>,
) -> Response {
    if state.registry.is_draining() {
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "daemon is draining for shutdown",
        );
    }
    let mode_raw = req.mode.as_deref().unwrap_or("follow_up");
    let Some(mode) = MessageMode::parse(mode_raw) else {
        return err_response(
            StatusCode::BAD_REQUEST,
            &format!("unknown mode '{mode_raw}' — use follow_up or steer"),
        );
    };
    let sender = match req.sender_id.clone().filter(|s| !s.trim().is_empty()) {
        None => Sender::cli(),
        Some(id) => {
            let live_title = match state.registry.get(&id).await {
                Some(h) => h.title(),
                None => local_sessions(&state.store, &state.registry, None)
                    .await
                    .into_iter()
                    .find(|s| s.id == id)
                    .map(|s| s.title)
                    .unwrap_or_default(),
            };
            let label = Some(live_title)
                .filter(|t| !t.is_empty())
                .or_else(|| req.sender_label.clone().filter(|l| !l.trim().is_empty()))
                .unwrap_or_else(|| short(&id).to_string());
            Sender {
                kind: "session",
                id: Some(id),
                label,
            }
        }
    };
    match send_from(
        &state.store,
        &state.registry,
        &sender,
        &req.target,
        mode,
        &req.text,
    )
    .await
    {
        Ok(receipt) => json_response(&receipt),
        Err(error) => err_response(StatusCode::BAD_REQUEST, &error),
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn terminal_frames_deserialize_into_the_remote_input_protocol() {
        use crate::remote::RemoteInput;
        assert_eq!(
            serde_json::from_str::<RemoteInput>(&terminal_input("follow_up", "hi".into())).unwrap(),
            RemoteInput::Prompt {
                text: "hi".into(),
                attachments: Vec::new()
            }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(&terminal_input("steer", "now".into())).unwrap(),
            RemoteInput::Steer { text: "now".into() }
        );
    }

    #[test]
    fn control_urls_become_websocket_urls() {
        assert_eq!(
            control_ws_url("http://127.0.0.1:41/tok"),
            "ws://127.0.0.1:41/tok/ws"
        );
        assert_eq!(control_ws_url("https://h/tok/"), "wss://h/tok/ws");
    }
}
