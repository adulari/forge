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
//! - `GET  /api/sessions`             running sessions (id, title, cwd, busy, cost, activity)
//! - `POST /api/sessions`             create ({cwd, worktree, title?, model?, resume?})
//! - `POST /api/sessions/{id}/archive` stop + hide a session (history kept; worktree kept)
//! - `GET  /api/history?session=<id>&before=<seq>&limit=<n>`  scrollback pagination
//!
//! Exposure mirrors `/remote`: `--lan` (default) binds 0.0.0.0 with self-signed HTTPS, `--local`
//! binds loopback plain HTTP, `--anywhere` binds loopback and opens a cloudflared/ngrok tunnel.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path as AxumPath, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;

use crate::cli::commands::run::{spawn_session_driver, DriverSpec, SessionDriverHandle};
use crate::remote;

/// How long an archive waits for the driver task to wind down before letting go.
const ARCHIVE_JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The persisted daemon token's file name inside the forge config dir.
const TOKEN_FILE: &str = "serve-token";

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

/// The daemon's session registry: id → running driver handle. Mirrors `mcp_serve`'s
/// LocalSessionManager pattern (one task per session, addressed by id).
pub(crate) struct SessionRegistry {
    sessions: tokio::sync::Mutex<std::collections::HashMap<String, Arc<SessionDriverHandle>>>,
}

impl SessionRegistry {
    pub(crate) fn new() -> Self {
        Self {
            sessions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub(crate) async fn insert(&self, handle: SessionDriverHandle) -> Arc<SessionDriverHandle> {
        let handle = Arc::new(handle);
        self.sessions
            .lock()
            .await
            .insert(handle.session_id.clone(), handle.clone());
        handle
    }

    pub(crate) async fn get(&self, id: &str) -> Option<Arc<SessionDriverHandle>> {
        self.sessions.lock().await.get(id).cloned()
    }

    pub(crate) async fn remove(&self, id: &str) -> Option<Arc<SessionDriverHandle>> {
        self.sessions.lock().await.remove(id)
    }

    pub(crate) async fn all(&self) -> Vec<Arc<SessionDriverHandle>> {
        self.sessions.lock().await.values().cloned().collect()
    }
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
}

/// One row of `GET /api/sessions`.
#[derive(serde::Serialize)]
struct SessionRow {
    id: String,
    title: String,
    cwd: String,
    worktree: Option<String>,
    busy: bool,
    cost_usd: f64,
    model: String,
    created_at: i64,
    last_activity: i64,
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
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let base = format!("/{token}");
    let state = Arc::new(DaemonState {
        registry: registry.clone(),
        store,
        base: base.clone(),
        mock,
        default_cwd,
    });

    let app = Router::new()
        .route(&base, get(page))
        .route(&format!("{base}/"), get(page))
        .route(&format!("{base}/ws"), get(ws_handler))
        .route(
            &format!("{base}/api/sessions"),
            get(list_sessions).post(create_session),
        )
        .route(
            &format!("{base}/api/sessions/{{id}}/archive"),
            post(archive_session),
        )
        .route(&format!("{base}/api/history"), get(history_page))
        .route(&format!("{base}/app.js"), get(app_js))
        .route(&format!("{base}/styles.css"), get(styles_css))
        .route(&format!("{base}/manifest.webmanifest"), get(manifest))
        .route(&format!("{base}/sw.js"), get(service_worker))
        .route(&format!("{base}/icon.svg"), get(icon))
        .fallback(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response() })
        .with_state(state);

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
        let kind = remote::detect_tunnel().ok_or_else(|| {
            anyhow::anyhow!(
                "no tunnel tool found on PATH — install cloudflared or ngrok for --anywhere"
            )
        })?;
        println!("⚒ opening a public tunnel via {} …", kind.label());
        let (public, child) = remote::spawn_tunnel(kind, addr.port()).await?;
        tunnel_child = Some(child);
        format!("{}/{token}", public.trim_end_matches('/'))
    } else if local {
        format!("http://127.0.0.1:{}/{token}", addr.port())
    } else {
        let host = remote::lan_display_host(config.remote.host.as_deref(), addr);
        format!("https://{host}:{}/{token}", addr.port())
    };

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
    tokio::select! {
        r = server => r?,
        _ = tokio::signal::ctrl_c() => {
            println!("\n⚒ shutting down — stopping sessions…");
            for handle in registry.all().await {
                handle.shutdown();
            }
            // Bounded: a wedged driver must not hold the daemon's exit hostage.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }
    drop(tunnel_child); // kill_on_drop tears the tunnel down with the daemon
    Ok(())
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
        let snap = h.snapshot_rx.borrow().clone();
        rows.push(SessionRow {
            id: h.session_id.clone(),
            title: h.title.clone(),
            cwd: h.cwd.clone(),
            worktree: h.worktree.clone(),
            busy: snap.busy,
            cost_usd: snap.cost_usd,
            model: snap.model,
            created_at: h.created_at,
            last_activity: h.last_activity.load(std::sync::atomic::Ordering::Relaxed),
        });
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    json_response(&rows)
}

/// `POST /api/sessions` — create (optionally in a fresh isolated worktree) and start driving.
async fn create_session(
    State(state): State<Arc<DaemonState>>,
    axum::Json(req): axum::Json<CreateSessionReq>,
) -> Response {
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

    // Worktree isolation: branch from HEAD of `cwd` into .forge/worktrees/<id> — the audited
    // WorktreeGuard creation, WITHOUT its drop-side removal: a daemon session's worktree must
    // outlive the handle (and the daemon), so the guard is intentionally leaked and the path
    // persisted on the session row instead. Archiving snapshots uncommitted edits onto the
    // branch and leaves both in place for a manual merge.
    let mut worktree: Option<String> = None;
    if req.worktree {
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
                std::mem::forget(guard); // persistence over RAII — see comment above
            }
            Err(e) => {
                return err_response(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("worktree create failed: {e}"),
                );
            }
        }
    }

