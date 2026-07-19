//! `forge serve` — the headless multi-session daemon (docs/features/remote-control.md).
//!
//! One long-lived process hosts N concurrent sessions, each driven by a headless
//! [`SessionDriver`](crate::cli::commands::run) task, all reachable from the SAME control page
//! the in-chat `/remote` serves — at a **stable origin**: a configured port (`[remote] port`,
//! default 7420) plus a daemon token persisted (0600) in the config dir. Stable origin ⇒ the
//! PWA's `scope`/`start_url` never change, so installing it to a phone home screen survives
//! forever — across daemon restarts, session ends, everything. Sessions keep running with zero
//! clients attached and are addressed with `?session=<id>` on the WS and history routes.
//!
//! Routes (all under `/<daemon-token>/`, wrong token = 404):
//! - `GET  /`                         control page (session list + the live session UI)
//! - `GET  /app.js|styles.css|manifest.webmanifest|sw.js|icon.svg`  PWA assets
//! - `WS   /ws?session=<id>&rev=<n>`  per-session stream with replay-from-rev
//! - `WS   /ws/fleet`                 low-bandwidth invalidation stream for fleet clients
//! - `GET  /api/sessions`             running sessions (id, title, cwd, busy, cost, activity)
//! - `GET  /api/sessions/past?limit=&before=`  persisted sessions NOT currently running — the
//!   resurrection browser's data; `resume` one with `POST /api/sessions {resume:<id>}`
//! - `POST /api/sessions`             create ({cwd, worktree, title?, model?, resume?, temper?})
//!   — `temper` starts the session in a given permission mode (case-insensitive: `Read-only`,
//!   `Ask`, `Auto-edit`, `Full`; unknown values are a `400`, never a silent default)
//! - `GET  /api/projects`             daemon default + recent project directories + browse roots
//! - `GET  /api/projects/browse?path=` allowlisted server-side directory browser
//! - `POST /api/sessions/{id}/archive` stop + hide a session (history kept; worktree kept)
//! - `POST /api/sessions/{id}/merge`   stop, snapshot the worktree, and merge its branch back
//!   into the base repo (3-way patch; conflicts are reported, never auto-resolved)
//! - `POST /api/sessions/{id}/discard` stop and drop the worktree + branch WITHOUT merging
//! - `GET  /api/history?session=<id>&before=<seq>&limit=<n>`  scrollback pagination
//! - `GET  /api/push/key`             the VAPID public key (`applicationServerKey`)
//! - `POST /api/push/subscribe`       store a Web Push subscription (dedupe by endpoint)
//! - `POST /api/push/unsubscribe`     remove one
//! - `POST /api/answer`               approve/deny a pending permission prompt over plain HTTP —
//!   the service worker calls this from a notification action (no page needed); the `seq` is
//!   validated exactly like the WS path (stale ⇒ 409, and the driver re-validates on receipt)
//! - `POST /api/voice/transcribe`     local whisper.cpp speech-to-text (voice.md, V1): multipart
//!   audio (wav/m4a/aac/mp4) + optional `?language=` -> `{"text": "..."}`. Session-independent —
//!   the model downloads into `{data_dir}/models/whisper/` on first use and is cached in memory.
//!
//! Exposure mirrors `/remote`: `--lan` (default) binds 0.0.0.0 with self-signed HTTPS, `--local`
//! binds loopback plain HTTP, `--tunnel` binds loopback and opens a cloudflared/ngrok tunnel —
//! by default an ephemeral quick tunnel (new random URL every launch); set `[remote] tunnel_name`
//! (cloudflared named tunnel) or `tunnel_hostname` alone (ngrok reserved domain) for a stable URL
//! across restarts (see [`remote::resolve_tunnel_kind`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, Path as AxumPath, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;

use crate::cli::commands::run::{spawn_session_driver, DriverSpec, SessionDriverHandle};
use crate::remote;

/// Fleet rows may change on every streaming snapshot, but clients only need human-scale refreshes.
const FLEET_INVALIDATION_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// How long an archive waits for the driver task to wind down before letting go.
const ARCHIVE_JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The persisted daemon token's file name inside the forge config dir.
const TOKEN_FILE: &str = "serve-token";

/// The discovery state file's name inside the forge config dir — see [`ServeState`].
const STATE_FILE: &str = "serve-state.json";

/// Read (or mint) the persisted daemon token. Unlike `/remote`'s per-session ephemeral token,
/// this one is generated ONCE and reused forever so the PWA origin stays stable; `rotate`
/// regenerates it (invalidating every installed PWA/link — deliberate, for revocation).
/// The file is created 0600 (owner-only) — it is the sole authentication for the daemon.
pub(crate) fn daemon_token(rotate: bool) -> Result<String> {
    let dir = forge_config::config_dir().context("no config directory on this platform")?;
    daemon_token_at(&dir.join(TOKEN_FILE), rotate)
}

/// [`daemon_token`] against an explicit path (unit-testable without touching the real config).
pub(crate) fn daemon_token_at(path: &std::path::Path, rotate: bool) -> Result<String> {
    if !rotate {
        if let Ok(existing) = std::fs::read_to_string(path) {
            let t = existing.trim();
            if (16..=64).contains(&t.len()) && t.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(t.to_string());
            }
        }
    }
    // 128 bits from the OS CSPRNG: this token is long-lived and may guard an internet-reachable
    // (`--anywhere`) control channel, so it gets twice the entropy of the ephemeral one.
    let token = format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, &token).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
}

/// Snapshot of a live `forge serve` daemon, written to `<config_dir>/serve-state.json` right
/// after a successful bind so a client that can't run a shell/fs plugin (namely the Tauri
/// desktop app — no such plugin is granted, see `mobile/src-tauri/capabilities/default.json`)
/// can still auto-detect a running daemon and offer to connect instead of asking for a pasted
/// URL. Advisory only: a reader MUST check `pid` is still alive before trusting the file — it
/// is removed on a graceful Ctrl-C shutdown, but a crash or `kill -9` leaves it stale on disk.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub(crate) struct ServeState {
    pub(crate) pid: u32,
    pub(crate) port: u16,
    /// `"local"` (loopback, plain HTTP), `"lan"` (0.0.0.0, self-signed HTTPS — a WebView-based
    /// client can't trust this cert, so auto-connect must not attempt it), or `"anywhere"`
    /// (loopback + public tunnel, real TLS).
    pub(crate) exposure: String,
    /// The full connect URL — the same string `forge serve` prints, ready to hand to a client.
    pub(crate) base_url: String,
    pub(crate) token: String,
    pub(crate) started_at: u64,
}

