//! Request gate for `forge serve`: resolves which client a token belongs to, enforces its scopes,
//! checks the `Host` header, writes the remote audit log and serves the Remote Access routes.
//!
//! The daemon's routes are registered under the owner token (`/<owner>/...`). A paired device's
//! own token is swapped for the owner prefix here, before routing, after its scopes have been
//! checked, so no handler ever sees a second path shape and the owner token never reaches a
//! client.

use crate::serve::DaemonState;
use crate::serve_clients::{
    ClientAuth, ClientRecord, ClientRegistry, Preset, Scope, DEFAULT_PAIR_TTL_SECS,
};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, Method, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

const AUDIT_FILE: &str = "remote-audit.jsonl";
const AUDIT_ROTATE_BYTES: u64 = 8 * 1024 * 1024;

const BUILTIN_HOST_SUFFIXES: [&str; 9] = [
    ".local",
    ".localhost",
    ".lan",
    ".ts.net",
    ".trycloudflare.com",
    ".ngrok-free.app",
    ".ngrok-free.dev",
    ".ngrok.app",
    ".ngrok.io",
];

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Exposure {
    pub(crate) mode: String,
    pub(crate) port: u16,
    pub(crate) lan_host: Option<String>,
    pub(crate) public_url: Option<String>,
}

pub(crate) struct Access {
    pub(crate) clients: ClientRegistry,
    audit: Option<AuditLog>,
    extra_hosts: Vec<String>,
    trusted_proxies: Vec<IpAddr>,
    exposure: Mutex<Exposure>,
}

static ACCESS: RwLock<Option<Arc<Access>>> = RwLock::new(None);

pub(crate) fn access() -> Arc<Access> {
    if let Some(access) = ACCESS.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return access;
    }
    let mut slot = ACCESS.write().unwrap_or_else(|e| e.into_inner());
    slot.get_or_insert_with(|| Arc::new(Access::new(None, None, &[], &[])))
        .clone()
}

pub(crate) fn install(access: Access) {
    *ACCESS.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(access));
}

/// Build the daemon's access state from `[remote]` config and the real config/data dirs.
pub(crate) fn init_from_config(config: &forge_config::Config) {
    let mut hosts = config.remote.allowed_hosts.clone();
    hosts.extend(config.remote.tunnel_hostname.clone());
    hosts.extend(config.remote.host.clone());
    let audit = forge_config::data_dir().map(|d| d.join(AUDIT_FILE));
    install(Access::new(
        ClientRegistry::default_path(),
        audit,
        &hosts,
        &config.remote.trusted_proxies,
    ));
}

impl Access {
    pub(crate) fn new(
        clients_path: Option<PathBuf>,
        audit_path: Option<PathBuf>,
        hosts: &[String],
        proxies: &[String],
    ) -> Self {
        Self {
            clients: ClientRegistry::open(clients_path),
            audit: audit_path.map(AuditLog::new),
            extra_hosts: hosts
                .iter()
                .map(|h| h.trim().to_ascii_lowercase())
                .collect(),
            trusted_proxies: proxies
                .iter()
                .filter_map(|p| p.trim().parse().ok())
                .collect(),
            exposure: Mutex::new(Exposure::default()),
        }
    }

    pub(crate) fn set_exposure(&self, exposure: Exposure) {
        *self.exposure.lock().unwrap_or_else(|e| e.into_inner()) = exposure;
    }

    fn exposure(&self) -> Exposure {
        self.exposure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn host_allowed(&self, header_value: &str) -> bool {
        if host_is_builtin_safe(header_value) {
            return true;
        }
        let host = strip_port(header_value);
        if let Some(public) = self.exposure().public_url.as_deref().and_then(url_host) {
            if public == host {
                return true;
            }
        }
        self.extra_hosts
            .iter()
            .any(|allowed| match allowed.strip_prefix("*.") {
                Some(suffix) => host.ends_with(&format!(".{suffix}")),
                None => strip_port(allowed) == host,
            })
    }

    pub(crate) fn record(&self, entry: AuditEntry<'_>) {
        if let Some(log) = &self.audit {
            log.append(&entry);
        }
    }

    fn client_ip(&self, peer: Option<IpAddr>, forwarded: Option<&str>) -> Option<IpAddr> {
        client_ip(peer, forwarded, &self.trusted_proxies)
    }
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split('/').next()?;
    Some(strip_port(authority))
}

fn strip_port(authority: &str) -> String {
    let authority = authority.trim().to_ascii_lowercase();
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host.to_string(),
        _ => authority,
    }
}