    let session_cwd = worktree.clone().unwrap_or_else(|| cwd.clone());
    let spec = DriverSpec {
        cwd: session_cwd,
        worktree: worktree.clone(),
        title: req.title.unwrap_or_default(),
        mock: state.mock,
        model: req.model,
        resume: req.resume,
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
        Err(e) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("session create failed: {e}"),
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
    if let Some(wt) = &handle.worktree {
        // Best-effort snapshot: uncommitted edits land on the session's branch so nothing is
        // lost; the worktree + branch stay in place for a manual review/merge.
        let _ = forge_core::worktree::commit_worktree(std::path::Path::new(wt));
    }
    let _ = state.store.archive_session(&id);
    if let Ok(h) = Arc::try_unwrap(handle) {
        h.join(ARCHIVE_JOIN_TIMEOUT).await;
    }
    json_response(&serde_json::json!({ "ok": true }))
}

/// Query for the per-session WS handshake.
#[derive(serde::Deserialize)]
struct WsParams {
    #[serde(default)]
    session: String,
    #[serde(default)]
    rev: u64,
}

async fn ws_handler(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(handle) = state.registry.get(&params.session).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let snapshot_rx = handle.snapshot_rx.clone();
    let events = handle.events.clone();
    let input_tx = handle.input_tx.clone();
    ws.on_upgrade(move |socket| remote::pump_ws(socket, snapshot_rx, events, input_tx, params.rev))
}

/// Query for `GET /api/history` — Phase 3's route plus the session address.
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

#[cfg(test)]
mod tests {
    use super::*;

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
            })
            .await
            .unwrap();
        b.input_tx
            .send(remote::RemoteInput::Prompt {
                text: "beta-marker task".into(),
            })
            .await
            .unwrap();

        async fn wait_done(h: &SessionDriverHandle, needle: &str) -> remote::Snapshot {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let s = h.snapshot_rx.borrow().clone();
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
            replayed.iter().any(|s| s.revision == sa2.revision),
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
        // The archived driver's last frame tells clients to stop reconnecting.
        assert!(b.snapshot_rx.borrow().closed, "final frame is closed");

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
        })
        .await
        .unwrap();
        assert!(std::path::Path::new(&wt_path).join(".git").exists());
        handle
            .input_tx
            .send(remote::RemoteInput::Prompt {
                text: "hello worktree".into(),
            })
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let snap = loop {
            let s = handle.snapshot_rx.borrow().clone();
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