impl ServeState {
    /// Pure serialization — unit-testable without touching a filesystem.
    fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("ServeState fields are all serializable")
    }

    pub(crate) fn process_is_alive(&self) -> bool {
        process_is_alive(self.pid)
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // Signal zero performs permission/liveness validation without delivering a signal.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

/// [`write_state`] against an explicit path (unit-testable without touching the real config).
fn write_state_at(path: &std::path::Path, state: &ServeState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating state directory {}", parent.display()))?;
    }
    // A reconnecting desktop/mobile client may read this while a daemon is restarting.  Write and
    // fsync a sibling before rename so it can observe either the complete old state or complete new
    // state, never a truncated JSON token/URL halfway through an overwrite.
    let temp = path.with_extension(format!(
        "tmp-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    {
        use std::io::Write;
        let mut file =
            std::fs::File::create(&temp).with_context(|| format!("writing {}", temp.display()))?;
        file.write_all(state.to_json().as_bytes())
            .with_context(|| format!("writing {}", temp.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", temp.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("securing {}", temp.display()))?;
    }
    std::fs::rename(&temp, path).with_context(|| format!("publishing {}", path.display()))?;
    Ok(())
}

/// Write [`ServeState`] to `<config_dir>/serve-state.json`. Best-effort from the caller's
/// perspective — a failure here must never take the daemon down, it just means desktop
/// auto-detect won't find this instance.
pub(crate) fn write_state(state: &ServeState) -> Result<()> {
    let dir = forge_config::config_dir().context("no config directory on this platform")?;
    write_state_at(&dir.join(STATE_FILE), state)
}

/// Read the advisory discovery record used by local CLI commands. Callers must still probe the
/// authenticated daemon endpoint because a crash can leave this file behind.
pub(crate) fn read_state() -> Result<Option<ServeState>> {
    let dir = forge_config::config_dir().context("no config directory on this platform")?;
    let path = dir.join(STATE_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {}", path.display()))
        .map(Some)
}

/// Remove the state file on graceful shutdown, so a dead daemon never LOOKS live to a reader
/// that only checks file existence. The pid-liveness check on the reader side (Tauri's
/// `detect_forge_serve`) is the belt to this suspenders — this just avoids leaving stale
/// advisory data behind after a clean exit.
fn remove_state() {
    if let Some(dir) = forge_config::config_dir() {
        let _ = std::fs::remove_file(dir.join(STATE_FILE));
    }
}

/// The daemon's session registry: id → running driver handle. Mirrors `mcp_serve`'s
/// LocalSessionManager pattern (one task per session, addressed by id).
pub(crate) struct SessionRegistry {
    sessions: tokio::sync::Mutex<std::collections::HashMap<String, Arc<SessionDriverHandle>>>,
    fleet_tx: tokio::sync::watch::Sender<u64>,
}

impl SessionRegistry {
    pub(crate) fn new() -> Self {
        Self {
            sessions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            fleet_tx: tokio::sync::watch::channel(0).0,
        }
    }

    fn notify_fleet(&self) {
        let next = self.fleet_tx.borrow().wrapping_add(1);
        self.fleet_tx.send_replace(next);
    }

    fn subscribe_fleet(&self) -> tokio::sync::watch::Receiver<u64> {
        self.fleet_tx.subscribe()
    }

    pub(crate) async fn insert(&self, handle: SessionDriverHandle) -> Arc<SessionDriverHandle> {
        let handle = Arc::new(handle);
        let mut snapshot_rx = handle.snapshot_rx.clone();
        let fleet_tx = self.fleet_tx.clone();
        tokio::spawn(async move {
            let mut next_allowed = tokio::time::Instant::now();
            let mut was_waiting = false;
            while snapshot_rx.changed().await.is_ok() {
                // Snapshot frames can change every ~30 ms while text streams. Coalesce bursts
                // once here for every fleet adapter instead of making each client defend itself.
                let waiting = {
                    let snapshot = snapshot_rx.borrow_and_update();
                    snapshot.snapshot.permission_prompt.is_some()
                        || snapshot.snapshot.question.is_some()
                };
                if attention_became_required(was_waiting, waiting) {
                    tokio::spawn(crate::anywhere::notify_attention_required());
                }
                was_waiting = waiting;
                tokio::time::sleep_until(next_allowed).await;
                let next = fleet_tx.borrow().wrapping_add(1);
                fleet_tx.send_replace(next);
                next_allowed = tokio::time::Instant::now() + FLEET_INVALIDATION_MIN_INTERVAL;
            }
        });
        self.sessions
            .lock()
            .await
            .insert(handle.session_id.clone(), handle.clone());
        self.notify_fleet();
        handle
    }

    pub(crate) async fn get(&self, id: &str) -> Option<Arc<SessionDriverHandle>> {
        self.sessions.lock().await.get(id).cloned()
    }

    pub(crate) async fn remove(&self, id: &str) -> Option<Arc<SessionDriverHandle>> {
        let removed = self.sessions.lock().await.remove(id);
        if removed.is_some() {
            self.notify_fleet();
        }
        removed
    }

    pub(crate) async fn all(&self) -> Vec<Arc<SessionDriverHandle>> {
        self.sessions.lock().await.values().cloned().collect()
    }
}

fn attention_became_required(was_waiting: bool, is_waiting: bool) -> bool {
    !was_waiting && is_waiting
}

/// Shared HTTP state for the daemon router.
struct DaemonState {
    registry: Arc<SessionRegistry>,
    store: Arc<forge_store::Store>,
    /// `/ <token>` — injected into the page/manifest like the single-session server does.
    base: String,
    /// Sessions created from the page inherit this (testing: `forge serve --mock`).
    mock: bool,
    /// The daemon process's cwd — the default for new sessions.
    default_cwd: String,
    /// Canonical directories the remote project browser may enumerate. The create-session API
    /// still accepts any explicit directory because possession of the daemon token already grants
    /// full agent control; this list limits passive filesystem disclosure through browsing.
    project_roots: Vec<PathBuf>,
    /// The Web Push sender (`None` when the VAPID key couldn't be loaded/minted — the push
    /// routes then answer 503 and everything else works normally).
    push: Option<Arc<crate::push::PushNotifier>>,
    /// The native iOS (APNs) sender (`None` when `FORGE_APNS_TEAM_ID`/`_KEY_ID`/`_KEY_PATH`
    /// aren't all set — same graceful-absence contract as `push`).
    apns: Option<Arc<crate::apns::ApnsNotifier>>,
    /// Local whisper.cpp speech-to-text (`POST /api/voice/transcribe`) — caches the loaded model
    /// across requests.
    voice: crate::voice::VoiceState,
    /// Wakes the one-shot managed connector supervisor after `forge anywhere enable` updates an
    /// already-running daemon. Repeated notifications are harmless.
    anywhere_enable: tokio::sync::watch::Sender<bool>,
}

#[derive(serde::Serialize)]
struct ConfigResponse {
    fields: Vec<ConfigField>,
}

#[derive(serde::Serialize)]
struct ConfigField {
    key: String,
    group: String,
    field_type: String,
    label: String,
    help: Option<String>,
    options: Vec<String>,
    value: String,
    default: String,
    modified: bool,
    source: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateConfigRequest {
    key: String,
    value: Option<String>,
    scope: ConfigScopeRequest,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConfigScopeRequest {
    User,
    Project,
}

impl From<ConfigScopeRequest> for forge_config::ConfigScope {
    fn from(value: ConfigScopeRequest) -> Self {
        match value {
            ConfigScopeRequest::User => Self::User,
            ConfigScopeRequest::Project => Self::Project,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateMcpServerRequest {
    name: String,
    transport: McpTransportRequest,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    url: Option<String>,
    token_env: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum McpTransportRequest {
    Stdio,
    Http,
    Sse,
}

/// One row of `GET /api/sessions` — the fleet dashboard's data. `waiting` is the killer signal
/// (a permission prompt or question is blocking the turn until a human decides); the list is
/// served with waiting sessions FIRST so the dashboard surfaces them without client-side logic.
#[derive(serde::Serialize)]
struct SessionRow {
    id: String,
    title: String,
    cwd: String,
    worktree: Option<String>,
    busy: bool,
    /// A permission prompt or question is pending — the session is blocked on a human.
    waiting: bool,
    cost_usd: f64,
    /// Context-window fill (v7 fleet fields), same numbers the statusline gauge shows.
    context_tokens: u64,
    context_limit: Option<u32>,
    model: String,
    created_at: i64,
    last_activity: i64,
}

/// Fleet ordering: waiting-on-decision first (they need a human NOW), then newest-created.
/// Created-at (not last-activity) as the tiebreak keeps the list stable while sessions stream;
/// the id breaks created-at ties (second granularity) so rows created in the same second don't
/// shuffle between polls with the registry map's iteration order.
fn sort_session_rows(rows: &mut [SessionRow]) {
    rows.sort_by(|a, b| {
        b.waiting
            .cmp(&a.waiting)
            .then(b.created_at.cmp(&a.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Body of `POST /api/sessions`.
#[derive(serde::Deserialize)]
struct CreateSessionReq {
    /// Working directory; defaults to the daemon's cwd.
    cwd: Option<String>,
    /// Run the session in an isolated git worktree branched from HEAD of `cwd`.
    #[serde(default)]
    worktree: bool,
    /// Optional display title.
    title: Option<String>,
    /// Optional model pin.
    model: Option<String>,
    /// Resume an existing session id instead of starting fresh.
    resume: Option<String>,
    /// Start the session directly in this temper/permission-mode instead of the default,
    /// avoiding a follow-up SHIFT+TAB/`/mode` round trip. Case-insensitive; accepts the same
    /// names the `/mode` picker shows (`Read-only`, `Ask`, `Auto-edit`, `Full`) — see
    /// [`forge_types::PermissionMode::from_label`]. An unrecognized value is a `400`, never a
    /// silent fallback to the default temper.
    temper: Option<String>,
}

#[derive(Clone, serde::Serialize)]
struct ProjectRow {
    path: String,
    name: String,
    is_git_repo: bool,
    last_activity: Option<i64>,
}

#[derive(serde::Serialize)]
struct ProjectCatalog {
    default_cwd: String,
    recent: Vec<ProjectRow>,
    roots: Vec<ProjectRow>,
}

#[derive(serde::Deserialize)]
struct BrowseProjectsQuery {
    path: Option<String>,
}

#[derive(serde::Serialize)]
struct BrowseProjectsResponse {
    path: String,
    parent: Option<String>,
    entries: Vec<ProjectRow>,
    roots: Vec<ProjectRow>,
    truncated: bool,
}

pub(crate) async fn serve_cmd(
    local: bool,
    anywhere: bool,
    port: Option<u16>,
    rotate_token: bool,
    mock: bool,
) -> Result<()> {
    let config = forge_config::load().unwrap_or_default();
    let port = port.unwrap_or_else(|| config.remote.serve_port());
    let token = daemon_token(rotate_token)?;
    if rotate_token {
        println!("⚒ daemon token rotated — previously installed PWAs/links are now invalid");
    }

    let store = Arc::new(crate::open_store()?);
    let registry = Arc::new(SessionRegistry::new());
    let default_cwd = std::env::current_dir()
        .and_then(|cwd| cwd.canonicalize())
        .context("resolving daemon workspace cwd")?
        .display()
        .to_string();
    let project_roots =
        resolve_project_roots(Path::new(&default_cwd), &config.remote.project_roots);
    let base = format!("/{token}");
    // Web Push: best-effort. A missing/broken VAPID key disables push (503 on its routes) but
    // must never take the daemon down with it.
    let push = match crate::push::PushNotifier::new(store.clone()) {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            eprintln!("⚠ web push disabled — VAPID key unavailable: {e}");
            None
        }
    };
    // Native push (ADR-0012): bring-your-own Apple key always wins when set — fully local, no
    // relay involvement. Otherwise default to the hosted relay (zero setup, works out of the
    // box) unless the operator opts out entirely. Same graceful-absence contract as Web Push
    // above: never blocks daemon startup. Precedence logic lives in
    // `apns::choose_apns_backend` so it has its own unit test.
    let apns = match crate::apns::choose_apns_backend() {
        crate::apns::ApnsChoice::Direct(config) => {
            match crate::apns::ApnsNotifier::new_direct(store.clone(), config) {
                Ok(n) => Some(Arc::new(n)),
                Err(e) => {
                    eprintln!("⚠ native push disabled — APNs key invalid: {e}");
                    None
                }
            }
        }
        crate::apns::ApnsChoice::Relay {
            base_url,
            relay_token,
        } => {
            match crate::apns::ApnsNotifier::new_relay(store.clone(), base_url.clone(), relay_token)
            {
                Ok(n) => {
                    println!(
                    "⚒ native push via hosted relay ({base_url}) — bring your own key with \
                     FORGE_APNS_TEAM_ID/_KEY_ID/_KEY_PATH, or opt out with FORGE_APNS_DISABLE_RELAY=1"
                );
                    Some(Arc::new(n))
                }
                Err(e) => {
                    eprintln!("⚠ native push disabled — relay unavailable: {e}");
                    None
                }
            }
        }
        crate::apns::ApnsChoice::Disabled => None,
    };
    let (anywhere_enable, anywhere_rx) = tokio::sync::watch::channel(config.anywhere.enabled);
    let state = Arc::new(DaemonState {
        registry: registry.clone(),
        store,
        base: base.clone(),
        mock,
        default_cwd,
        project_roots,
        push,
        apns,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable,
    });
    let app = daemon_router(state);

    // Forge Anywhere gets a private loopback copy of the SAME router. The public LAN/local/tunnel
    // listener below is unchanged, while the connector never needs to upload the daemon token or
    // trust a self-signed LAN certificate. Both background tasks are best-effort: managed-service
    // failures must never prevent local Forge from starting or staying available.
    let anywhere_task = tokio::spawn(anywhere_supervisor(anywhere_rx, app.clone(), token.clone()));

    // Bind + expose, mirroring `/remote`: LAN = 0.0.0.0 + self-signed HTTPS; local/anywhere =
    // loopback plain HTTP (a tunnel terminates TLS at the provider).
    let bind_ip: std::net::IpAddr = if local || anywhere {
        std::net::Ipv4Addr::LOCALHOST.into()
    } else {
        std::net::Ipv4Addr::UNSPECIFIED.into()
    };
    let listener = std::net::TcpListener::bind((bind_ip, port)).with_context(|| {
        format!(
            "binding {bind_ip}:{port} — is another forge serve running? \
             (`[remote] port` or --port picks a different one)"
        )
    })?;
    let addr = listener.local_addr()?;
    listener.set_nonblocking(true)?;

    let mut tunnel_child = None;
    let url = if anywhere {
        let kind =
            remote::resolve_tunnel_kind(&config.remote).map_err(|e| anyhow::anyhow!("{e}"))?;
        let fixed = remote::preferred_tunnel_kind(&config.remote).is_some();
        println!(
            "⚒ opening a public tunnel via {} ({})…",
            kind.label(),
            if fixed {
                "stable URL"
            } else {
                "quick tunnel — new URL every launch"
            }
        );
        let (public, child) = remote::spawn_tunnel(kind, addr.port(), &config.remote).await?;
        tunnel_child = Some(child);
        format!("{}/{token}", public.trim_end_matches('/'))
    } else if local {
        format!("http://127.0.0.1:{}/{token}", addr.port())
    } else {
        let host = remote::lan_display_host(config.remote.host.as_deref(), addr);
        format!("https://{host}:{}/{token}", addr.port())
    };

    // Discovery state for clients that can't run a shell/fs plugin (the Tauri desktop app):
    // `url` above is already exactly `base_url` for every exposure mode (local/lan/anywhere all
    // build it as `scheme://host:port/{token}`), so it's reused as-is here.
    let exposure = if anywhere {
        "anywhere"
    } else if local {
        "local"
    } else {
        "lan"
    };
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Err(e) = write_state(&ServeState {
        pid: std::process::id(),
        port: addr.port(),
        exposure: exposure.to_string(),
        base_url: url.clone(),
        token: token.clone(),
        started_at,
    }) {
        eprintln!(
            "⚠ could not write serve-state.json — desktop auto-detect won't find this daemon: {e}"
        );
    }

    println!("⚒ forge serve — multi-session daemon");
    println!("  listening on {addr} (stable port; sessions survive disconnects)");
    println!("  connect: {url}");
    if let Some(qr) = remote::qr_lines(&url) {
        for line in qr {
            println!("{line}");
        }
    }
    if anywhere {
        println!("  ⚠ anyone with the link can drive these sessions — the token is the only gate");
    }

    // Serve until Ctrl-C, then wind sessions down cleanly.
    let server = async {
        if local || anywhere {
            let tokio_listener = tokio::net::TcpListener::from_std(listener)?;
            axum::serve(tokio_listener, app).await?;
        } else {
            let host = remote::lan_display_host(config.remote.host.as_deref(), addr);
            let tls = remote::generate_self_signed(vec![host, "localhost".to_string()])
                .map_err(|e| anyhow::anyhow!("self-signed cert generation failed: {e}"))?;
            println!("  TLS fingerprint: {}", tls.fingerprint);
            let tls_config =
                axum_server::tls_rustls::RustlsConfig::from_pem(tls.cert_pem, tls.key_pem).await?;
            axum_server::from_tcp_rustls(listener, tls_config)?
                .serve(app.into_make_service())
                .await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let serve_result = tokio::select! {
        r = server => r,
        _ = tokio::signal::ctrl_c() => {
            println!("\n⚒ shutting down — stopping sessions…");
            for handle in registry.all().await {
                handle.shutdown();
            }
            // Bounded: a wedged driver must not hold the daemon's exit hostage.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            remove_state();
            Ok(())
        }
    };
    anywhere_task.abort();
    drop(tunnel_child); // kill_on_drop tears the tunnel down with the daemon
    serve_result
}

/// The daemon's full route table over `state.base` — extracted from [`serve_cmd`] so tests can
/// drive the real router (tower `oneshot`) without binding a socket.
fn daemon_router(state: Arc<DaemonState>) -> Router {
    use tower_http::cors::CorsLayer;
    let base = state.base.clone();
    Router::new()
        .route(&base, get(page))
        .route(&format!("{base}/"), get(page))
        .route(&format!("{base}/ws"), get(ws_handler))
        .route(&format!("{base}/ws/fleet"), get(fleet_ws_handler))
        .route(
            &format!("{base}/api/sessions"),
            get(list_sessions).post(create_session),
        )
        .route(&format!("{base}/api/projects"), get(project_catalog))
        .route(&format!("{base}/api/projects/browse"), get(browse_projects))
        .route(&format!("{base}/api/sessions/past"), get(past_sessions))
        .route(&format!("{base}/api/sessions/tree"), get(session_tree))
        .route(
            &format!("{base}/api/sessions/{{id}}/fork"),
            post(fork_session),
        )
        .route(
            &format!("{base}/api/sessions/{{id}}/archive"),
            post(archive_session),
        )
        .route(
            &format!("{base}/api/sessions/{{id}}/merge"),
            post(merge_session),
        )
        .route(
            &format!("{base}/api/sessions/{{id}}/discard"),
            post(discard_session),
        )
        .route(&format!("{base}/api/history"), get(history_page))
        .route(&format!("{base}/api/usage"), get(usage_page))
        .route(&format!("{base}/api/hooks"), get(hooks_page))
        .route(&format!("{base}/api/skills"), get(skills_page))
        .route(&format!("{base}/api/workflows"), get(workflows_page))
        .route(&format!("{base}/api/models"), get(models_page))
        .route(&format!("{base}/api/plans"), get(plans_page))
        .route(
            &format!("{base}/api/mcp"),
            get(mcp_page).post(create_mcp_server),
        )
        .route(
            &format!("{base}/api/config"),
            get(config_page).put(update_config),
        )
        // File/image upload (v7): multipart, session-addressed, stored under the session's
        // `.forge/uploads/<id>/` and delivered to its driver as `RemoteInput::Attach`. The
        // per-route body limit replaces axum's 2 MB default.
        .route(
            &format!("{base}/api/upload"),
            get(serve_upload)
                .post(upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    remote::UPLOAD_BODY_LIMIT,
                )),
        )
        // Local whisper.cpp speech-to-text (voice.md, V1): multipart audio in, `{"text": ...}`
        // out. Session-independent (no `?session=`) — the daemon-wide model cache lives on
        // `DaemonState.voice`, not on any one session.
        .route(
            &format!("{base}/api/voice/transcribe"),
            post(voice_transcribe).layer(axum::extract::DefaultBodyLimit::max(
                crate::voice::VOICE_UPLOAD_BODY_LIMIT,
            )),
        )
        .route(&format!("{base}/api/push/key"), get(push_key))
        .route(&format!("{base}/api/push/subscribe"), post(push_subscribe))
        .route(
            &format!("{base}/api/push/unsubscribe"),
            post(push_unsubscribe),
        )
        .route(&format!("{base}/api/answer"), post(answer))
        .route(
            &format!("{base}/api/anywhere/enable"),
            post(enable_anywhere_connector),
        )
        .route(&format!("{base}/app.js"), get(app_js))
        .route(&format!("{base}/styles.css"), get(styles_css))
        .route(&format!("{base}/manifest.webmanifest"), get(manifest))
        .route(&format!("{base}/sw.js"), get(service_worker))
        .route(&format!("{base}/icon.svg"), get(icon))
        .fallback(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response() })
        // Allow cross-origin browser clients (e.g. the Expo-web build served from
        // localhost:8081) to reach a daemon on a different origin. The daemon is
        // authed by the URL-path token, not cookies, so a permissive policy is safe.
        .layer(CorsLayer::very_permissive())
        .with_state(state)
}

async fn enable_anywhere_connector(State(state): State<Arc<DaemonState>>) -> Response {
    signal_anywhere_enable(&state.anywhere_enable);
    axum::http::StatusCode::NO_CONTENT.into_response()
}

fn signal_anywhere_enable(enabled: &tokio::sync::watch::Sender<bool>) {
    enabled.send_replace(true);
}

async fn anywhere_supervisor(
    mut enabled: tokio::sync::watch::Receiver<bool>,
    app: Router,
    token: String,
) {
    while !*enabled.borrow() {
        tokio::select! {
            changed = enabled.changed() => {
                if changed.is_err() { return; }
            }
            () = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                if forge_config::load().is_ok_and(|config| config.anywhere.enabled) {
                    break;
                }
            }
        }
    }
    match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await {
        Ok(listener) => {
            let port = match listener.local_addr() {
                Ok(address) => address.port(),
                Err(error) => {
                    eprintln!("⚠ Forge Anywhere local bridge address unavailable: {error}");
                    return;
                }
            };
            let _bridge = AbortTask(tokio::spawn(async move {
                if let Err(error) = axum::serve(listener, app).await {
                    eprintln!(
                        "⚠ Forge Anywhere local bridge stopped — local/direct Forge is unaffected: {error}"
                    );
                }
            }));
            let _connector = AbortTask(
                crate::anywhere::spawn_connector(format!("http://127.0.0.1:{port}/{token}")),
            );
            std::future::pending::<()>().await;
        }
        Err(error) => eprintln!(
            "⚠ Forge Anywhere connector disabled for this run — local/direct Forge is unaffected: {error}"
        ),
    }
}

struct AbortTask(tokio::task::JoinHandle<()>);

impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn page(State(state): State<Arc<DaemonState>>) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::X_FRAME_OPTIONS, "DENY"),
            (
                axum::http::header::CONTENT_SECURITY_POLICY,
                remote::PAGE_CSP,
            ),
            (axum::http::header::REFERRER_POLICY, "no-referrer"),
        ],
        remote::CONTROL_PAGE.replace("__BASE__", &state.base),
    )
        .into_response()
}

async fn app_js(State(state): State<Arc<DaemonState>>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        remote::APP_JS.replace("__BASE__", &state.base),
    )
        .into_response()
}

async fn styles_css() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css")],
        remote::STYLES_CSS,
    )
        .into_response()
}

async fn manifest(State(state): State<Arc<DaemonState>>) -> Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/manifest+json",
        )],
        remote::manifest_json(&state.base),
    )
        .into_response()
}

async fn service_worker() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        remote::SERVICE_WORKER,
    )
        .into_response()
}

async fn icon() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        remote::ICON_SVG,
    )
        .into_response()
}

/// `GET /api/sessions` — the fleet list, newest first.
async fn list_sessions(State(state): State<Arc<DaemonState>>) -> Response {
    let mut rows: Vec<SessionRow> = Vec::new();
    for h in state.registry.all().await {
        let snap = h.snapshot_rx.borrow().snapshot.clone();
        rows.push(SessionRow {
            id: h.session_id.clone(),
            title: h.title.clone(),
            cwd: h.cwd.clone(),
            worktree: h.worktree.clone(),
            busy: snap.busy,
            waiting: snap.permission_prompt.is_some() || snap.question.is_some(),
            cost_usd: snap.cost_usd,
            context_tokens: snap.context_tokens,
            context_limit: snap.context_limit,
            model: snap.model,
            created_at: h.created_at,
            last_activity: h.last_activity.load(std::sync::atomic::Ordering::Relaxed),
        });
    }
    sort_session_rows(&mut rows);
    json_response(&rows)
}

fn expand_project_root(raw: &str) -> PathBuf {
    if raw == "~" {
        return std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(raw)
}

fn resolve_project_roots(default_cwd: &Path, configured: &[String]) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(configured.len() + 1);
    for candidate in std::iter::once(default_cwd.to_path_buf())
        .chain(configured.iter().map(|raw| expand_project_root(raw)))
    {
        match candidate.canonicalize() {
            Ok(path) if path.is_dir() => {
                if !roots.contains(&path) {
                    roots.push(path);
                }
            }
            Ok(_) => eprintln!(
                "⚠ remote project root is not a directory: {}",
                candidate.display()
            ),
            Err(error) => eprintln!(
                "⚠ remote project root is unavailable ({}): {error}",
                candidate.display()
            ),
        }
    }
    roots
}

fn project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn project_row(path: &Path, last_activity: Option<i64>) -> ProjectRow {
    ProjectRow {
        path: path.display().to_string(),
        name: project_name(path),
        is_git_repo: path.join(".git").exists(),
        last_activity,
    }
}

fn path_is_browsable(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn browse_project_directory(
    requested: PathBuf,
    roots: Vec<PathBuf>,
) -> std::result::Result<BrowseProjectsResponse, String> {
    let path = requested
        .canonicalize()
        .map_err(|error| format!("project directory is unavailable: {error}"))?;
    if !path.is_dir() {
        return Err("project path is not a directory".to_string());
    }
    if !path_is_browsable(&path, &roots) {
        return Err("project path is outside the configured browse roots".to_string());
    }

    let mut entries = std::fs::read_dir(&path)
        .map_err(|error| format!("project directory cannot be read: {error}"))?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                return None;
            }
            let canonical = entry.path().canonicalize().ok()?;
            path_is_browsable(&canonical, &roots).then(|| project_row(&canonical, None))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.is_git_repo
            .cmp(&a.is_git_repo)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let truncated = entries.len() > 200;
    entries.truncate(200);
    let parent = path
        .parent()
        .filter(|parent| path_is_browsable(parent, &roots))
        .map(|parent| parent.display().to_string());
    let root_rows = roots.iter().map(|root| project_row(root, None)).collect();
    Ok(BrowseProjectsResponse {
        path: path.display().to_string(),
        parent,
        entries,
        roots: root_rows,
        truncated,
    })
}

/// `GET /api/projects` — the zero-input default plus MRU project choices. Worktree sessions are
/// intentionally excluded: their generated directories are implementation details, not durable
/// projects a person should be encouraged to pick again.
async fn project_catalog(State(state): State<Arc<DaemonState>>) -> Response {
    let mut candidates: Vec<(String, i64)> = state
        .registry
        .all()
        .await
        .into_iter()
        .filter(|handle| handle.worktree.is_none())
        .map(|handle| {
            (
                handle.cwd.clone(),
                handle
                    .last_activity
                    .load(std::sync::atomic::Ordering::Relaxed),
            )
        })
        .collect();

    let store = state.store.clone();
    let past = match tokio::task::spawn_blocking(move || store.list_sessions_for_resume()).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("listing recent projects failed: {error}"),
            )
        }
        Err(error) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("listing recent projects task failed: {error}"),
            )
        }
    };
    candidates.extend(
        past.into_iter()
            .filter(|session| session.worktree_path.is_none())
            .map(|session| (session.cwd, session.last_activity)),
    );
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.1));

    let default_path = PathBuf::from(&state.default_cwd);
    let mut seen = std::collections::HashSet::new();
    seen.insert(default_path.clone());
    let recent = candidates
        .into_iter()
        .filter_map(|(raw, last_activity)| {
            let path = PathBuf::from(raw).canonicalize().ok()?;
            if !path.is_dir() || !seen.insert(path.clone()) {
                return None;
            }
            Some(project_row(&path, Some(last_activity)))
        })
        .take(8)
        .collect();
    let roots = state
        .project_roots
        .iter()
        .map(|root| project_row(root, None))
        .collect();

    json_response(&ProjectCatalog {
        default_cwd: state.default_cwd.clone(),
        recent,
        roots,
    })
}

/// `GET /api/projects/browse?path=` — enumerate only canonical descendants of the daemon's
/// configured roots. Canonicalizing before the containment check blocks both `..` traversal and
/// symlink escapes.
async fn browse_projects(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<BrowseProjectsQuery>,
) -> Response {
    let roots = state.project_roots.clone();
    let requested = query
        .path
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| roots.first().cloned());
    let Some(requested) = requested else {
        return err_response(
            axum::http::StatusCode::NOT_FOUND,
            "no project roots are available",
        );
    };

    let result =
        tokio::task::spawn_blocking(move || browse_project_directory(requested, roots)).await;

    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(error) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("browsing project directory failed: {error}"),
        ),
    }
}