/// Hosts that cannot be a DNS-rebinding vector: addresses, loopback names, single-label names
/// and the suffixes only an operator's own network or tunnel agent can resolve.
pub(crate) fn host_is_builtin_safe(header_value: &str) -> bool {
    let host = strip_port(header_value);
    if host.is_empty() || host.parse::<IpAddr>().is_ok() || host == "localhost" {
        return true;
    }
    if !host.contains('.') {
        return true;
    }
    BUILTIN_HOST_SUFFIXES
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

pub(crate) fn client_ip(
    peer: Option<IpAddr>,
    forwarded: Option<&str>,
    trusted: &[IpAddr],
) -> Option<IpAddr> {
    let peer = peer?;
    let is_trusted = |ip: &IpAddr| ip.is_loopback() || trusted.contains(ip);
    if !is_trusted(&peer) {
        return Some(peer);
    }
    let Some(forwarded) = forwarded else {
        return Some(peer);
    };
    for hop in forwarded.rsplit(',') {
        match hop.trim().parse::<IpAddr>() {
            Ok(ip) if is_trusted(&ip) => continue,
            Ok(ip) => return Some(ip),
            Err(_) => return Some(peer),
        }
    }
    Some(peer)
}

pub(crate) enum Need {
    Scope(Scope),
    Owner,
}

/// The scope a request needs, from its method and its path below the token.
pub(crate) fn required(method: &Method, path: &str) -> Need {
    let read = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if path.starts_with("/api/clients") || path == "/api/remote-access" {
        return Need::Owner;
    }
    if matches!(
        path,
        "" | "/" | "/app.js" | "/styles.css" | "/manifest.webmanifest" | "/sw.js" | "/icon.svg"
    ) {
        return Need::Owner;
    }
    if path == "/ws/terminal" || path.starts_with("/api/terminals") {
        return Need::Scope(Scope::Terminal);
    }
    if path.starts_with("/api/providers")
        || path.starts_with("/api/onboarding")
        || path == "/api/anywhere/enable"
        || (!read && matches!(path, "/api/update" | "/api/config" | "/api/mcp"))
    {
        return Need::Scope(Scope::ManageProviders);
    }
    if path == "/api/answer" || (path.starts_with("/api/dispatches/") && path.ends_with("/approve"))
    {
        return Need::Scope(Scope::Approve);
    }
    if read || path.starts_with("/api/push/") || path == "/api/ws-ticket" {
        return Need::Scope(Scope::View);
    }
    if ["/api/git/", "/api/pr/", "/api/actions/", "/api/workspace/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Need::Scope(Scope::WorkspaceWrite);
    }
    Need::Scope(Scope::Prompt)
}

fn refusal(scope: &str, what: &str) -> Response {
    let body = serde_json::json!({
        "error": format!("this connection cannot {what}"),
        "code": "scope_denied",
        "scope": scope,
    });
    (StatusCode::FORBIDDEN, Json(body)).into_response()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

fn session_of(path: &str, query: &str) -> Option<String> {
    if let Some(id) = query_param(query, "session") {
        return Some(id);
    }
    path.strip_prefix("/api/sessions/")
        .and_then(|rest| rest.split('/').next())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn is_ws_path(path: &str) -> bool {
    matches!(path, "/ws" | "/ws/terminal" | "/ws/fleet")
}

pub(crate) async fn gate(
    State(state): State<Arc<DaemonState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let access = access();
    let owner = state.base.trim_start_matches('/').to_string();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let forwarded = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let ip = access
        .client_ip(peer, forwarded.as_deref())
        .map(|ip| ip.to_string());

    let trimmed = path.trim_start_matches('/');
    let (segment, rest) = match trimmed.split_once('/') {
        Some((segment, rest)) => (segment, format!("/{rest}")),
        None => (trimmed, String::new()),
    };

    let auth = if segment == owner {
        Some(ClientAuth::owner())
    } else if crate::serve_clients::looks_like_client_token(segment) {
        access.clients.authenticate(segment)
    } else if is_ws_path(&path) {
        query_param(&query, "ticket").and_then(|t| access.clients.redeem_ticket(&t))
    } else {
        None
    };
    let Some(auth) = auth else {
        return next.run(req).await;
    };
    let (route_path, via_ticket) =
        if segment == owner || crate::serve_clients::looks_like_client_token(segment) {
            (rest, false)
        } else {
            (path.clone(), true)
        };

    let session = session_of(&route_path, &query);
    let deny = |decision: &str, response: Response| {
        access.record(AuditEntry {
            client: auth.name(),
            label: &auth.label,
            route: format!("{method} {route_path}"),
            session: session.clone(),
            decision,
            detail: None,
            status: Some(response.status().as_u16()),
            ip: ip.clone(),
        });
        response
    };

    if let Some(host) = host.as_deref() {
        if !access.host_allowed(host) {
            let body = serde_json::json!({
                "error": "host header is not allowed for this daemon",
                "code": "host_not_allowed",
            });
            return deny(
                "deny:host",
                (StatusCode::FORBIDDEN, Json(body)).into_response(),
            );
        }
    }

    match required(&method, &route_path) {
        Need::Owner if !auth.is_owner() => {
            return deny(
                "deny:owner-only",
                refusal("owner", "manage devices or open the legacy page"),
            );
        }
        Need::Scope(scope) if !auth.allows(scope) => {
            return deny(
                &format!("deny:{}", scope.as_str()),
                refusal(scope.as_str(), scope_action(scope)),
            );
        }
        _ => {}
    }

    let audited = method != Method::GET || is_ws_path(&route_path);
    let label = auth.label.clone();
    let name = auth.name().to_string();
    if auth.id.is_some() || via_ticket {
        let rewritten = format!(
            "/{owner}{route_path}{}",
            if query.is_empty() {
                String::new()
            } else {
                format!("?{query}")
            }
        );
        if let Ok(path_and_query) = rewritten.parse() {
            let mut parts = req.uri().clone().into_parts();
            parts.path_and_query = Some(path_and_query);
            if let Ok(uri) = Uri::from_parts(parts) {
                *req.uri_mut() = uri;
            }
        }
    }
    req.extensions_mut().insert(auth);
    let response = next.run(req).await;
    if audited {
        access.record(AuditEntry {
            client: &name,
            label: &label,
            route: format!("{method} {route_path}"),
            session,
            decision: "allow",
            detail: None,
            status: Some(response.status().as_u16()),
            ip,
        });
    }
    response
}

fn scope_action(scope: Scope) -> &'static str {
    match scope {
        Scope::View => "view this host",
        Scope::Prompt => "send prompts or change sessions",
        Scope::Approve => "approve or deny requests",
        Scope::Terminal => "use the terminal",
        Scope::WorkspaceWrite => "change files, git or pull requests",
        Scope::ManageProviders => "change providers, configuration or updates",
    }
}

#[derive(Serialize)]
pub(crate) struct AuditEntry<'a> {
    pub(crate) client: &'a str,
    pub(crate) label: &'a str,
    pub(crate) route: String,
    pub(crate) session: Option<String>,
    pub(crate) decision: &'a str,
    pub(crate) detail: Option<&'a str>,
    pub(crate) status: Option<u16>,
    pub(crate) ip: Option<String>,
}

struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn append(&self, entry: &AuditEntry<'_>) {
        use std::io::Write as _;
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut value = match serde_json::to_value(entry) {
            Ok(value) => value,
            Err(_) => return,
        };
        value["ts"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() >= AUDIT_ROTATE_BYTES) {
            let _ = std::fs::rename(&self.path, self.path.with_extension("jsonl.1"));
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        if let Ok(mut file) = options.open(&self.path) {
            let _ = writeln!(file, "{value}");
        }
    }
}

/// Aborts spawned socket tasks when the future that owns them is dropped, so a revoked client's
/// connection closes instead of lingering in detached tasks.
pub(crate) struct AbortOnDrop(pub(crate) Vec<tokio::task::AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// Filters inbound WebSocket frames for one connection: the scope each input kind needs, and an
/// audit line for every approval.
pub(crate) struct InputGate {
    auth: ClientAuth,
    session: String,
}

#[derive(Deserialize)]
struct FrameKind {
    kind: String,
    #[serde(default)]
    yes: Option<bool>,
}

pub(crate) fn input_scope(kind: &str) -> Scope {
    match kind {
        "allow" | "answer" => Scope::Approve,
        _ => Scope::Prompt,
    }
}

impl InputGate {
    pub(crate) fn new(auth: ClientAuth, session: &str) -> Self {
        Self {
            auth,
            session: session.to_string(),
        }
    }

    /// `Some(reason)` when the frame must be dropped; the reason is shown to the client.
    pub(crate) fn check(&self, frame: &str) -> Option<String> {
        let kind = serde_json::from_str::<FrameKind>(frame).ok()?;
        let scope = input_scope(&kind.kind);
        let allowed = self.auth.allows(scope);
        if scope == Scope::Approve || !allowed {
            let detail = kind.yes.map(|yes| if yes { "approve" } else { "deny" });
            access().record(AuditEntry {
                client: self.auth.name(),
                label: &self.auth.label,
                route: format!("WS input {}", kind.kind),
                session: Some(self.session.clone()),
                decision: if allowed { "allow" } else { "deny" },
                detail,
                status: None,
                ip: None,
            });
        }
        (!allowed).then(|| format!("scope_denied:{}", scope.as_str()))
    }
}

#[derive(Serialize)]
struct ClientView {
    id: String,
    label: String,
    scopes: Vec<Scope>,
    created_at: u64,
    last_seen: Option<u64>,
    pending: bool,
    preset: &'static str,
}

fn view(record: &ClientRecord) -> ClientView {
    let viewer = record.scopes == Preset::Viewer.scopes();
    ClientView {
        id: record.id.clone(),
        label: record.label.clone(),
        scopes: record.scopes.clone(),
        created_at: record.created_at,
        last_seen: record.last_seen,
        pending: record.pending_until.is_some(),
        preset: if viewer { "viewer" } else { "full" },
    }
}

#[derive(Serialize)]
struct AddressView {
    kind: &'static str,
    base: Option<String>,
    note: String,
}

fn tailscale_ip() -> Option<String> {
    let out = std::process::Command::new("tailscale")
        .args(["ip", "-4"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .map(str::to_string)
        })
        .flatten()
        .filter(|ip| ip.parse::<IpAddr>().is_ok())
}

fn addresses(exposure: &Exposure, tailscale: Option<String>) -> Vec<AddressView> {
    let port = exposure.port;
    let lan_reachable = exposure.mode == "lan";
    let local = AddressView {
        kind: "local",
        base: Some(if lan_reachable {
            format!("https://localhost:{port}")
        } else {
            format!("http://127.0.0.1:{port}")
        }),
        note: "This computer only.".into(),
    };
    let lan = match (&exposure.lan_host, lan_reachable) {
        (Some(host), true) => AddressView {
            kind: "lan",
            base: Some(format!("https://{host}:{port}")),
            note: "Same Wi-Fi or network. Self-signed certificate.".into(),
        },
        _ => AddressView {
            kind: "lan",
            base: None,
            note: "Not exposed: the daemon listens on loopback. Start it with `forge serve --lan`."
                .into(),
        },
    };
    let tailscale = match (tailscale, lan_reachable) {
        (Some(ip), true) => AddressView {
            kind: "tailscale",
            base: Some(format!("https://{ip}:{port}")),
            note: "Reachable from your tailnet. Self-signed certificate.".into(),
        },
        (Some(_), false) => AddressView {
            kind: "tailscale",
            base: None,
            note:
                "Tailscale is up, but the daemon is on loopback. Start it with `forge serve --lan`."
                    .into(),
        },
        (None, _) => AddressView {
            kind: "tailscale",
            base: None,
            note: "Tailscale was not detected on this machine.".into(),
        },
    };
    let public = match &exposure.public_url {
        Some(url) => AddressView {
            kind: "public",
            base: Some(url.trim_end_matches('/').to_string()),
            note: "Reachable from anywhere. Anyone holding a link can use it until revoked.".into(),
        },
        None => AddressView {
            kind: "public",
            base: None,
            note: "No public tunnel. Start one with `forge serve --tunnel`.".into(),
        },
    };
    vec![local, lan, tailscale, public]
}

#[derive(Serialize)]
struct RemoteAccessView {
    online: bool,
    mode: String,
    addresses: Vec<AddressView>,
    clients: Vec<ClientView>,
}

async fn remote_access() -> Json<RemoteAccessView> {
    let access = access();
    let exposure = access.exposure();
    let tailscale = tokio::task::spawn_blocking(tailscale_ip)
        .await
        .ok()
        .flatten();
    Json(RemoteAccessView {
        online: true,
        mode: exposure.mode.clone(),
        addresses: addresses(&exposure, tailscale),
        clients: access.clients.list().iter().map(view).collect(),
    })
}

#[derive(Deserialize, Default)]
struct PairRequest {
    #[serde(default)]
    label: String,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    ttl_secs: Option<u64>,
}

#[derive(Serialize)]
struct PairResponse {
    client: ClientView,
    token: String,
    expires_in: u64,
}

async fn pair(Extension(auth): Extension<ClientAuth>, body: Option<Json<PairRequest>>) -> Response {
    let request = body.map(|Json(b)| b).unwrap_or_default();
    let preset = match request.preset.as_deref() {
        None => Preset::Full,
        Some(text) => match Preset::parse(text) {
            Some(preset) => preset,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "preset must be `full` or `viewer`" })),
                )
                    .into_response()
            }
        },
    };
    let ttl = request.ttl_secs.unwrap_or(DEFAULT_PAIR_TTL_SECS);
    let access = access();
    match access
        .clients
        .mint(&request.label, preset.scopes(), Some(ttl))
    {
        Ok((record, token)) => {
            access.record(AuditEntry {
                client: auth.name(),
                label: &auth.label,
                route: "POST /api/clients/pair".into(),
                session: None,
                decision: "allow",
                detail: Some(&record.id),
                status: Some(200),
                ip: None,
            });
            Json(PairResponse {
                client: view(&record),
                token,
                expires_in: ttl.clamp(30, crate::serve_clients::MAX_PAIR_TTL_SECS),
            })
            .into_response()
        }
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn revoke(
    Extension(auth): Extension<ClientAuth>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let access = access();
    match access.clients.revoke(&id) {
        Ok(record) => {
            access.record(AuditEntry {
                client: auth.name(),
                label: &auth.label,
                route: "DELETE /api/clients".into(),
                session: None,
                decision: "allow",
                detail: Some(&record.id),
                status: Some(200),
                ip: None,
            });
            Json(serde_json::json!({ "ok": true, "id": record.id })).into_response()
        }
        Err(error) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn ws_ticket(Extension(auth): Extension<ClientAuth>) -> Json<serde_json::Value> {
    let ticket = access().clients.issue_ticket(&auth);
    Json(serde_json::json!({ "ticket": ticket, "expires_in": 30 }))
}

async fn whoami(Extension(auth): Extension<ClientAuth>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "id": auth.id,
        "label": auth.label,
        "owner": auth.is_owner(),
        "scopes": auth.scopes,
    }))
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/remote-access"), get(remote_access))
        .route(&format!("{base}/api/clients/pair"), post(pair))
        .route(&format!("{base}/api/clients/{{id}}"), delete(revoke))
        .route(&format!("{base}/api/ws-ticket"), post(ws_ticket))
        .route(&format!("{base}/api/whoami"), get(whoami))
}

/// Wrap a freshly built daemon router so the gate runs before it routes.
pub(crate) fn guard(state: Arc<DaemonState>, inner: Router) -> Router {
    Router::new()
        .fallback_service(inner)
        .layer(axum::middleware::from_fn_with_state(state, gate))
}

#[cfg(test)]
#[path = "serve_access_tests.rs"]
mod tests;