/// `POST /api/sessions` — create (optionally in a fresh isolated worktree) and start driving.
async fn create_session(
    State(state): State<Arc<DaemonState>>,
    axum::Json(req): axum::Json<CreateSessionReq>,
) -> Response {
    // Validate `temper` first and fail fast — before any worktree/driver side effect — so an
    // unrecognized value never silently falls back to the default temper.
    let temper = match req.temper.as_deref() {
        Some(raw) => match parse_temper(raw) {
            Ok(mode) => Some(mode),
            Err(msg) => return err_response(axum::http::StatusCode::BAD_REQUEST, &msg),
        },
        None => None,
    };

    let cwd = req
        .cwd
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| state.default_cwd.clone());
    let cwd_path = std::path::Path::new(&cwd);
    if !cwd_path.is_dir() {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            &format!("cwd is not a directory: {cwd}"),
        );
    }

    let cwd = match std::fs::canonicalize(&cwd) {
        Ok(path) if path.is_dir() => path.display().to_string(),
        Ok(path) => {
            return err_response(
                axum::http::StatusCode::BAD_REQUEST,
                &format!("cwd is not a directory: {}", path.display()),
            )
        }
        Err(error) => {
            return err_response(
                axum::http::StatusCode::BAD_REQUEST,
                &format!("cwd is unavailable: {error}"),
            )
        }
    };

    // Keep the guard alive through driver startup so an early error removes its new worktree.
    // Once live, the worktree intentionally outlives the handle for manual review or merge.
    let mut worktree: Option<String> = None;
    let worktree_guard = if req.worktree {
        if !forge_core::worktree::is_git_repo(cwd_path) {
            return err_response(
                axum::http::StatusCode::BAD_REQUEST,
                &format!("worktree: {cwd} is not a git repository"),
            );
        }
        let wt_id = forge_types::new_id().chars().take(12).collect::<String>();
        match forge_core::worktree::WorktreeGuard::create(cwd_path, &wt_id) {
            Ok(guard) => {
                worktree = Some(guard.path().display().to_string());
                Some(guard)
            }
            Err(e) => {
                return err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("worktree create failed: {e}"),
                );
            }
        }
    } else {
        None
    };

    let session_cwd = worktree.clone().unwrap_or_else(|| cwd.clone());
    let is_resume = req.resume.is_some();
    if let Some(session_id) = req.resume.as_deref() {
        match state.store.session_handoff_blocked(session_id) {
            Ok(true) => {
                return err_response(
                    axum::http::StatusCode::CONFLICT,
                    "session is frozen by an Anywhere handoff and cannot be resumed",
                )
            }
            Err(error) => {
                return err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("handoff state check failed: {error}"),
                )
            }
            Ok(false) => {}
        }
    }
    // Idempotent resume: if a driver for this id is ALREADY live — a double-tapped Resume, two
    // phones, or a resume racing an archive's tail flush — return the existing handle instead of
    // spawning a SECOND driver for the same id. Two drivers for one session both call
    // `store.add_message` with independent `next_seq`, colliding seqs and interleaving the
    // transcript. Checking before the spawn closes the reported double-tap window (the first
    // request has already registered by the time the second arrives). A vanishingly small
    // check-then-insert residual remains; the registry's last-writer-wins insert bounds it to one
    // extra short-lived driver rather than unbounded duplication.
    if let Some(rid) = req.resume.as_deref() {
        if let Some(existing) = state.registry.get(rid).await {
            return json_response(&serde_json::json!({
                "id": existing.session_id,
                "title": existing.title,
                "cwd": existing.cwd,
                "worktree": existing.worktree,
            }));
        }
    }
    let spec = DriverSpec {
        cwd: session_cwd,
        worktree: worktree.clone(),
        title: req.title.unwrap_or_default(),
        mock: state.mock,
        model: req.model,
        resume: req.resume,
        temper,
        push: state.push.clone(),
        apns: state.apns.clone(),
    };
    match spawn_session_driver(spec).await {
        Ok(handle) => {
            if let Some(guard) = worktree_guard {
                std::mem::forget(guard);
            }
            // Resuming is an explicit "bring this back" — an archived session (from the
            // past-sessions browser) should stop being hidden once it's live again, the same
            // way the archive button hid it.
            if is_resume {
                let _ = state.store.unarchive_session(&handle.session_id);
            }
            let handle = state.registry.insert(handle).await;
            json_response(&serde_json::json!({
                "id": handle.session_id,
                "title": handle.title,
                "cwd": handle.cwd,
                "worktree": handle.worktree,
            }))
        }
        Err(e) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("session create failed: {e}"),
        ),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkSessionReq {
    at_seq: i64,
}

async fn fork_session(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    axum::Json(req): axum::Json<ForkSessionReq>,
) -> Response {
    let store = state.store.clone();
    let source = id.clone();
    let store_for_fork = store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let cwd = store_for_fork
            .session_cwd(&source)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "no such session".to_string())?;
        let fork_id = store_for_fork
            .fork_session(&source, req.at_seq)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((fork_id, cwd))
    })
    .await;
    let (fork_id, cwd) = match result {
        Ok(Ok(value)) => value,
        Ok(Err(message)) if message == "no such session" => {
            return err_response(axum::http::StatusCode::NOT_FOUND, &message)
        }
        Ok(Err(message)) => return err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "could not create session fork",
            )
        }
    };
    let spec = DriverSpec {
        cwd,
        worktree: None,
        title: format!("Fork of {}", &id[..id.len().min(8)]),
        mock: state.mock,
        model: store
            .session_models(&id)
            .ok()
            .and_then(|models| models.last().cloned()),
        resume: Some(fork_id.clone()),
        temper: None,
        push: state.push.clone(),
        apns: state.apns.clone(),
    };
    match spawn_session_driver(spec).await {
        Ok(handle) => {
            let handle = state.registry.insert(handle).await;
            json_response(&serde_json::json!({
                "id": handle.session_id,
                "title": handle.title,
                "cwd": handle.cwd,
                "worktree": handle.worktree,
            }))
        }
        Err(error) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("fork start failed: {error}"),
        ),
    }
}

/// `POST /api/sessions/{id}/archive` — stop the driver, snapshot a worktree's uncommitted
/// edits onto its branch (never silently lose work), and hide the session from lists.
async fn archive_session(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(handle) = state.registry.remove(&id).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    handle.shutdown();
    handle.join(ARCHIVE_JOIN_TIMEOUT).await;
    if let Some(wt) = &handle.worktree {
        // Best-effort snapshot: uncommitted edits land on the session's branch so nothing is
        // lost; the worktree + branch stay in place for a manual review/merge.
        let _ = forge_core::worktree::commit_worktree(std::path::Path::new(wt));
    }
    let _ = state.store.archive_session(&id);
    json_response(&serde_json::json!({ "ok": true }))
}

#[derive(serde::Serialize)]
struct SessionTreeRow {
    id: String,
    title: Option<String>,
    forked_from: Option<String>,
    forked_at_seq: Option<i64>,
    created_at: i64,
}

async fn session_tree(State(state): State<Arc<DaemonState>>) -> Response {
    let store = state.store.clone();
    match tokio::task::spawn_blocking(move || store.fork_nodes()).await {
        Ok(Ok(nodes)) => json_response(
            &nodes
                .into_iter()
                .map(|node| SessionTreeRow {
                    id: node.id,
                    title: node.title,
                    forked_from: node.forked_from,
                    forked_at_seq: node.forked_at_seq,
                    created_at: node.created_at,
                })
                .collect::<Vec<_>>(),
        ),
        _ => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read session tree",
        ),
    }
}

/// One row of `GET /api/sessions/past` — a persisted session the user could resurrect. Distinct
/// from [`SessionRow`] (which describes a LIVE driver): these come straight from the store, so
/// there is no busy/waiting/context state to report, only the durable metadata.
#[derive(serde::Serialize)]
struct PastSessionRow {
    id: String,
    title: String,
    cwd: String,
    worktree: Option<String>,
    /// `true` if the session was explicitly archived (the archive button, not just orphaned by
    /// a daemon restart). The page marks these with an "archived" badge; they resume the same
    /// way as any other past session (`POST /api/sessions {resume:<id>}` un-archives them).
    archived: bool,
    message_count: i64,
    cost_usd: f64,
    last_activity: i64,
    created_at: i64,
    /// First user message — a one-line hint of what the session was about.
    preview: Option<String>,
}

/// Query for `GET /api/sessions/past` — cursor pagination over most-recently-used past sessions.
#[derive(serde::Deserialize)]
struct PastParams {
    /// Max rows to return (clamped 1..=200, default 50).
    limit: Option<usize>,
    /// Return only sessions with `last_activity` strictly before this unix-seconds cursor.
    before: Option<i64>,
}

/// `GET /api/sessions/past` — persisted top-level sessions that are NOT currently running, newest
/// activity first, so the page can browse and resurrect one (`POST /api/sessions {resume:<id>}`).
/// Uses [`forge_store::Store::list_sessions_for_resume`] (MRU-ordered, subagent rows excluded,
/// but — unlike `list_sessions` — archived rows INCLUDED and flagged) and simply drops the ids
/// the registry is actively driving.
async fn past_sessions(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<PastParams>,
) -> Response {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let before = params.before;
    // Ids the daemon is currently driving — these are shown by `/api/sessions`, not here.
    let running: std::collections::HashSet<String> = state
        .registry
        .all()
        .await
        .into_iter()
        .map(|h| h.session_id.clone())
        .collect();
    let store = state.store.clone();
    let rows: Vec<PastSessionRow> = tokio::task::spawn_blocking(move || {
        store
            .list_sessions_for_resume()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !running.contains(&s.id))
            .filter(|s| before.is_none_or(|b| s.last_activity < b))
            .take(limit)
            .map(|s| PastSessionRow {
                title: s.title.clone().unwrap_or_default(),
                cwd: s.cwd,
                worktree: s.worktree_path,
                archived: s.archived,
                message_count: s.message_count,
                cost_usd: s.total_cost_usd,
                last_activity: s.last_activity,
                created_at: s.created_at,
                preview: s.preview.map(|p| p.chars().take(140).collect()),
                id: s.id,
            })
            .collect()
    })
    .await
    .unwrap_or_default();
    json_response(&rows)
}

/// Derive the base repo root and branch for a daemon worktree session from its stored worktree
/// path. Worktrees are always created at `<repo_root>/.forge/worktrees/<child_id>` on branch
/// `forge/subagent/<child_id>` (see [`forge_core::worktree::WorktreeGuard::create`]), so both are
/// recoverable from the path alone — no extra state, and it matches exactly what the guard's own
/// removal uses. Returns `None` if the path is too shallow to be one of ours.
fn worktree_repo_and_branch(worktree: &str) -> Option<(std::path::PathBuf, String)> {
    let wt = std::path::Path::new(worktree);
    let child_id = wt.file_name()?.to_str()?.to_string();
    // `<repo_root>/.forge/worktrees/<child_id>` → strip three components back to `<repo_root>`.
    let repo_root = wt.parent()?.parent()?.parent()?.to_path_buf();
    Some((repo_root, format!("forge/subagent/{child_id}")))
}

/// Tracked, staged-or-modified files in the base repo (`--untracked-files=no` so registered
/// worktree dirs and other untracked cruft never count). A non-empty list means merging would
/// apply a patch on top of uncommitted work — the merge route refuses rather than do that
/// silently. Best-effort: a git failure returns empty (the 3-way apply is the real safety net).
fn base_dirty_tracked(repo_root: &std::path::Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args([
            "-C",
            repo_root.to_str().unwrap_or("."),
            "status",
            "--porcelain",
            "--untracked-files=no",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.get(3..).map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Remove a session's worktree directory and its branch. Uses `--force` on the worktree (the
/// snapshot commit already captured its edits) and `branch -D` on the branch. Returns any git
/// stderr so the caller can surface a partial failure. Callers MUST have stopped the driver
/// first (its cwd is inside the worktree).
fn remove_worktree(repo_root: &std::path::Path, worktree: &str, branch: &str) -> Vec<String> {
    let root = repo_root.to_str().unwrap_or(".");
    let mut errs = Vec::new();
    for args in [
        vec!["-C", root, "worktree", "remove", "--force", worktree],
        vec!["-C", root, "branch", "-D", branch],
    ] {
        match std::process::Command::new("git").args(&args).output() {
            Ok(o) if !o.status.success() => {
                errs.push(String::from_utf8_lossy(&o.stderr).trim().to_string());
            }
            Err(e) => errs.push(e.to_string()),
            _ => {}
        }
    }
    errs
}

/// Outcome of the blocking git merge sequence (snapshot → 3-way apply → conditional cleanup).
enum MergeOutcome {
    /// Applied cleanly; the worktree + branch were removed. Staged (uncommitted) in the base.
    Clean,
    /// Overlapping edits — nothing removed, files listed for a manual resolution.
    Conflicts(Vec<String>),
    /// A hard git error (not a conflict); state left intact.
    Error(String),
}

/// Run `git -C <root> <args>`, returning trimmed stdout on success.
fn git_stdout(root: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["-C", root.to_str().unwrap_or(".")];
    full.extend_from_slice(args);
    let out = std::process::Command::new("git")
        .args(&full)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Parse the conflicted-file list `git apply --3way` prints to stderr, tolerating both the modern
/// `Applied patch to 'FILE' with conflicts.` and older `Applied patch FILE with conflicts.`
/// wordings plus `error: patch failed: FILE:N`.
fn parse_apply_conflicts(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|line| {
            if let Some(rest) = line
                .strip_prefix("Applied patch ")
                .and_then(|r| r.strip_suffix(" with conflicts."))
            {
                let rest = rest.strip_prefix("to ").unwrap_or(rest);
                return Some(rest.trim_matches(['\'', '"']).to_string());
            }
            if let Some(rest) = line.strip_prefix("error: patch failed: ") {
                return rest.rsplit_once(':').map(|(p, _)| p.to_string());
            }
            None
        })
        .collect()
}

/// The blocking git merge sequence: snapshot the worktree's uncommitted edits onto its branch,
/// then 3-way-apply the branch's changes SINCE THE FORK POINT (`merge-base`, not HEAD — so a base
/// that advanced since the fork is 3-way-merged, never silently overwritten) onto the base tree.
/// On a clean apply the changes are staged and the worktree + branch removed. On conflict the base
/// tree is restored to its pre-merge state (`reset --hard`, safe because the caller guaranteed it
/// was clean) so nothing is left half-applied, and the worktree + branch are kept for manual work.
fn run_merge(repo_root: &std::path::Path, worktree: &str, branch: &str) -> MergeOutcome {
    if let Err(e) = forge_core::worktree::commit_worktree(std::path::Path::new(worktree)) {
        return MergeOutcome::Error(format!("snapshotting the worktree failed: {e}"));
    }
    let mergebase = match git_stdout(repo_root, &["merge-base", "HEAD", branch]) {
        Ok(b) => b,
        Err(e) => return MergeOutcome::Error(format!("finding the fork point failed: {e}")),
    };
    let diff = std::process::Command::new("git")
        .args([
            "-C",
            repo_root.to_str().unwrap_or("."),
            "diff",
            "--diff-filter=ACDMR",
            &mergebase,
            branch,
        ])
        .output();
    let patch = match diff {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => return MergeOutcome::Error(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => return MergeOutcome::Error(e.to_string()),
    };
    if patch.is_empty() {
        // The branch adds nothing over the fork point — a no-op merge, cleanly done.
        remove_worktree(repo_root, worktree, branch);
        return MergeOutcome::Clean;
    }
    let apply = std::process::Command::new("git")
        .args([
            "-C",
            repo_root.to_str().unwrap_or("."),
            "apply",
            "--3way",
            "--index",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&patch)?;
            }
            child.wait_with_output()
        });
    let apply = match apply {
        Ok(o) => o,
        Err(e) => return MergeOutcome::Error(e.to_string()),
    };
    if apply.status.success() {
        remove_worktree(repo_root, worktree, branch);
        return MergeOutcome::Clean;
    }
    // Conflict (or hard error): restore the base tree so it is left exactly as we found it —
    // `git apply --3way` writes conflict markers + a conflicted index otherwise. Safe because the
    // caller refused to start on a dirty base, so HEAD is the pre-merge state.
    let stderr = String::from_utf8_lossy(&apply.stderr).into_owned();
    let _ = std::process::Command::new("git")
        .args([
            "-C",
            repo_root.to_str().unwrap_or("."),
            "reset",
            "--hard",
            "HEAD",
        ])
        .output();
    let conflicts = parse_apply_conflicts(&stderr);
    if conflicts.is_empty() {
        MergeOutcome::Error(stderr.trim().to_string())
    } else {
        MergeOutcome::Conflicts(conflicts)
    }
}

/// Stop a driver and wait for it to wind down, so its worktree is quiescent before git touches it.
async fn stop_and_join(handle: Arc<SessionDriverHandle>) {
    handle.shutdown();
    handle.join(ARCHIVE_JOIN_TIMEOUT).await;
}

/// `POST /api/sessions/{id}/merge` — stop the session, snapshot its worktree onto the branch, and
/// merge that branch back into the base repo via a 3-way patch. Guards against data loss:
/// - refuses (409) if the base repo has uncommitted TRACKED changes — never a silent merge on top;
/// - snapshots the worktree first ([`forge_core::worktree::commit_worktree`]) so nothing is lost;
/// - on conflict, reports the files and leaves the worktree + branch intact (no auto-resolution).
///
/// The merged changes are STAGED (not committed) in the base tree for the user to review + commit.
async fn merge_session(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(handle) = state.registry.get(&id).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let Some(worktree) = handle.worktree.clone() else {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "session has no worktree to merge",
        );
    };
    let Some((repo_root, branch)) = worktree_repo_and_branch(&worktree) else {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "worktree path is not a recognised forge worktree",
        );
    };

    // Guard BEFORE stopping the session: a refused merge must leave it running untouched.
    let dirty = {
        let root = repo_root.clone();
        tokio::task::spawn_blocking(move || base_dirty_tracked(&root))
            .await
            .unwrap_or_default()
    };
    if !dirty.is_empty() {
        return (
            axum::http::StatusCode::CONFLICT,
            [
                (axum::http::header::CONTENT_TYPE, "application/json"),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            serde_json::json!({
                "error": "the base branch has uncommitted changes — commit or stash them, then merge",
                "dirty_files": dirty,
            })
            .to_string(),
        )
            .into_response();
    }

    // Atomically claim ownership: `remove` is the only serialization point, so a concurrent
    // merge/discard for the same id (a double-tapped button, two clients) gets `None` here and
    // bails — instead of two git sequences contending on the base repo index, which can
    // half-apply/duplicate a merge or `reset --hard` away another request's staged changes (base-
    // repo data loss). The dirty precondition above already ran on the pre-remove handle.
    let Some(handle) = state.registry.remove(&id).await else {
        return err_response(
            axum::http::StatusCode::CONFLICT,
            "session is already being merged or discarded",
        );
    };
    let resume_spec = DriverSpec {
        cwd: handle.cwd.clone(),
        worktree: handle.worktree.clone(),
        title: handle.title.clone(),
        mock: state.mock,
        model: state
            .store
            .session_models(&id)
            .ok()
            .and_then(|models| models.last().cloned()),
        resume: Some(id.clone()),
        temper: None,
        push: state.push.clone(),
        apns: state.apns.clone(),
    };
    stop_and_join(handle).await;
    let outcome = {
        let (root, br, wt) = (repo_root.clone(), branch.clone(), worktree.clone());
        tokio::task::spawn_blocking(move || run_merge(&root, &wt, &br))
            .await
            .unwrap_or_else(|e| MergeOutcome::Error(format!("merge task failed: {e}")))
    };
    if let MergeOutcome::Conflicts(files) = outcome {
        match spawn_session_driver(resume_spec).await {
            Ok(handle) => {
                state.registry.insert(handle).await;
                return (
                    axum::http::StatusCode::CONFLICT,
                    [
                        (axum::http::header::CONTENT_TYPE, "application/json"),
                        (axum::http::header::CACHE_CONTROL, "no-store"),
                    ],
                    serde_json::json!({
                        "error": "merge conflicts — resolve them by hand in the worktree; the session remains resumable",
                        "conflicts": files,
                        "branch": branch,
                        "worktree": worktree,
                    })
                    .to_string(),
                )
                    .into_response();
            }
            Err(e) => {
                return err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("merge conflicts, but session restart failed: {e}"),
                )
            }
        }
    }
    let _ = state.store.archive_session(&id);
    match outcome {
        MergeOutcome::Clean => json_response(&serde_json::json!({
            "ok": true, "merged": true, "branch": branch,
        })),
        MergeOutcome::Conflicts(_) => unreachable!("conflicts restore a resumable driver"),
        MergeOutcome::Error(msg) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("merge failed: {msg}"),
        ),
    }
}

/// `POST /api/sessions/{id}/discard` — stop the session and drop its worktree + branch WITHOUT
/// merging. This force-deletes the branch (unmerged commits and all), so the client MUST confirm
/// with the user first (the page's Discard button does); the request itself is that confirmation,
/// mirroring how the archive button gates its own stop.
async fn discard_session(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(handle) = state.registry.get(&id).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let Some(worktree) = handle.worktree.clone() else {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "session has no worktree to discard",
        );
    };
    let Some((repo_root, branch)) = worktree_repo_and_branch(&worktree) else {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "worktree path is not a recognised forge worktree",
        );
    };
    // Atomically claim ownership (see merge_session): a concurrent discard/merge for the same id
    // gets `None` and bails, so the worktree-remove + branch-delete git sequence never races a
    // merge's apply against the same base repo.
    let Some(handle) = state.registry.remove(&id).await else {
        return err_response(
            axum::http::StatusCode::CONFLICT,
            "session is already being merged or discarded",
        );
    };
    stop_and_join(handle).await;
    let _ = state.store.archive_session(&id);
    let errs = {
        let (root, br, wt) = (repo_root, branch.clone(), worktree);
        tokio::task::spawn_blocking(move || remove_worktree(&root, &wt, &br))
            .await
            .unwrap_or_default()
    };
    json_response(&serde_json::json!({
        "ok": true, "discarded": true, "branch": branch,
        "warnings": errs.into_iter().filter(|e| !e.is_empty()).collect::<Vec<_>>(),
    }))
}

/// Query for the per-session WS handshake.
#[derive(serde::Deserialize)]
struct WsParams {
    #[serde(default)]
    session: String,
    #[serde(default)]
    rev: u64,
}

/// Counts one attached WS client for the lifetime of its connection — the push debounce signal
/// (`crate::push::should_push`): any client attached ⇒ someone is watching ⇒ no push.
struct WsClientGuard(Arc<std::sync::atomic::AtomicUsize>);

impl WsClientGuard {
    fn attach(counter: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for WsClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn ws_handler(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> Response {
    match state.store.session_handoff_blocked(&params.session) {
        Ok(true) => {
            return err_response(
                axum::http::StatusCode::CONFLICT,
                "session is frozen by an Anywhere handoff",
            )
        }
        Err(error) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("handoff state check failed: {error}"),
            )
        }
        Ok(false) => {}
    }
    let Some(handle) = state.registry.get(&params.session).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let snapshot_rx = handle.snapshot_rx.clone();
    let events = handle.events.clone();
    let input_tx = handle.input_tx.clone();
    let clients = handle.ws_clients.clone();
    ws.on_upgrade(move |socket| async move {
        let _attached = WsClientGuard::attach(clients);
        remote::pump_ws(socket, snapshot_rx, events, input_tx, params.rev).await;
    })
}

async fn fleet_ws_handler(State(state): State<Arc<DaemonState>>, ws: WebSocketUpgrade) -> Response {
    let fleet_rx = state.registry.subscribe_fleet();
    ws.on_upgrade(move |socket| pump_fleet_ws(socket, fleet_rx))
}

async fn pump_fleet_ws(mut socket: WebSocket, mut fleet_rx: tokio::sync::watch::Receiver<u64>) {
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
            _ = keepalive.tick() => {
                if socket.send(WsMessage::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Web push + notification-action answers
// ---------------------------------------------------------------------------

/// `GET /api/push/key` — the VAPID public key the page hands to `PushManager.subscribe`.
async fn push_key(State(state): State<Arc<DaemonState>>) -> Response {
    match &state.push {
        Some(p) => json_response(&serde_json::json!({ "key": p.public_key_b64url() })),
        None => err_response(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "web push is unavailable (no VAPID key)",
        ),
    }
}

/// Body of `POST /api/push/subscribe` / `unsubscribe` — three shapes on the same two routes,
/// discriminated structurally (untagged) by which fields are present:
/// - the browser's `PushSubscription.toJSON()` (Web Push) — `{endpoint, keys}`. No explicit
///   `kind` marker, since existing web clients already POST exactly this shape.
/// - a native device token (APNs) — `{device_token, environment}`.
/// - a Live Activity's per-activity push token — `{session_id, push_token, environment}`.
///
/// Untagged deserialization tries each variant in order and picks the first whose required
/// fields are all present, so field NAMES (not a `kind` tag) are the discriminator — every
/// variant here has a field name no other variant shares.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum SubscribeReq {
    LiveActivity {
        session_id: String,
        push_token: String,
        #[serde(default)]
        environment: String,
    },
    Apns {
        device_token: String,
        #[serde(default)]
        environment: String,
    },
    Webpush {
        endpoint: String,
        #[serde(default)]
        keys: SubscribeKeys,
    },
}

#[derive(serde::Deserialize, Default)]
struct SubscribeKeys {
    #[serde(default)]
    p256dh: String,
    #[serde(default)]
    auth: String,
}

/// Validate a Web Push subscription before storing it: a well-formed push endpoint URL plus
/// decodable RFC 8291 keys of exactly the right shape (65-byte uncompressed P-256 point, 16-byte
/// auth). Garbage is rejected at the door, not discovered at send time.
fn validate_subscription(endpoint: &str, keys: &SubscribeKeys) -> Result<(), &'static str> {
    if endpoint.len() > 2048 {
        return Err("endpoint too long");
    }
    if crate::push::endpoint_origin(endpoint).is_none() {
        return Err("endpoint is not an http(s) URL");
    }
    match crate::push::b64url_decode(&keys.p256dh) {
        Some(k) if k.len() == 65 && k[0] == 0x04 => {}
        _ => return Err("keys.p256dh must be a base64url 65-byte uncompressed P-256 point"),
    }
    match crate::push::b64url_decode(&keys.auth) {
        Some(a) if a.len() == 16 => {}
        _ => return Err("keys.auth must be a base64url 16-byte secret"),
    }
    Ok(())
}

/// A stored APNs environment is either `"production"` or `"sandbox"` — anything else (including
/// an omitted/empty field) safely falls back to sandbox, mirroring `ApnsNotifier::host`'s own
/// fallback: a misrouted sandbox token merely fails rather than reaching the wrong audience.
fn normalize_apns_environment(environment: &str) -> &'static str {
    if environment == "production" {
        "production"
    } else {
        "sandbox"
    }
}

/// `POST /api/push/subscribe` — store (deduped) so a notification reaches this
/// browser/device/Live Activity.
async fn push_subscribe(
    State(state): State<Arc<DaemonState>>,
    axum::Json(req): axum::Json<SubscribeReq>,
) -> Response {
    match req {
        SubscribeReq::LiveActivity {
            session_id,
            push_token,
            environment,
        } => {
            if state.apns.is_none() {
                return err_response(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "native push is unavailable (no APNs key configured)",
                );
            }
            if session_id.is_empty() {
                return err_response(
                    axum::http::StatusCode::BAD_REQUEST,
                    "session_id must be non-empty",
                );
            }
            if !crate::apns::is_valid_token(&push_token) {
                return err_response(
                    axum::http::StatusCode::BAD_REQUEST,
                    "push_token must be 64 lowercase hexadecimal characters",
                );
            }
            let environment = normalize_apns_environment(&environment).to_string();
            let store = state.store.clone();
            let stored = tokio::task::spawn_blocking(move || {
                store.upsert_live_activity_token(&session_id, &push_token, &environment)
            })
            .await;
            match stored {
                Ok(Ok(())) => json_response(&serde_json::json!({ "ok": true })),
                _ => err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "storing the live activity token failed",
                ),
            }
        }
        SubscribeReq::Apns {
            device_token,
            environment,
        } => {
            if state.apns.is_none() {
                return err_response(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "native push is unavailable (no APNs key configured)",
                );
            }
            if !crate::apns::is_valid_token(&device_token) {
                return err_response(
                    axum::http::StatusCode::BAD_REQUEST,
                    "device_token must be 64 lowercase hexadecimal characters",
                );
            }
            let environment = normalize_apns_environment(&environment).to_string();
            let store = state.store.clone();
            let stored = tokio::task::spawn_blocking(move || {
                store.upsert_apns_subscription(&device_token, &environment)
            })
            .await;
            match stored {
                Ok(Ok(_)) => json_response(&serde_json::json!({ "ok": true })),
                _ => err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "storing the subscription failed",
                ),
            }
        }
        SubscribeReq::Webpush { endpoint, keys } => {
            if state.push.is_none() {
                return err_response(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "web push is unavailable (no VAPID key)",
                );
            }
            if let Err(msg) = validate_subscription(&endpoint, &keys) {
                return err_response(axum::http::StatusCode::BAD_REQUEST, msg);
            }
            let store = state.store.clone();
            let stored = tokio::task::spawn_blocking(move || {
                store.upsert_push_subscription(&endpoint, &keys.p256dh, &keys.auth)
            })
            .await;
            match stored {
                Ok(Ok(_)) => json_response(&serde_json::json!({ "ok": true })),
                _ => err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "storing the subscription failed",
                ),
            }
        }
    }
}

/// `POST /api/push/unsubscribe` — forget a subscription (by endpoint/device token/session id).
async fn push_unsubscribe(
    State(state): State<Arc<DaemonState>>,
    axum::Json(req): axum::Json<SubscribeReq>,
) -> Response {
    let store = state.store.clone();
    match req {
        // `push_token` isn't needed to delete (session_id is the key) — it's only required by
        // the shared request shape's discriminator.
        SubscribeReq::LiveActivity { session_id, .. } => {
            let _ =
                tokio::task::spawn_blocking(move || store.delete_live_activity_token(&session_id))
                    .await;
        }
        SubscribeReq::Apns { device_token, .. } => {
            let _ =
                tokio::task::spawn_blocking(move || store.delete_apns_subscription(&device_token))
                    .await;
        }
        SubscribeReq::Webpush { endpoint, .. } => {
            let _ = tokio::task::spawn_blocking(move || store.delete_push_subscription(&endpoint))
                .await;
        }
    }
    json_response(&serde_json::json!({ "ok": true }))
}

/// Body of `POST /api/answer` — a notification action resolving a permission prompt.
#[derive(serde::Deserialize)]
struct AnswerReq {
    session: String,
    seq: u64,
    allow: bool,
}

/// `POST /api/answer` — approve/deny a pending permission prompt WITHOUT a page: the service
/// worker calls this straight from the notification's Allow/Deny buttons. `seq` must echo the
/// prompt's [`remote::Snapshot::prompt_seq`] exactly like the WS path — a stale/unknown seq is
/// a 409 no-op, and the driver re-validates on receipt (so a prompt swapped between our check
/// and the queue drain still can't be mis-approved).
async fn answer(
    State(state): State<Arc<DaemonState>>,
    axum::Json(req): axum::Json<AnswerReq>,
) -> Response {
    let Some(handle) = state.registry.get(&req.session).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let snap = handle.snapshot_rx.borrow().snapshot.clone();
    if snap.permission_prompt.is_none() {
        return err_response(
            axum::http::StatusCode::CONFLICT,
            "no permission prompt is pending",
        );
    }
    if !remote::prompt_seq_current(snap.prompt_seq, req.seq) {
        return err_response(
            axum::http::StatusCode::CONFLICT,
            "stale answer ignored — the prompt changed; review the current one",
        );
    }
    if handle
        .input_tx
        .send(remote::RemoteInput::Allow {
            yes: req.allow,
            seq: req.seq,
        })
        .await
        .is_err()
    {
        return err_response(axum::http::StatusCode::CONFLICT, "session is shutting down");
    }
    json_response(&serde_json::json!({ "ok": true }))
}

/// Query for `POST /api/upload` — which session the files belong to.
#[derive(serde::Deserialize)]
struct UploadParams {
    #[serde(default)]
    session: String,
}

/// `POST /api/upload?session=<id>` — multipart file/image upload (v7). Files are stored under
/// the session's own scratch area (`<session cwd>/.forge/uploads/<id>/`, names sanitized —
/// see [`remote::store_upload`]) and delivered to its driver as [`remote::RemoteInput::Attach`]:
/// images become vision input on the next turn, text files an `@path` mention.
async fn upload(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<UploadParams>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    let Some(handle) = state.registry.get(&params.session).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let dir = std::path::Path::new(&handle.cwd)
        .join(".forge")
        .join("uploads")
        .join(remote::sanitize_upload_name(&handle.session_id));
    let mut stored: Vec<serde_json::Value> = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                return err_response(
                    axum::http::StatusCode::BAD_REQUEST,
                    &format!("malformed multipart body: {e}"),
                );
            }
        };
        let name = field.file_name().unwrap_or("upload").to_string();
        let content_type = field.content_type().map(str::to_string);
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => {
                // axum surfaces the body-limit overflow here.
                return err_response(
                    axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                    &format!("upload failed: {e}"),
                );
            }
        };
        match remote::store_upload(&dir, &name, content_type.as_deref(), &bytes) {
            Ok((path, image)) => {
                let path_str = path.display().to_string();
                if handle
                    .input_tx
                    .send(remote::RemoteInput::Attach {
                        path: path_str.clone(),
                        image,
                    })
                    .await
                    .is_err()
                {
                    return err_response(
                        axum::http::StatusCode::CONFLICT,
                        "session is shutting down",
                    );
                }
                stored.push(serde_json::json!({
                    "name": remote::sanitize_upload_name(&name),
                    "path": path_str,
                    "image": image,
                }));
            }
            Err(msg) => {
                return err_response(axum::http::StatusCode::UNPROCESSABLE_ENTITY, &msg);
            }
        }
    }
    if stored.is_empty() {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "no files in the body");
    }
    json_response(&serde_json::json!({ "files": stored }))
}

/// Query for `POST /api/voice/transcribe` — an optional language override for this one clip.
#[derive(serde::Deserialize)]
struct VoiceTranscribeParams {
    language: Option<String>,
}

/// `POST /api/voice/transcribe?language=<code>` — multipart audio in (first field with bytes,
/// any name), `{"text": "..."}` out. Decodes wav/m4a/aac/mp4, downloads the configured whisper
/// model on first use, and transcribes locally (voice.md, V1). Not session-scoped: the model
/// cache lives on `DaemonState`, not any one session.
async fn voice_transcribe(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<VoiceTranscribeParams>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    let field = match multipart.next_field().await {
        Ok(Some(f)) => f,
        Ok(None) => {
            return err_response(axum::http::StatusCode::BAD_REQUEST, "no audio in the body")
        }
        Err(e) => {
            return err_response(
                axum::http::StatusCode::BAD_REQUEST,
                &format!("malformed multipart body: {e}"),
            );
        }
    };
    let hint = field
        .file_name()
        .map(str::to_string)
        .or_else(|| field.content_type().map(str::to_string));
    let bytes = match field.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            return err_response(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                &format!("upload failed: {e}"),
            );
        }
    };

    let models_dir = match crate::voice::models_dir() {
        Ok(d) => d,
        Err(e) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("{e}"),
            )
        }
    };
    let config = forge_config::load().unwrap_or_default();
    match crate::voice::transcribe_upload(
        &state.voice,
        &config.voice,
        &models_dir,
        bytes,
        hint,
        params.language,
    )
    .await
    {
        Ok(text) => json_response(&serde_json::json!({ "text": text })),
        Err(e) => err_response(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            &format!("{e}"),
        ),
    }
}

/// Query for `GET /api/upload` — which session's upload area to serve from, and which stored
/// file inside it.
#[derive(serde::Deserialize)]
struct ServeUploadParams {
    #[serde(default)]
    session: String,
    #[serde(default)]
    path: String,
}

/// `GET /api/upload?session=<id>&path=<stored-path>` — stream back the raw bytes of a file
/// previously stored by `POST /api/upload` (the `path` in its response, and the same string
/// mirrored into a persisted `@path` mention). `POST /api/upload` only ever accepted bytes; there
/// was previously no way to retrieve them, so a client re-rendering history after a reload (new
/// device, app restart) had no way to show a historical image — it was gone the moment the live
/// optimistic bubble scrolled away.
///
/// Reads the session's cwd straight from the store — like [`history_page`], NOT from the
/// in-memory `SessionRegistry` — so a historical image still renders even after the session's
/// driver has wound down or the daemon restarted. `path` is confined to that session's own
/// `.forge/uploads/` directory with the exact same canonicalize + `starts_with` check used in
/// `handle_remote_attach`/`store_upload`'s callers; anything outside it (or an unknown session,
/// or a missing file) is a 403/404 — never a 500.
async fn serve_upload(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<ServeUploadParams>,
) -> Response {
    if params.session.is_empty() || params.path.is_empty() {
        return err_response(axum::http::StatusCode::NOT_FOUND, "missing session or path");
    }
    let store = state.store.clone();
    let session = params.session.clone();
    let cwd = match tokio::task::spawn_blocking(move || store.session_cwd(&session))
        .await
        .unwrap_or(Ok(None))
    {
        Ok(Some(cwd)) => cwd,
        _ => return err_response(axum::http::StatusCode::NOT_FOUND, "no such session"),
    };
    let root = std::path::Path::new(&cwd).join(".forge").join("uploads");
    let confined = std::fs::canonicalize(&params.path)
        .ok()
        .zip(std::fs::canonicalize(&root).ok())
        .map(|(p, r)| p.starts_with(&r))
        .unwrap_or(false);
    if !confined {
        return err_response(
            axum::http::StatusCode::FORBIDDEN,
            "path is outside this session's uploads",
        );
    }
    let bytes = match tokio::fs::read(&params.path).await {
        Ok(b) => b,
        Err(_) => return err_response(axum::http::StatusCode::NOT_FOUND, "file not found"),
    };
    (
        [
            (
                axum::http::header::CONTENT_TYPE,
                guess_content_type(&params.path),
            ),
            (
                axum::http::header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response()
}

/// Content-Type by file extension — the uploads store only ever accepts images and UTF-8 text
/// ([`remote::store_upload`]), so this only needs to cover those; anything else falls back to a
/// generic binary type rather than guessing wrong.
fn guess_content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "md" => "text/plain; charset=utf-8",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

#[derive(serde::Deserialize)]
struct UsageParams {
    session: Option<String>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageTotals {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageProvider {
    provider: String,
    kind: String,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageQuota {
    provider: String,
    kind: String,
    window_kind: String,
    status: String,
    resets_at: Option<i64>,
    fraction: Option<f64>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageWindow {
    since_epoch: i64,
    combined: UsageTotals,
    providers: Vec<UsageProvider>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionUsage {
    session_id: String,
    combined: UsageTotals,
    providers: Vec<UsageProvider>,
}
#[derive(serde::Serialize)]
struct UsageResponse {
    week: UsageWindow,
    session: Option<SessionUsage>,
    quota: Vec<UsageQuota>,
}

fn provider_kind(provider: &str) -> &'static str {
    if provider.ends_with("-cli") {
        "bridge"
    } else if provider.ends_with("-oauth") || provider == "gemini" {
        "oauth"
    } else {
        "api"
    }
}
fn usage_providers(rows: Vec<forge_store::ProviderUsage>) -> (UsageTotals, Vec<UsageProvider>) {
    let total = rows.iter().fold(
        UsageTotals {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        },
        |mut t, r| {
            t.input_tokens += r.input_tokens;
            t.output_tokens += r.output_tokens;
            t.cost_usd += r.cost_usd;
            t
        },
    );
    let providers = rows
        .into_iter()
        .map(|r| UsageProvider {
            kind: provider_kind(&r.provider).into(),
            provider: r.provider,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cost_usd: r.cost_usd,
        })
        .collect();
    (total, providers)
}
#[derive(serde::Serialize)]
struct SkillRow {
    name: String,
    description: String,
    scope: String,
    tier: Option<String>,
    resources: usize,
}

async fn skills_page() -> Response {
    let rows = tokio::task::spawn_blocking(|| {
        let catalog = forge_skills::Catalog::load(&forge_config::command_sources());
        let mut skills: Vec<SkillRow> = catalog
            .all_skills()
            .into_iter()
            .map(|skill| SkillRow {
                name: skill.name.clone(),
                description: skill.description.clone(),
                scope: skill.scope.label().to_string(),
                tier: skill
                    .tier
                    .map(|tier| format!("{tier:?}").to_ascii_lowercase()),
                resources: skill.resources.len(),
            })
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    })
    .await;
    match rows {
        Ok(rows) => json_response(&rows),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read skills catalog",
        ),
    }
}

#[derive(serde::Deserialize)]
struct WorkflowsParams {
    #[serde(default)]
    session: String,
}

#[derive(serde::Serialize)]
struct WorkflowRow {
    name: String,
    description: String,
    when_to_use: Option<String>,
    phases: Vec<String>,
}

/// Pull a string field (`description: '…'` / `"…"`) out of a workflow script's `meta` literal.
/// The meta block is a pure literal by contract (forge-core's authoring guidance), so a
/// lightweight scan is faithful enough for a library listing — no JS engine needed.
fn meta_string_field(meta: &str, field: &str) -> Option<String> {
    let idx = meta.find(&format!("{field}:"))?;
    let rest = &meta[idx + field.len() + 1..];
    let rest = rest.trim_start();
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' && quote != '`' {
        return None;
    }
    let body = &rest[1..];
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
        } else if c == quote {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

/// Extract the `export const meta = {…}` literal (balanced braces, quote-aware).
fn meta_literal(script: &str) -> Option<&str> {
    let start = script.find("export const meta")?;
    let open = script[start..].find('{')? + start;
    let mut depth = 0usize;
    let mut in_str: Option<char> = None;
    let mut prev_backslash = false;
    for (i, c) in script[open..].char_indices() {
        if let Some(q) = in_str {
            if prev_backslash {
                prev_backslash = false;
            } else if c == '\\' {
                prev_backslash = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => in_str = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&script[open..=open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Phase titles from the meta literal's `phases: [{ title: '…' }, …]` array, in order.
fn meta_phase_titles(meta: &str) -> Vec<String> {
    let Some(idx) = meta.find("phases:") else {
        return Vec::new();
    };
    let Some(tail) = meta.get(idx..) else {
        return Vec::new();
    };
    let Some(end) = tail.find(']') else {
        return Vec::new();
    };
    let mut titles = Vec::new();
    let mut rest = &tail[..end];
    while let Some(t) = meta_string_field(rest, "title") {
        let consumed = rest.find("title:").unwrap_or(0) + "title:".len() + t.len() + 2;
        titles.push(t);
        rest = rest.get(consumed..).unwrap_or("");
    }
    titles
}

/// `GET /api/workflows?session=<id>` — the saved-workflow library for the session's project
/// (`.forge/workflows/*.js`), with `meta` description/whenToUse/phases parsed out for the
/// app's library screen. Falls back to the daemon's default cwd when no session is given.
async fn workflows_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<WorkflowsParams>,
) -> Response {
    let cwd = if params.session.is_empty() {
        state.default_cwd.clone()
    } else {
        match state.registry.get(&params.session).await {
            Some(handle) => handle.cwd.clone(),
            None => state.default_cwd.clone(),
        }
    };
    let rows = tokio::task::spawn_blocking(move || {
        let dir = std::path::Path::new(&cwd).join(".forge").join("workflows");
        let mut rows: Vec<WorkflowRow> = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return rows;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("js") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let script = std::fs::read_to_string(&path).unwrap_or_default();
            let meta = meta_literal(&script).unwrap_or("");
            rows.push(WorkflowRow {
                name: name.to_string(),
                description: meta_string_field(meta, "description").unwrap_or_default(),
                when_to_use: meta_string_field(meta, "whenToUse"),
                phases: meta_phase_titles(meta),
            });
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows
    })
    .await;
    match rows {
        Ok(rows) => json_response(&rows),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read workflows",
        ),
    }
}

#[derive(serde::Serialize)]
struct ModelsResponse {
    catalog: &'static str,
    providers: Vec<ModelProvider>,
}

#[derive(serde::Serialize)]
struct ModelProvider {
    provider: String,
    models: Vec<ModelRow>,
}

#[derive(serde::Serialize)]
struct ModelRow {
    id: String,
    name: String,
    frontier: bool,
    free: bool,
    paid: bool,
    subscription: bool,
    estimated_cost_usd: f64,
    health: Option<ModelHealth>,
    tier: &'static str,
    benchmark_intelligence: Option<f64>,
    benchmark_coding: Option<f64>,
    context_window: Option<u32>,
}

#[derive(serde::Serialize, Clone)]
struct ModelHealth {
    until_epoch: i64,
    reason: String,
}

async fn models_page(State(state): State<Arc<DaemonState>>) -> Response {
    let store = state.store.clone();
    match tokio::task::spawn_blocking(move || {
        let Some(catalog) = crate::cli::commands::models::load_cached_catalog() else {
            return ModelsResponse {
                catalog: "unavailable",
                providers: Vec::new(),
            };
        };
        let config = forge_config::load().unwrap_or_default();
        let pricing = forge_mesh::pricing::Pricing::from_config(&config);
        let benches: std::collections::HashMap<_, _> = store
            .current_benched_report()
            .unwrap_or_default()
            .into_iter()
            .map(|(model, until_epoch, reason)| {
                (
                    model,
                    ModelHealth {
                        until_epoch,
                        reason,
                    },
                )
            })
            .collect();
        let context_windows = store.all_model_contexts().unwrap_or_default();
        ModelsResponse {
            catalog: "available",
            providers: catalog
                .by_provider(&pricing)
                .into_iter()
                .map(|provider| ModelProvider {
                    provider: provider.provider,
                    models: provider
                        .models
                        .into_iter()
                        .map(|model| ModelRow {
                            health: benches.get(&model.id).cloned(),
                            id: model.id.clone(),
                            name: model.name,
                            frontier: model.frontier,
                            free: model.free,
                            paid: model.paid,
                            subscription: model.subscription,
                            estimated_cost_usd: model.cost,
                            tier: if model.frontier {
                                "complex"
                            } else if model.subscription || model.paid {
                                "standard"
                            } else {
                                "trivial"
                            },
                            benchmark_intelligence: catalog
                                .benchmark_for(&model.id)
                                .map(|score| score.0),
                            benchmark_coding: catalog.benchmark_for(&model.id).map(|score| score.1),
                            context_window: context_windows
                                .get(&model.id)
                                .copied()
                                .or_else(|| forge_mesh::pricing::context_limit(&model.id)),
                        })
                        .collect(),
                })
                .collect(),
        }
    })
    .await
    {
        Ok(response) => json_response(&response),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read model catalog",
        ),
    }
}

async fn config_page() -> Response {
    match tokio::task::spawn_blocking(config_response).await {
        Ok(response) => json_response(&response),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read configuration",
        ),
    }
}

async fn update_config(Json(request): Json<UpdateConfigRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let descriptors = forge_config::config_descriptors();
        if !descriptors
            .iter()
            .any(|descriptor| descriptor.path == request.key)
        {
            return Err("unknown configuration field".to_string());
        }
        let scope = request.scope.into();
        match request.value {
            Some(value) => forge_config::set_config_value(scope, &request.key, &value),
            None => forge_config::reset_config_value(scope, &request.key),
        }
        .map_err(|error| error.to_string())?;
        Ok(config_response())
    })
    .await;

    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(error)) => err_response(axum::http::StatusCode::BAD_REQUEST, &error),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not update configuration",
        ),
    }
}

fn config_response() -> ConfigResponse {
    ConfigResponse {
        fields: forge_config::config_descriptors()
            .into_iter()
            .map(|descriptor| {
                let (field_type, options) = match descriptor.kind {
                    forge_config::SettingKind::Bool => ("bool", Vec::new()),
                    forge_config::SettingKind::Int => ("int", Vec::new()),
                    forge_config::SettingKind::Float => ("float", Vec::new()),
                    forge_config::SettingKind::List => ("list", Vec::new()),
                    forge_config::SettingKind::Json => ("json", Vec::new()),
                    forge_config::SettingKind::Enum(options) => {
                        ("enum", options.into_iter().map(str::to_string).collect())
                    }
                    forge_config::SettingKind::Text => ("text", Vec::new()),
                };
                ConfigField {
                    key: descriptor.path,
                    group: descriptor.group,
                    field_type: field_type.to_string(),
                    label: descriptor.label,
                    help: descriptor.help,
                    options,
                    value: descriptor.value.display(),
                    default: descriptor.default.display(),
                    modified: descriptor.modified,
                    source: descriptor.source.to_string(),
                }
            })
            .collect(),
    }
}

#[derive(serde::Serialize)]
struct HookRow {
    event: String,
    matcher: Option<String>,
    command: String,
    timeout_secs: u64,
    cc_compat: bool,
}
async fn hooks_page() -> Response {
    match tokio::task::spawn_blocking(|| {
        forge_config::load()
            .unwrap_or_default()
            .hooks
            .into_iter()
            .map(|hook| HookRow {
                event: hook.event.cc_name().to_string(),
                matcher: hook.matcher,
                command: hook.command,
                timeout_secs: hook.timeout_secs,
                cc_compat: hook.cc_compat,
            })
            .collect::<Vec<_>>()
    })
    .await
    {
        Ok(hooks) => json_response(&hooks),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read hooks",
        ),
    }
}

#[derive(serde::Serialize)]
struct PlanRow {
    session_id: String,
    session_title: String,
    title: String,
    steps: Vec<remote::SnapPlanStep>,
    notes: Option<String>,
}
async fn plans_page(State(state): State<Arc<DaemonState>>) -> Response {
    let mut plans = Vec::new();
    for handle in state.registry.all().await {
        let snapshot = handle.snapshot_rx.borrow().snapshot.clone();
        if let Some(plan) = snapshot.plan {
            plans.push(PlanRow {
                session_id: handle.session_id.clone(),
                session_title: handle.title.clone(),
                title: plan.title,
                steps: plan.steps,
                notes: plan.notes,
            });
        }
    }
    json_response(&plans)
}

#[derive(serde::Serialize)]
struct McpServerRow {
    name: String,
    transport: String,
    enabled: bool,
    auth_configured: bool,
    secret_env_count: usize,
}

#[derive(serde::Serialize)]
struct McpResponse {
    servers: Vec<McpServerRow>,
    allowed_servers: Vec<String>,
    allowed_tools: Vec<String>,
    call_timeout_secs: u64,
    connect_timeout_secs: u64,
}

async fn create_mcp_server(Json(request): Json<CreateMcpServerRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let name = request.name.trim();
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '-' || character == '_'
            })
        {
            return Err(
                "server name must use only letters, numbers, hyphens, or underscores".to_string(),
            );
        }
        let path = std::path::PathBuf::from(".forge/mcp.toml");
        let mut config = forge_config::load_mcp_toml(&path);
        if config.servers.iter().any(|server| server.name == name) {
            return Err("a server with that name already exists".to_string());
        }
        let transport = match request.transport {
            McpTransportRequest::Stdio => {
                let command = request
                    .command
                    .filter(|command| !command.trim().is_empty())
                    .ok_or_else(|| "stdio servers need a command".to_string())?;
                forge_config::McpTransport::Stdio {
                    command,
                    args: request.args,
                    env: std::collections::HashMap::new(),
                }
            }
            McpTransportRequest::Http => forge_config::McpTransport::Http {
                url: valid_mcp_url(request.url)?,
                headers: std::collections::HashMap::new(),
            },
            McpTransportRequest::Sse => forge_config::McpTransport::Sse {
                url: valid_mcp_url(request.url)?,
                headers: std::collections::HashMap::new(),
            },
        };
        let auth = request
            .token_env
            .filter(|name| !name.trim().is_empty())
            .map(|token_env| forge_config::McpAuth {
                token_env: Some(token_env),
                token_keyring: None,
                header: None,
                oauth: None,
            });
        config.servers.push(forge_config::McpServerConfig {
            name: name.to_string(),
            transport,
            auth,
            secret_env: Vec::new(),
            enabled: true,
        });
        forge_config::write_mcp_toml(&path, &config).map_err(|error| error.to_string())
    })
    .await;
    match result {
        Ok(Ok(())) => mcp_page().await,
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not save MCP server",
        ),
    }
}

fn valid_mcp_url(url: Option<String>) -> Result<String, String> {
    let url = url.ok_or_else(|| "HTTP and SSE servers need an http(s) URL".to_string())?;
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url)
    } else {
        Err("HTTP and SSE servers need an http(s) URL".to_string())
    }
}

async fn mcp_page() -> Response {
    match tokio::task::spawn_blocking(|| {
        let config = forge_config::load().unwrap_or_default();
        McpResponse {
            servers: config
                .mcp
                .servers
                .iter()
                .map(|server| McpServerRow {
                    name: server.name.clone(),
                    transport: server.transport_label().to_string(),
                    enabled: server.enabled,
                    auth_configured: server.auth.is_some(),
                    secret_env_count: server.secret_env.len(),
                })
                .collect(),
            allowed_servers: config.mcp.allow.servers,
            allowed_tools: config.mcp.allow.tools,
            call_timeout_secs: config.mcp.call_timeout_secs,
            connect_timeout_secs: config.mcp.connect_timeout_secs,
        }
    })
    .await
    {
        Ok(response) => json_response(&response),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read MCP configuration",
        ),
    }
}

async fn usage_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<UsageParams>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let now = chrono::Utc::now().timestamp();
        let week_rows = store
            .usage_by_provider_since(now - 604800)
            .unwrap_or_default();
        let (combined, providers) = usage_providers(week_rows);
        let session = params.session.filter(|s| !s.is_empty()).map(|id| {
            let (combined, providers) =
                usage_providers(store.usage_by_provider_for_session(&id).unwrap_or_default());
            SessionUsage {
                session_id: id,
                combined,
                providers,
            }
        });
        let quota = store
            .subscription_windows()
            .unwrap_or_default()
            .into_iter()
            .map(|q| UsageQuota {
                kind: provider_kind(&q.provider).into(),
                provider: q.provider,
                window_kind: q.window_kind,
                status: q.status,
                resets_at: q.resets_at,
                fraction: q.fraction,
            })
            .collect();
        UsageResponse {
            week: UsageWindow {
                since_epoch: now - 604800,
                combined,
                providers,
            },
            session,
            quota,
        }
    })
    .await;
    match result {
        Ok(body) => json_response(&body),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "usage unavailable",
        ),
    }
}

#[derive(serde::Deserialize)]
struct HistoryParams {
    #[serde(default)]
    session: String,
    before: Option<i64>,
    limit: Option<usize>,
}

async fn history_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HistoryParams>,
) -> Response {
    let limit = remote::history_page_limit(params.limit);
    let sid = params.session;
    let before = params.before;
    let store = state.store.clone();
    let rows: Vec<remote::HistoryRow> = if sid.is_empty() {
        Vec::new()
    } else {
        tokio::task::spawn_blocking(move || {
            store
                .load_history_page(&sid, before, limit)
                .unwrap_or_default()
                .into_iter()
                .map(|r| remote::HistoryRow {
                    seq: r.seq,
                    role: r.role.as_str().to_string(),
                    content: r.content,
                    model: r.model,
                    created_at: r.created_at,
                    visibility: r.visibility.as_str().to_string(),
                })
                .collect()
        })
        .await
        .unwrap_or_default()
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
    )
        .into_response()
}

fn json_response<T: serde::Serialize>(value: &T) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_string(value).unwrap_or_else(|_| "{}".into()),
    )
        .into_response()
}

fn err_response(status: axum::http::StatusCode, msg: &str) -> Response {
    (
        status,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::json!({ "error": msg }).to_string(),
    )
        .into_response()
}

/// Parse `CreateSessionReq::temper` — case-insensitive, the same names/aliases
/// [`forge_types::PermissionMode::from_label`] accepts for the `/mode` picker. On failure,
/// returns the `400` body text listing every valid name (never silently defaults).
fn parse_temper(raw: &str) -> Result<forge_types::PermissionMode, String> {
    forge_types::PermissionMode::from_label(raw).ok_or_else(|| {
        let valid = forge_types::PermissionMode::all()
            .iter()
            .map(|m| m.label())
            .collect::<Vec<_>>()
            .join(", ");
        format!("temper: unknown value {raw:?} — valid names: {valid}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_attention_push_fires_only_on_waiting_transition() {
        assert!(attention_became_required(false, true));
        assert!(!attention_became_required(true, true));
        assert!(!attention_became_required(true, false));
        assert!(!attention_became_required(false, false));
    }

    #[test]
    fn running_pre_enable_daemon_receives_connector_activation() {
        let (enabled, receiver) = tokio::sync::watch::channel(false);
        signal_anywhere_enable(&enabled);
        assert!(*receiver.borrow());
        // Repeated enable requests attach to the same one-shot supervisor.
        signal_anywhere_enable(&enabled);
        assert!(*receiver.borrow());
    }

    #[test]
    fn project_roots_are_canonical_deduplicated_and_default_first() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let extra = temp.path().join("extra");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&extra).unwrap();

        let roots = resolve_project_roots(
            &default,
            &[
                default.join(".").display().to_string(),
                extra.display().to_string(),
                extra.join("..").join("extra").display().to_string(),
            ],
        );

        assert_eq!(
            roots,
            vec![
                default.canonicalize().unwrap(),
                extra.canonicalize().unwrap()
            ]
        );
    }

    #[test]
    fn workflow_meta_parsers_extract_library_fields() {
        // A realistic authored workflow: `export const meta = {…}` with a brace and a
        // matching quote inside a string value (must not break literal balancing or field
        // extraction), plus an escaped apostrophe in whenToUse.
        let script = r#"
export const meta = {
  name: 'code-review',
  description: 'Review a diff { and } braces',
  whenToUse: 'When you\'re unsure it is right',
  phases: [
    { title: 'Scan', prompt: 'p1' },
    { title: 'Verify', prompt: 'p2' },
    { title: 'Report', prompt: 'p3' },
  ],
};

export async function run() {}
"#;
        let meta = meta_literal(script).expect("meta literal");
        assert!(meta.starts_with('{') && meta.ends_with('}'));
        assert!(meta.contains("phases:"));
        assert!(!meta.contains("export async function"));

        assert_eq!(
            meta_string_field(meta, "description").as_deref(),
            Some("Review a diff { and } braces")
        );
        assert_eq!(
            meta_string_field(meta, "whenToUse").as_deref(),
            Some("When you're unsure it is right")
        );
        assert_eq!(
            meta_phase_titles(meta),
            vec![
                "Scan".to_string(),
                "Verify".to_string(),
                "Report".to_string()
            ]
        );

        assert!(meta_literal("no meta here").is_none());
        assert_eq!(
            meta_phase_titles("{ description: 'x' }"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn project_browser_stays_inside_roots_and_prioritizes_git_projects() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let ordinary = root.path().join("alpha");
        let git = root.path().join("zeta-git");
        std::fs::create_dir_all(&ordinary).unwrap();
        std::fs::create_dir_all(git.join(".git")).unwrap();
        std::fs::create_dir_all(root.path().join(".hidden")).unwrap();
        let canonical_root = root.path().canonicalize().unwrap();

        let result =
            browse_project_directory(canonical_root.clone(), vec![canonical_root.clone()]).unwrap();
        assert_eq!(result.path, canonical_root.display().to_string());
        assert!(
            result.parent.is_none(),
            "the browse root cannot navigate upward"
        );
        assert_eq!(
            result
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["zeta-git", "alpha"]
        );
        assert!(result.entries[0].is_git_repo);

        let error = browse_project_directory(outside.path().to_path_buf(), vec![canonical_root])
            .err()
            .expect("outside path must be rejected");
        assert!(
            error.contains("outside the configured browse roots"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn project_browser_rejects_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let escape = root.path().join("escape");
        symlink(outside.path(), &escape).unwrap();
        let canonical_root = root.path().canonicalize().unwrap();

        let listing =
            browse_project_directory(canonical_root.clone(), vec![canonical_root.clone()]).unwrap();
        assert!(listing.entries.iter().all(|entry| entry.name != "escape"));

        let error = browse_project_directory(escape, vec![canonical_root])
            .err()
            .expect("symlink escape must be rejected");
        assert!(
            error.contains("outside the configured browse roots"),
            "{error}"
        );
    }

    #[test]
    fn parse_temper_accepts_every_picker_name_case_insensitively() {
        use forge_types::PermissionMode;
        for (raw, expect) in [
            ("Read-only", PermissionMode::Plan),
            ("read-ONLY", PermissionMode::Plan),
            ("Ask", PermissionMode::Default),
            ("ASK", PermissionMode::Default),
            ("Auto-edit", PermissionMode::AcceptEdits),
            ("auto-EDIT", PermissionMode::AcceptEdits),
            ("Full", PermissionMode::Bypass),
            ("FULL", PermissionMode::Bypass),
            // Picker-level availability is the bar: Full parses with no extra guard here, same
            // as it's offered (unguarded) as a row by the `/mode` picker.
            ("bypass", PermissionMode::Bypass),
        ] {
            assert_eq!(parse_temper(raw), Ok(expect), "temper {raw:?}");
        }
    }

    #[test]
    fn parse_temper_rejects_unknown_value_listing_every_valid_name() {
        let err = parse_temper("yolo").unwrap_err();
        assert!(err.contains("yolo"), "{err}");
        for label in ["Read-only", "Ask", "Auto-edit", "Full"] {
            assert!(err.contains(label), "{err} missing {label}");
        }
    }

    #[test]
    fn daemon_token_persists_and_rotates() {
        let dir = std::env::temp_dir().join(format!("forge-serve-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("serve-token");

        // First call mints a 32-hex token and writes it.
        let t1 = daemon_token_at(&path, false).unwrap();
        assert_eq!(t1.len(), 32);
        assert!(t1.chars().all(|c| c.is_ascii_hexdigit()));
        // Second call returns the SAME token (stable origin is the whole point).
        let t2 = daemon_token_at(&path, false).unwrap();
        assert_eq!(t1, t2, "token is stable across restarts");
        // Rotation mints a fresh one and persists it.
        let t3 = daemon_token_at(&path, true).unwrap();
        assert_ne!(t1, t3, "rotate mints a new token");
        assert_eq!(daemon_token_at(&path, false).unwrap(), t3);
        // 0600 on unix: the token is the daemon's only credential.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "token file is owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_state_serializes_expected_shape() {
        let state = ServeState {
            pid: 12345,
            port: 7420,
            exposure: "local".to_string(),
            base_url: "http://127.0.0.1:7420/abc123".to_string(),
            token: "abc123".to_string(),
            started_at: 1_700_000_000,
        };
        let parsed: serde_json::Value = serde_json::from_str(&state.to_json()).unwrap();
        assert_eq!(parsed["pid"], 12345);
        assert_eq!(parsed["port"], 7420);
        assert_eq!(parsed["exposure"], "local");
        assert_eq!(parsed["base_url"], "http://127.0.0.1:7420/abc123");
        assert_eq!(parsed["token"], "abc123");
        assert_eq!(parsed["started_at"], 1_700_000_000);
    }

    #[tokio::test]
    async fn fleet_subscribers_observe_registry_invalidations() {
        let registry = SessionRegistry::new();
        let mut subscriber = registry.subscribe_fleet();
        assert_eq!(*subscriber.borrow(), 0);
        registry.notify_fleet();
        subscriber.changed().await.unwrap();
        assert_eq!(*subscriber.borrow_and_update(), 1);
        registry.notify_fleet();
        subscriber.changed().await.unwrap();
        assert_eq!(*subscriber.borrow_and_update(), 2);
    }

    #[test]
    fn serve_state_writes_0600_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("forge-serve-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("serve-state.json");
        let state = ServeState {
            pid: std::process::id(),
            port: 7452,
            exposure: "anywhere".to_string(),
            base_url: "https://example.trycloudflare.com/deadbeef".to_string(),
            token: "deadbeef".to_string(),
            started_at: 1_700_000_001,
        };
        write_state_at(&path, &state).unwrap();
        let read_back: ServeState =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read_back, state);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "state file is owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The daemon's core promises, end to end over REAL driver tasks (offline mock provider,
    /// isolated FORGE_DB): registry create/list/archive; two sessions driven CONCURRENTLY over
    /// separate handles with zero cross-talk; sessions keep running with zero clients attached
    /// (nothing ever connects a WS here — input goes straight down each handle's queue); the
    /// per-session event log answers replay like the single-session server's; and archiving
    /// stops the driver + hides the session while keeping its history.
    /// Serializes the tests that point `FORGE_DB` at a scratch database — the env var is
    /// process-wide, so two such tests running in parallel would race each other's stores.
    static FORGE_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn daemon_sessions_are_isolated_and_survive_zero_clients() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // NEVER the real store: everything below writes sessions/messages.
        std::env::set_var("FORGE_DB", dir.join("serve-test.db"));

        let registry = SessionRegistry::new();
        let cwd = dir.display().to_string();
        let mk = |title: &str| DriverSpec {
            cwd: cwd.clone(),
            worktree: None,
            title: title.to_string(),
            mock: true,
            model: None,
            resume: None,
            temper: None,
            push: None,
            apns: None,
        };
        let a = registry
            .insert(spawn_session_driver(mk("alpha")).await.unwrap())
            .await;
        let b = registry
            .insert(spawn_session_driver(mk("beta")).await.unwrap())
            .await;
        assert_ne!(a.session_id, b.session_id);
        assert_eq!(registry.all().await.len(), 2, "both sessions listed");

        // Drive both concurrently over their own handles.
        a.input_tx
            .send(remote::RemoteInput::Prompt {
                text: "alpha-marker task".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        b.input_tx
            .send(remote::RemoteInput::Prompt {
                text: "beta-marker task".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();

        async fn wait_done(h: &SessionDriverHandle, needle: &str) -> remote::Snapshot {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let s = h.snapshot_rx.borrow().snapshot.clone();
                if !s.busy && s.transcript.join("\n").contains(needle) {
                    return s;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for {needle:?}; snapshot: {s:?}"
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        let sa = wait_done(&a, "alpha-marker").await;
        let sb = wait_done(&b, "beta-marker").await;
        // Isolation: neither session's stream carries the other's turn.
        assert!(!sa.transcript.join("\n").contains("beta-marker"));
        assert!(!sb.transcript.join("\n").contains("alpha-marker"));
        assert_eq!(sa.session_id, a.session_id);
        assert_eq!(sb.session_id, b.session_id);
        // v6 identity fields ride in every frame.
        assert_eq!(sa.title, "alpha");
        assert_eq!(sb.title, "beta");
        assert_eq!(sa.cwd, cwd);

        // Zero clients: no WS was ever attached, and the watch receiver we hold is passive —
        // the session still accepts + completes another turn.
        a.input_tx
            .send(remote::RemoteInput::Prompt {
                text: "second-marker follow-up".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        let sa2 = wait_done(&a, "second-marker").await;
        assert!(sa2.revision > sa.revision, "state kept advancing");
        // The event log can replay the frames a disconnected client missed.
        let replayed = a
            .events
            .lock()
            .unwrap()
            .replay_after(sa.revision)
            .expect("gap is fillable from the ring");
        assert!(
            replayed.iter().any(|s| s.snapshot.revision == sa2.revision),
            "replay covers the missed frames"
        );

        // Archive beta: driver stops, session is hidden from lists, history survives.
        let store = crate::open_store().unwrap();
        let removed = registry.remove(&b.session_id).await.expect("beta present");
        removed.shutdown();
        store.archive_session(&b.session_id).unwrap();
        if let Ok(h) = Arc::try_unwrap(removed) {
            h.join(std::time::Duration::from_secs(5)).await;
        }
        assert_eq!(registry.all().await.len(), 1, "beta removed from registry");
        assert!(store.session_archived(&b.session_id).unwrap());
        let listed: Vec<String> = store
            .list_sessions()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert!(!listed.contains(&b.session_id), "archived → hidden");
        assert!(listed.contains(&a.session_id), "alpha still listed");
        assert!(
            !store.load_messages(&b.session_id).unwrap().is_empty(),
            "archive hides, never deletes"
        );
        // The archived driver's last frame tells clients to stop reconnecting. The driver winds
        // down asynchronously (it notices shutdown on its next loop tick, then runs SessionEnd
        // hooks before the final broadcast) — and the `Arc::try_unwrap` join above is skipped
        // whenever the test still holds its own handle clone — so poll instead of racing it
        // (observed flaky on contended CI runners).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !b.snapshot_rx.borrow().snapshot.closed {
            assert!(
                std::time::Instant::now() < deadline,
                "final closed frame never arrived"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `DriverSpec::temper` reaches the session through the same [`forge_core::Session::
    /// set_temper`] setter `picker_accept` calls for `PickerKind::Tempers` — including `Full`
    /// (Bypass), which the `/mode` picker offers unguarded, so the API applies it the same way
    /// with no extra confirmation layered on top.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn driver_spec_temper_sets_the_session_permission_mode() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-temper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("temper-test.db"));

        let handle = spawn_session_driver(DriverSpec {
            cwd: dir.display().to_string(),
            worktree: None,
            title: "temper-e2e".into(),
            mock: true,
            model: None,
            resume: None,
            temper: Some(forge_types::PermissionMode::Bypass),
            push: None,
            apns: None,
        })
        .await
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let temper = handle.snapshot_rx.borrow().snapshot.temper.clone();
            if temper == "Full" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "temper never applied — last seen {temper:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        handle.shutdown();
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Worktree-backed session create: the worktree exists on disk, is a real git worktree of
    /// the repo, and the driver session runs inside it (cwd + snapshot.worktree agree).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worktree_session_create_makes_a_real_worktree() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("serve-wt.db"));
        // A tiny real repo with one commit (worktrees branch from HEAD).
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(dir.join("README.md"), "hi\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);

        assert!(forge_core::worktree::is_git_repo(&dir));
        let wt_id = forge_types::new_id().chars().take(12).collect::<String>();
        let guard = forge_core::worktree::WorktreeGuard::create(&dir, &wt_id).unwrap();
        let wt_path = guard.path().display().to_string();
        std::mem::forget(guard); // daemon semantics: the worktree outlives the handle

        let handle = spawn_session_driver(DriverSpec {
            cwd: wt_path.clone(),
            worktree: Some(wt_path.clone()),
            title: "wt".into(),
            mock: true,
            model: None,
            resume: None,
            temper: None,
            push: None,
            apns: None,
        })
        .await
        .unwrap();
        assert!(std::path::Path::new(&wt_path).join(".git").exists());
        handle
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "hello worktree".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let snap = loop {
            let s = handle.snapshot_rx.borrow().snapshot.clone();
            if !s.busy && !s.transcript.is_empty() {
                break s;
            }
            assert!(std::time::Instant::now() < deadline, "turn never finished");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(snap.worktree.as_deref(), Some(wt_path.as_str()));
        assert_eq!(snap.cwd, wt_path);
        // The store row remembers the worktree (survives daemon restarts).
        let store = crate::open_store().unwrap();
        assert_eq!(
            store
                .session_worktree(&handle.session_id)
                .unwrap()
                .as_deref(),
            Some(wt_path.as_str())
        );
        handle.shutdown();
        handle.join(std::time::Duration::from_secs(5)).await;
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A minimal real git repo under `dir` with one commit of `files`, returning a `git` runner.
    fn init_repo(dir: &std::path::Path, files: &[(&str, &str)]) -> impl Fn(&[&str]) + use<> {
        let d = dir.to_path_buf();
        let git = move |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&d)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        git(&["init", "-q"]);
        for (name, content) in files {
            std::fs::write(dir.join(name), content).unwrap();
        }
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        git
    }

    /// Spawn a mock worktree-backed session (leaked guard, daemon semantics) and register it.
    async fn spawn_worktree_session(
        registry: &SessionRegistry,
        repo: &std::path::Path,
    ) -> (Arc<SessionDriverHandle>, String) {
        let wt_id = forge_types::new_id().chars().take(12).collect::<String>();
        let guard = forge_core::worktree::WorktreeGuard::create(repo, &wt_id).unwrap();
        let wt_path = guard.path().display().to_string();
        std::mem::forget(guard);
        let handle = registry
            .insert(
                spawn_session_driver(DriverSpec {
                    cwd: wt_path.clone(),
                    worktree: Some(wt_path.clone()),
                    title: "wt".into(),
                    mock: true,
                    model: None,
                    resume: None,
                    temper: None,
                    push: None,
                    apns: None,
                })
                .await
                .unwrap(),
            )
            .await;
        (handle, wt_path)
    }

    fn post_req(base_and_path: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::post(base_and_path)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    async fn json_body(resp: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    /// A clean worktree merge: the branch's new file applies onto the base tree (staged), and the
    /// worktree directory + branch are removed. Runs over the REAL router + a REAL driver.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worktree_merge_clean_applies_and_removes() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("merge.db"));
        let _ = init_repo(&dir, &[("README.md", "hi\n")]);

        let registry = Arc::new(SessionRegistry::new());
        let (handle, wt_path) = spawn_worktree_session(&registry, &dir).await;
        // A brand-new file in the worktree — the change we expect to see merged back.
        std::fs::write(
            std::path::Path::new(&wt_path).join("added.txt"),
            "from worktree\n",
        )
        .unwrap();

        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let resp = router
            .clone()
            .oneshot(post_req(&format!(
                "/tok/api/sessions/{}/merge",
                handle.session_id
            )))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::OK,
            "clean merge is 200"
        );
        let j = json_body(resp).await;
        assert_eq!(j["merged"], true);
        // The change landed in the base tree, staged for review, not committed.
        assert!(
            dir.join("added.txt").exists(),
            "merged file is in the base tree"
        );
        let staged = std::process::Command::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "diff",
                "--cached",
                "--name-only",
            ])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&staged.stdout).contains("added.txt"),
            "the merged change is staged"
        );
        // Worktree + branch are gone.
        assert!(
            !std::path::Path::new(&wt_path).exists(),
            "worktree dir removed"
        );
        let branch = worktree_repo_and_branch(&wt_path).unwrap().1;
        let exists = std::process::Command::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "rev-parse",
                "--verify",
                &branch,
            ])
            .output()
            .unwrap();
        assert!(!exists.status.success(), "branch removed after clean merge");
        // Idempotent-ish: a second merge on the now-unknown session is a 404, never a panic.
        let resp = router
            .oneshot(post_req(&format!(
                "/tok/api/sessions/{}/merge",
                handle.session_id
            )))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A conflicting merge: the base committed a different edit to the same lines, so the 3-way
    /// apply conflicts — the route reports the file and leaves the worktree + branch INTACT.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worktree_merge_conflict_reports_and_preserves() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-conf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("conf.db"));
        let git = init_repo(&dir, &[("f.txt", "l1\nl2\nl3\n")]);

        let registry = Arc::new(SessionRegistry::new());
        let (handle, wt_path) = spawn_worktree_session(&registry, &dir).await;
        // Worktree edits the middle line one way…
        std::fs::write(
            std::path::Path::new(&wt_path).join("f.txt"),
            "l1\nWORKTREE\nl3\n",
        )
        .unwrap();
        // …while the base COMMITS a different middle line (clean tree, but conflicting content).
        std::fs::write(dir.join("f.txt"), "l1\nBASE\nl3\n").unwrap();
        git(&["commit", "-qam", "base edit"]);

        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let resp = router
            .oneshot(post_req(&format!(
                "/tok/api/sessions/{}/merge",
                handle.session_id
            )))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::CONFLICT,
            "conflict is 409"
        );
        let j = json_body(resp).await;
        assert!(
            j["conflicts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == "f.txt"),
            "the conflicting file is reported: {j}"
        );
        // Nothing destroyed: worktree + branch survive for a manual resolution.
        assert!(
            std::path::Path::new(&wt_path).exists(),
            "worktree kept on conflict"
        );
        let branch = worktree_repo_and_branch(&wt_path).unwrap().1;
        let exists = std::process::Command::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "rev-parse",
                "--verify",
                &branch,
            ])
            .output()
            .unwrap();
        assert!(exists.status.success(), "branch kept on conflict");
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A dirty base tree refuses the merge (409) and leaves the session RUNNING — never a silent
    /// apply on top of uncommitted work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worktree_merge_refuses_dirty_base() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-dirty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("dirty.db"));
        let _ = init_repo(&dir, &[("f.txt", "base\n")]);

        let registry = Arc::new(SessionRegistry::new());
        let (handle, wt_path) = spawn_worktree_session(&registry, &dir).await;
        std::fs::write(std::path::Path::new(&wt_path).join("x.txt"), "wt\n").unwrap();
        // Uncommitted TRACKED edit in the base tree.
        std::fs::write(dir.join("f.txt"), "dirty edit\n").unwrap();

        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let resp = router
            .oneshot(post_req(&format!(
                "/tok/api/sessions/{}/merge",
                handle.session_id
            )))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
        let j = json_body(resp).await;
        assert!(j["dirty_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "f.txt"));
        // Refusal is non-destructive: the session is still registered and running.
        assert!(
            registry.get(&handle.session_id).await.is_some(),
            "session untouched"
        );
        assert!(std::path::Path::new(&wt_path).exists());
        handle.shutdown();
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Discard drops the worktree + branch without merging.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worktree_discard_drops_worktree_and_branch() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-disc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("disc.db"));
        let _ = init_repo(&dir, &[("README.md", "hi\n")]);

        let registry = Arc::new(SessionRegistry::new());
        let (handle, wt_path) = spawn_worktree_session(&registry, &dir).await;
        std::fs::write(
            std::path::Path::new(&wt_path).join("junk.txt"),
            "throwaway\n",
        )
        .unwrap();
        let branch = worktree_repo_and_branch(&wt_path).unwrap().1;

        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let resp = router
            .oneshot(post_req(&format!(
                "/tok/api/sessions/{}/discard",
                handle.session_id
            )))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(json_body(resp).await["discarded"], true);
        assert!(!std::path::Path::new(&wt_path).exists(), "worktree removed");
        assert!(
            !dir.join("junk.txt").exists(),
            "worktree edit is NOT in the base tree"
        );
        let exists = std::process::Command::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "rev-parse",
                "--verify",
                &branch,
            ])
            .output()
            .unwrap();
        assert!(!exists.status.success(), "branch removed on discard");
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The past-session browser lists persisted, non-running sessions and excludes the ones the
    /// daemon is actively driving.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn past_sessions_lists_persisted_not_running() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-past-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("past.db"));

        let mk = |title: &str| DriverSpec {
            cwd: dir.display().to_string(),
            worktree: None,
            title: title.to_string(),
            mock: true,
            model: None,
            resume: None,
            temper: None,
            push: None,
            apns: None,
        };
        // A "past" session: driven to completion (so it has a user message) but NOT registered.
        let past = Arc::new(spawn_session_driver(mk("gone")).await.unwrap());
        past.input_tx
            .send(remote::RemoteInput::Prompt {
                text: "past-marker".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        // A "running" session: registered in the registry, so it must be excluded from the list.
        let registry = Arc::new(SessionRegistry::new());
        let running = registry
            .insert(spawn_session_driver(mk("live")).await.unwrap())
            .await;
        running
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "live-marker".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();

        let wait_idle = |h: Arc<SessionDriverHandle>| async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let s = h.snapshot_rx.borrow().snapshot.clone();
                if !s.busy && !s.transcript.is_empty() {
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "turn never finished");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        };
        wait_idle(past.clone()).await;
        wait_idle(running.clone()).await;

        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: Arc::new(crate::open_store().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let resp = router
            .oneshot(
                axum::http::Request::get("/tok/api/sessions/past?limit=50")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let rows = json_body(resp).await;
        let ids: Vec<&str> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert!(
            ids.contains(&past.session_id.as_str()),
            "past session listed: {rows}"
        );
        assert!(
            !ids.contains(&running.session_id.as_str()),
            "the actively-driven session is NOT in the past list"
        );

        past.shutdown();
        running.shutdown();
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session hidden by the archive button (not just orphaned by a daemon restart) is ALSO
    /// browsable from `/api/sessions/past`, flagged `archived: true` — and resuming it works and
    /// un-archives it, so it stops being hidden once it's live again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn past_sessions_includes_archived_and_resume_unarchives() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir =
            std::env::temp_dir().join(format!("forge-serve-past-arch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("past-arch.db"));

        let mk = |title: &str| DriverSpec {
            cwd: dir.display().to_string(),
            worktree: None,
            title: title.to_string(),
            mock: true,
            model: None,
            resume: None,
            temper: None,
            push: None,
            apns: None,
        };
        let archived = Arc::new(spawn_session_driver(mk("was-archived")).await.unwrap());
        archived
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "archived-marker".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();

        let wait_idle = |h: Arc<SessionDriverHandle>| async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let s = h.snapshot_rx.borrow().snapshot.clone();
                if !s.busy && !s.transcript.is_empty() {
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "turn never finished");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        };
        wait_idle(archived.clone()).await;
        let archived_id = archived.session_id.clone();

        // Stop the driver and archive it, mirroring `POST .../archive`'s effect on the store.
        archived.shutdown();
        if let Ok(h) = Arc::try_unwrap(archived) {
            h.join(std::time::Duration::from_secs(5)).await;
        }
        let store = crate::open_store().unwrap();
        store.archive_session(&archived_id).unwrap();
        assert!(store.session_archived(&archived_id).unwrap());

        let registry = Arc::new(SessionRegistry::new());
        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: Arc::new(crate::open_store().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);

        // (1) It's listed, flagged `archived: true` — distinct from the merely-orphaned case.
        let resp = router
            .clone()
            .oneshot(
                axum::http::Request::get("/tok/api/sessions/past?limit=50")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let rows = json_body(resp).await;
        let row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == archived_id)
            .expect("archived session listed in past-sessions");
        assert_eq!(row["archived"], true, "flagged archived: {row}");

        // (2) Resuming it succeeds and un-archives it.
        let post_json = |path: &str, body: String| {
            axum::http::Request::post(format!("/tok{path}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap()
        };
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/sessions",
                serde_json::json!({ "resume": archived_id }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "resume succeeds");
        let body = json_body(resp).await;
        assert_eq!(body["id"], archived_id, "resumed the same session");

        let store = crate::open_store().unwrap();
        assert!(
            !store.session_archived(&archived_id).unwrap(),
            "resume un-archives the session"
        );

        if let Some(h) = registry.remove(&archived_id).await {
            h.shutdown();
            if let Ok(h) = Arc::try_unwrap(h) {
                h.join(std::time::Duration::from_secs(5)).await;
            }
        }
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    use p256::elliptic_curve::sec1::ToSec1Point;
    use tower::util::ServiceExt;

    /// A daemon state + router over an in-memory store with NO push configured — the push
    /// routes must degrade to 503 (and `/api/answer` to 404 for unknown sessions), never panic.
    #[tokio::test]
    async fn push_routes_degrade_cleanly_without_a_vapid_key() {
        let state = Arc::new(DaemonState {
            registry: Arc::new(SessionRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: ".".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let get_key = axum::http::Request::get("/tok/api/push/key")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(get_key).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let answer = axum::http::Request::post("/tok/api/answer")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"session":"nope","seq":1,"allow":true}"#,
            ))
            .unwrap();
        let resp = router.clone().oneshot(answer).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// A device-token (APNs) or Live Activity subscribe attempt with no APNs key configured
    /// degrades to a 503 — same contract as the Web Push routes above, never a panic.
    #[tokio::test]
    async fn apns_routes_degrade_cleanly_without_a_key() {
        let state = Arc::new(DaemonState {
            registry: Arc::new(SessionRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: ".".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let post_json = |path: &str, body: String| {
            axum::http::Request::post(format!("/tok{path}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap()
        };
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/push/subscribe",
                serde_json::json!({ "device_token": "abc", "environment": "sandbox" }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/push/subscribe",
                serde_json::json!({
                    "session_id": "s1", "push_token": "tok", "environment": "sandbox"
                })
                .to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    /// A throwaway PKCS8 EC test key (`openssl ecparam -genkey -name prime256v1 | openssl pkcs8
    /// -topk8 -nocrypt`) — not a real Apple credential, purely so tests can construct a real
    /// `ApnsNotifier` without touching the filesystem or a real Apple account.
    #[cfg(test)]
    const TEST_APNS_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggdRyR/MhlcExmPgF\n\
        bvIV3tJ+Aa3erm2iVpWJnKOfBRWhRANCAATERnMvcXzCXFTZmK+y7nBo+w+1DKDY\n\
        Zr6aHkQuv2itYFWLLjJmaTE17aL3i0yDIvQ3G1FZIPWcsyL2qfK1T5el\n\
        -----END PRIVATE KEY-----\n";

    /// The three subscribe shapes (Web Push, APNs device token, Live Activity token) route to
    /// the right store table via the untagged `SubscribeReq` discriminator, and each unsubscribe
    /// actually removes what its subscribe stored.
    #[tokio::test]
    async fn apns_and_live_activity_subscribe_unsubscribe_round_trip() {
        let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
        let apns_config = crate::apns::ApnsConfig::from_pem_for_test(
            TEST_APNS_KEY_PEM,
            "TEAM123456",
            "KEY7890AB",
        );
        let apns =
            Arc::new(crate::apns::ApnsNotifier::new_direct(store.clone(), apns_config).unwrap());
        let state = Arc::new(DaemonState {
            registry: Arc::new(SessionRegistry::new()),
            store: store.clone(),
            base: "/tok".into(),
            mock: true,
            default_cwd: ".".into(),
            project_roots: Vec::new(),
            push: None,
            apns: Some(apns),
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let post_json = |path: &str, body: String| {
            axum::http::Request::post(format!("/tok{path}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap()
        };

        for invalid_body in [
            serde_json::json!({ "device_token": "not-an-apns-token", "environment": "production" }),
            serde_json::json!({
                "session_id": "sess-invalid",
                "push_token": "not-a-live-activity-token",
                "environment": "sandbox"
            }),
        ] {
            let resp = router
                .clone()
                .oneshot(post_json("/api/push/subscribe", invalid_body.to_string()))
                .await
                .unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        }
        assert!(store.list_apns_subscriptions().unwrap().is_empty());
        assert!(store
            .get_live_activity_token("sess-invalid")
            .unwrap()
            .is_none());

        // APNs device token: subscribe stores it, unsubscribe removes it.
        let device_token = "a0".repeat(32);
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/push/subscribe",
                serde_json::json!({ "device_token": device_token, "environment": "production" })
                    .to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(store.list_apns_subscriptions().unwrap().len(), 1);
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/push/unsubscribe",
                serde_json::json!({ "device_token": device_token }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(store.list_apns_subscriptions().unwrap().is_empty());

        // Live Activity token: subscribe stores it keyed by session, unsubscribe removes it.
        let activity_token = "b1".repeat(32);
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/push/subscribe",
                serde_json::json!({
                    "session_id": "sess-1", "push_token": activity_token, "environment": "sandbox"
                })
                .to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            store
                .get_live_activity_token("sess-1")
                .unwrap()
                .expect("stored")
                .push_token,
            activity_token
        );
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/push/unsubscribe",
                serde_json::json!({
                    "session_id": "sess-1", "push_token": activity_token
                })
                .to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(store.get_live_activity_token("sess-1").unwrap().is_none());
    }

    /// `GET /api/upload` (v7 history-image playback): serves back a file previously stored by
    /// `POST /api/upload` — success, path-traversal rejection, and an unknown session all in one
    /// pass, since they share the same confinement check.
    #[tokio::test]
    async fn serve_upload_streams_a_stored_file_and_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("forge-serve-upload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
        let session_id = store
            .create_session(&dir.display().to_string(), "safe")
            .unwrap();

        // Stash a file exactly where `remote::store_upload` would have put it.
        let uploads = dir
            .join(".forge")
            .join("uploads")
            .join(remote::sanitize_upload_name(&session_id));
        std::fs::create_dir_all(&uploads).unwrap();
        let file_path = uploads.join("123-photo.png");
        std::fs::write(&file_path, b"fake-png-bytes").unwrap();
        let path_str = file_path.display().to_string();

        let state = Arc::new(DaemonState {
            registry: Arc::new(SessionRegistry::new()),
            store: store.clone(),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);

        // Successful serve: 200, right bytes, content-type inferred from the extension.
        let req = axum::http::Request::get(format!(
            "/tok/api/upload?session={session_id}&path={path_str}"
        ))
        .body(axum::body::Body::empty())
        .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "image/png"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"fake-png-bytes");

        // A path outside THIS session's `.forge/uploads/` (a traversal / wrong-session attempt)
        // is a 403, never a 500 — even though the file genuinely exists on disk.
        let outside = dir.join("secret.txt");
        std::fs::write(&outside, b"nope").unwrap();
        let req = axum::http::Request::get(format!(
            "/tok/api/upload?session={session_id}&path={}",
            outside.display()
        ))
        .body(axum::body::Body::empty())
        .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);

        // An unknown session is a 404, never a 500.
        let req = axum::http::Request::get(format!(
            "/tok/api/upload?session=nope-not-a-real-session&path={path_str}"
        ))
        .body(axum::body::Body::empty())
        .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole Phase-5 promise, end to end over a REAL driver (offline mock provider,
    /// isolated FORGE_DB **and** isolated config so the temper is the safe default that asks
    /// before writes) and a REAL local push endpoint standing in for the browser vendor:
    ///
    /// 1. a browser subscription stored through `POST /api/push/subscribe` (garbage rejected),
    /// 2. a turn hits a permission prompt with ZERO clients attached → an **encrypted** POST
    ///    arrives at the push endpoint carrying a verifiable VAPID JWT + TTL, and the body
    ///    decrypts (with the receiver's keys) to the permission payload with the right seq,
    /// 3. `POST /api/answer` rejects a wrong session (404) and a stale seq (409), accepts the
    ///    current seq (200) — resolving the prompt exactly like a WS Allow,
    /// 4. with a client attached, the turn-done transition is debounced: NO push,
    /// 5. with the client gone, the next completed turn pushes `"done"`,
    /// 6. `POST /api/push/unsubscribe` forgets the subscription.
    // 4 workers: this test parks one in `block_in_place` (the turn's pending confirm), runs the
    // driver loop, the mock push HTTP service, AND reqwest deliveries concurrently — headroom
    // matters on CI's small, contended runners.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn permission_prompt_pushes_encrypted_notification_and_debounces() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-push-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("push-test.db"));
        // Isolated config dir + a PINNED permission temper. The pin matters: config loading
        // merges `./.forge/config.toml` with figment's parent-directory search, so a dev
        // machine's (gitignored) repo config — or the shipped AcceptEdits default on CI —
        // would otherwise decide whether write_file prompts. `FORGE_PERMISSION_MODE` is the
        // env layer (highest precedence), so "ask before writes" holds everywhere. Both are
        // restored below; the only tests that build sessions serialize on FORGE_DB_LOCK.
        let old_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("FORGE_PERMISSION_MODE", "default");
        // The test binary's cwd is inside the Forge workspace, so autofix's project-structure
        // auto-detection would run `cargo check --all-targets` against the WHOLE WORKSPACE after
        // the allowed write — minutes on a cold CI cache (the diagnosed "turn never completed").
        std::env::set_var("FORGE_AUTOFIX__AUTO_DETECT", "false");

        // The stand-in for the browser vendor's push service: captures every POST it receives.
        let (cap_tx, mut cap_rx) =
            tokio::sync::mpsc::unbounded_channel::<(axum::http::HeaderMap, Vec<u8>)>();
        let push_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let push_addr = push_listener.local_addr().unwrap();
        let push_service = Router::new().route(
            "/wp/{id}",
            post(
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let tx = cap_tx.clone();
                    async move {
                        let _ = tx.send((headers, body.to_vec()));
                        axum::http::StatusCode::CREATED
                    }
                },
            ),
        );
        tokio::spawn(async move {
            axum::serve(push_listener, push_service).await.ok();
        });

        // The "browser": a fixed receiver keypair + auth secret we can decrypt with.
        let ua_secret = p256::SecretKey::from_slice(&[42u8; 32]).unwrap();
        let ua_public = ua_secret.public_key().to_sec1_point(false);
        let auth: [u8; 16] = [7u8; 16];
        let endpoint = format!("http://{push_addr}/wp/sub1");

        let store = Arc::new(crate::open_store().unwrap());
        let vapid_public;
        let notifier = {
            let n = Arc::new(
                crate::push::PushNotifier::with_key(
                    store.clone(),
                    crate::push::VapidKey::from_scalar(&[9u8; 32]),
                )
                .unwrap(),
            );
            vapid_public = n.public_key_b64url();
            n
        };

        // A real driver with push wired, hosted behind the real daemon router.
        let registry = Arc::new(SessionRegistry::new());
        let handle = registry
            .insert(
                spawn_session_driver(DriverSpec {
                    cwd: dir.display().to_string(),
                    worktree: None,
                    title: "push-e2e".into(),
                    mock: true,
                    model: None,
                    resume: None,
                    temper: None,
                    push: Some(notifier.clone()),
                    apns: None,
                })
                .await
                .unwrap(),
            )
            .await;
        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: store.clone(),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: Some(notifier),
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let post_json = |path: &str, body: String| {
            axum::http::Request::post(format!("/tok{path}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap()
        };

        // (1) The advertised key is the VAPID public key; the subscription stores through the
        // route (and a garbage one is rejected at the door).
        let resp = router
            .clone()
            .oneshot(
                axum::http::Request::get("/tok/api/push/key")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1 << 16)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["key"], vapid_public);

        let sub_body = serde_json::json!({
            "endpoint": endpoint,
            "keys": {
                "p256dh": crate::push::b64url(ua_public.as_bytes()),
                "auth": crate::push::b64url(&auth),
            },
        })
        .to_string();
        let resp = router
            .clone()
            .oneshot(post_json("/api/push/subscribe", sub_body.clone()))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        // Re-subscribing dedupes; a malformed key is a 400.
        router
            .clone()
            .oneshot(post_json("/api/push/subscribe", sub_body))
            .await
            .unwrap();
        assert_eq!(store.list_push_subscriptions().unwrap().len(), 1);
        let bad = serde_json::json!({
            "endpoint": endpoint, "keys": {"p256dh": "bogus", "auth": "AAAA"}
        });
        let resp = router
            .clone()
            .oneshot(post_json("/api/push/subscribe", bad.to_string()))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);

        // (2) Run a turn that hits a permission prompt with zero clients attached.
        handle
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "mock:write please".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let pending = loop {
            let s = handle.snapshot_rx.borrow().snapshot.clone();
            if s.permission_prompt.is_some() {
                break s;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no permission prompt appeared; snapshot: {s:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        };
        let (headers, sealed) =
            tokio::time::timeout(std::time::Duration::from_secs(30), cap_rx.recv())
                .await
                .expect("a push must arrive at the (mock) push service")
                .expect("capture channel open");
        assert_eq!(
            headers.get("ttl").unwrap().to_str().unwrap(),
            "300",
            "decision TTL"
        );
        assert_eq!(
            headers.get("content-encoding").unwrap().to_str().unwrap(),
            "aes128gcm"
        );
        assert_eq!(headers.get("urgency").unwrap().to_str().unwrap(), "high");
        let authz = headers.get("authorization").unwrap().to_str().unwrap();
        let t = authz
            .strip_prefix("vapid t=")
            .and_then(|r| r.split(", k=").next())
            .expect("vapid t= present");
        assert_eq!(
            authz.split(", k=").nth(1),
            Some(vapid_public.as_str()),
            "k= advertises OUR key"
        );
        // The JWT verifies with the advertised key and targets the push service's origin.
        let seg: Vec<&str> = t.split('.').collect();
        assert_eq!(seg.len(), 3);
        let claims: serde_json::Value =
            serde_json::from_slice(&crate::push::b64url_decode(seg[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], format!("http://{push_addr}"));
        use p256::ecdsa::signature::Verifier;
        let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(
            &crate::push::b64url_decode(&vapid_public).unwrap(),
        )
        .unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&crate::push::b64url_decode(seg[2]).unwrap())
            .unwrap();
        vk.verify(format!("{}.{}", seg[0], seg[1]).as_bytes(), &sig)
            .expect("VAPID JWT verifies");
        // The body is REAL ciphertext that only the receiver's keys open — and it says exactly
        // which prompt needs answering.
        let payload = crate::push::decrypt_payload(&ua_secret, &auth, &sealed).unwrap();
        let p: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(p["kind"], "permission");
        assert_eq!(p["session"], handle.session_id.as_str());
        assert_eq!(p["seq"], pending.prompt_seq);
        assert!(
            p["body"].as_str().unwrap().contains("write_file"),
            "payload names the tool: {p}"
        );

        // (3) /api/answer: wrong session 404; stale seq 409 (prompt survives); current seq 200.
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/answer",
                format!(
                    r#"{{"session":"ghost","seq":{},"allow":true}}"#,
                    pending.prompt_seq
                ),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/answer",
                format!(
                    r#"{{"session":"{}","seq":{},"allow":true}}"#,
                    handle.session_id,
                    pending.prompt_seq + 1
                ),
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::CONFLICT,
            "a stale seq can never approve"
        );
        assert!(
            handle
                .snapshot_rx
                .borrow()
                .snapshot
                .permission_prompt
                .is_some(),
            "the prompt survives a stale answer"
        );
        // (4) Attach a "client" BEFORE approving so the coming turn-done edge is debounced.
        handle
            .ws_clients
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/answer",
                format!(
                    r#"{{"session":"{}","seq":{},"allow":true}}"#,
                    handle.session_id, pending.prompt_seq
                ),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        // Generous deadline: CI runners execute the whole suite in parallel on few cores, so
        // wall-clock here includes heavy scheduler contention, not just the (instant) mock turn.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let s = handle.snapshot_rx.borrow().snapshot.clone();
            if !s.busy && s.permission_prompt.is_none() && !s.transcript.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "turn never completed after the remote allow; last snapshot: busy={} prompt={:?} \
                 question={:?} seq={} notes={:?} transcript_tail={:?}",
                s.busy,
                s.permission_prompt,
                s.question,
                s.prompt_seq,
                s.notes,
                s.transcript.iter().rev().take(3).collect::<Vec<_>>()
            );
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        assert!(
            !handle
                .snapshot_rx
                .borrow()
                .snapshot
                .transcript
                .iter()
                .any(|l| l.contains("autofix")),
            "FORGE_AUTOFIX__AUTO_DETECT=false must keep the workspace-wide check out of the turn"
        );
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(
            cap_rx.try_recv().is_err(),
            "turn-done must NOT push while a client is connected"
        );

        // (5) Client gone → the next completed turn pushes "done".
        handle
            .ws_clients
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        handle
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "quick check".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        let (headers, sealed) =
            tokio::time::timeout(std::time::Duration::from_secs(60), cap_rx.recv())
                .await
                .expect("a done push must arrive once no client is attached")
                .unwrap();
        assert_eq!(
            headers.get("ttl").unwrap().to_str().unwrap(),
            "3600",
            "completion TTL"
        );
        let payload = crate::push::decrypt_payload(&ua_secret, &auth, &sealed).unwrap();
        let p: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(p["kind"], "done");

        // (6) Unsubscribe forgets the row.
        let resp = router
            .clone()
            .oneshot(post_json(
                "/api/push/unsubscribe",
                serde_json::json!({ "endpoint": endpoint }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(store.list_push_subscriptions().unwrap().is_empty());

        handle.shutdown();
        std::env::remove_var("FORGE_AUTOFIX__AUTO_DETECT");
        std::env::remove_var("FORGE_PERMISSION_MODE");
        match old_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fleet_rows_sort_waiting_first_and_carry_the_v7_fields() {
        let mk = |id: &str, waiting: bool, created_at: i64| SessionRow {
            id: id.into(),
            title: String::new(),
            cwd: "/w".into(),
            worktree: None,
            busy: !waiting,
            waiting,
            cost_usd: 0.5,
            context_tokens: 18_200,
            context_limit: Some(200_000),
            model: "m".into(),
            created_at,
            last_activity: created_at,
        };
        let mut rows = vec![
            mk("new-idle", false, 30),
            mk("old-wait", true, 10),
            mk("mid", false, 20),
        ];
        sort_session_rows(&mut rows);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["old-wait", "new-idle", "mid"],
            "waiting-on-decision beats recency; the rest stay newest-first"
        );
        // Same-second creations (created_at has second granularity) must order deterministically
        // — by id — so the dashboard doesn't shuffle rows between polls with map iteration order.
        let mut ties = vec![
            mk("bbb", false, 30),
            mk("aaa", false, 30),
            mk("ccc", false, 30),
        ];
        sort_session_rows(&mut ties);
        assert_eq!(
            ties.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["aaa", "bbb", "ccc"],
            "created-at ties break by id, stably"
        );
        // The wire shape carries the fleet fields the dashboard reads.
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&rows[0]).unwrap()).unwrap();
        assert_eq!(v["waiting"], true);
        assert_eq!(v["context_tokens"], 18_200);
        assert_eq!(v["context_limit"], 200_000);
        assert_eq!(v["cost_usd"], 0.5);
        assert_eq!(v["last_activity"], 10);
    }

    /// The whole upload promise over a REAL driver + the REAL router: a multipart POST with a
    /// hostile filename lands sanitized inside the session's `.forge/uploads/<id>/` scratch
    /// area, the driver attaches it, and the NEXT prompt carries the file's content (`@path`
    /// expansion) — plus an image upload arming vision input, and the reject paths (unknown
    /// session, non-UTF-8 non-image).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_stores_sanitized_and_rides_the_next_prompt() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-upload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("upload-test.db"));

        let registry = Arc::new(SessionRegistry::new());
        let handle = registry
            .insert(
                spawn_session_driver(DriverSpec {
                    cwd: dir.display().to_string(),
                    worktree: None,
                    title: "upload-e2e".into(),
                    mock: true,
                    model: None,
                    resume: None,
                    temper: None,
                    push: None,
                    apns: None,
                })
                .await
                .unwrap(),
            )
            .await;
        let state = Arc::new(DaemonState {
            registry: registry.clone(),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = daemon_router(state);
        let multipart = |session: &str, filename: &str, ctype: &str, body: &[u8]| {
            let b = "XFORGEBOUNDARY";
            let mut payload = format!(
                "--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
                 Content-Type: {ctype}\r\n\r\n"
            )
            .into_bytes();
            payload.extend_from_slice(body);
            payload.extend_from_slice(format!("\r\n--{b}--\r\n").as_bytes());
            axum::http::Request::post(format!("/tok/api/upload?session={session}"))
                .header("content-type", format!("multipart/form-data; boundary={b}"))
                .body(axum::body::Body::from(payload))
                .unwrap()
        };

        // Unknown session → 404 (session-scoped like every other route).
        let resp = router
            .clone()
            .oneshot(multipart("ghost", "a.txt", "text/plain", b"x"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);

        // A traversal-shaped filename stores FLATTENED inside the session's upload dir.
        let resp = router
            .clone()
            .oneshot(multipart(
                &handle.session_id,
                "../../escape attempt.txt",
                "text/plain",
                b"upload-marker-content",
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1 << 16)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let stored = std::path::PathBuf::from(v["files"][0]["path"].as_str().unwrap());
        let updir = dir
            .join(".forge")
            .join("uploads")
            .join(remote::sanitize_upload_name(&handle.session_id));
        assert!(
            stored.starts_with(&updir),
            "stored inside the session scratch area: {stored:?}"
        );
        assert!(stored
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-escape_attempt.txt"));
        assert_eq!(v["files"][0]["image"], false);
        assert_eq!(
            std::fs::read(&stored).unwrap(),
            b"upload-marker-content",
            "content stored verbatim"
        );

        // …and an image upload alongside it (fake bytes are fine: images aren't UTF-8-gated).
        let resp = router
            .clone()
            .oneshot(multipart(
                &handle.session_id,
                "shot.png",
                "image/png",
                &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a],
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // The driver noted both attaches (📎 text mention + 🖼 vision input).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let s = handle.snapshot_rx.borrow().snapshot.clone();
            let t = s.transcript.join("\n");
            if t.contains("📎 attached") && t.contains("🖼 image attached") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "attach notes never appeared; transcript: {t}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }

        // The NEXT prompt rides with the text file @-mentioned — its content reaches the turn.
        handle
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "use the attached notes".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let s = handle.snapshot_rx.borrow().snapshot.clone();
            let t = s.transcript.join("\n");
            if !s.busy && t.contains("📎 included") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the uploaded file was never included; transcript: {t}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }

        // Reject path: a non-image, non-UTF-8 body has no injection path → 422.
        let resp = router
            .clone()
            .oneshot(multipart(
                &handle.session_id,
                "prog.bin",
                "application/octet-stream",
                &[0x00, 0xff, 0xfe],
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);

        handle.shutdown();
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The plan card end to end over a REAL driver (mock provider): `/plan` → the snapshot
    /// carries `plan` + the approval question whose option 1 is "Build it" — and a remote
    /// Answer("1") (exactly what the page's Approve button sends) approves it, switching the
    /// temper and running the build turn, identical to a local choice.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn plan_card_projects_and_remote_approve_builds() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("forge-serve-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("FORGE_DB", dir.join("plan-test.db"));

        let handle = spawn_session_driver(DriverSpec {
            cwd: dir.display().to_string(),
            worktree: None,
            title: "plan-e2e".into(),
            mock: true,
            model: None,
            resume: None,
            temper: None,
            push: None,
            apns: None,
        })
        .await
        .unwrap();
        handle
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "/plan mock:plan the feature".into(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();

        // The proposal lands in the snapshot together with its approval question.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let pending = loop {
            let s = handle.snapshot_rx.borrow().snapshot.clone();
            if s.plan.is_some() && s.question.is_some() {
                break s;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "plan card + approval question never appeared; snapshot: {s:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        };
        let plan = pending.plan.as_ref().unwrap();
        assert!(!plan.title.is_empty());
        assert!(!plan.steps.is_empty(), "steps rode the wire: {plan:?}");
        assert_eq!(
            pending.question_options.first().map(|o| o.label.as_str()),
            Some("Build it"),
            "the page's Approve button answers THIS option by number"
        );

        // Approve exactly like the page does: Answer("1") with the current seq.
        handle
            .input_tx
            .send(remote::RemoteInput::Answer {
                text: "1".into(),
                seq: pending.prompt_seq,
            })
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let s = handle.snapshot_rx.borrow().snapshot.clone();
            let t = s.transcript.join("\n");
            if !s.busy && t.contains("plan approved — building in Auto-edit") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "approval never built; transcript: {t}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }

        handle.shutdown();
        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_token_rejects_a_corrupted_file() {
        let dir = std::env::temp_dir().join(format!("forge-serve-token2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("serve-token");
        std::fs::write(&path, "not hex at all!!").unwrap();
        let t = daemon_token_at(&path, false).unwrap();
        assert_eq!(t.len(), 32, "corrupted token is replaced, not trusted");
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
