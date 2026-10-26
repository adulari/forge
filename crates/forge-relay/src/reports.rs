//! `POST /v1/reports` — receives scrubbed issue reports from Forge clients, validates them against
//! a closed schema, re-scrubs every free-text field, rate-limits, records counts in SQLite and
//! hands the GitHub issue work to a background worker (`reports_sync.rs`). The route answers 202
//! as soon as the report is durably counted; GitHub being down never fails or slows a client.

use crate::github::{GitHub, Secret};
use crate::reports_db::{NewReport, ReportsDb};
use crate::reports_scrub::scrub_to;
use crate::AppState;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use governor::{Quota, RateLimiter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::hash::{BuildHasher, RandomState};
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const MAX_BODY_BYTES: usize = 32 * 1024;

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub(crate) struct ReportsConfig {
    pub(crate) token: Option<Secret>,
    pub(crate) repo: String,
    pub(crate) api_base: String,
    pub(crate) db_path: String,
    pub(crate) disabled: bool,
    pub(crate) per_install_per_hour: u32,
    pub(crate) per_ip_per_hour: u32,
    pub(crate) global_per_hour: u32,
    /// Minimum seconds between GitHub writes for one problem (coalesces bursts into one edit).
    pub(crate) min_update_secs: i64,
}

impl std::fmt::Debug for ReportsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportsConfig")
            .field("token", &self.token)
            .field("repo", &self.repo)
            .field("api_base", &self.api_base)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

fn env_u32(name: &str, default: u32) -> anyhow::Result<u32> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(raw) => raw
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| anyhow::anyhow!("{name} must be a positive integer")),
    }
}

impl ReportsConfig {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let repo =
            std::env::var("FORGE_RELAY_GITHUB_REPO").unwrap_or_else(|_| "Adulari/forge".into());
        anyhow::ensure!(
            repo.split('/').count() == 2
                && repo
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.')),
            "FORGE_RELAY_GITHUB_REPO must look like owner/name"
        );
        let disabled = matches!(
            std::env::var("FORGE_RELAY_REPORTS_DISABLED")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes"
        );
        Ok(Self {
            token: std::env::var("FORGE_RELAY_GITHUB_TOKEN")
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .map(Secret::new),
            repo,
            api_base: std::env::var("FORGE_RELAY_GITHUB_API")
                .unwrap_or_else(|_| "https://api.github.com".into()),
            db_path: std::env::var("FORGE_RELAY_REPORTS_DB")
                .unwrap_or_else(|_| "./reports.sqlite".into()),
            disabled,
            per_install_per_hour: env_u32("FORGE_RELAY_REPORTS_PER_INSTALL_PER_HOUR", 30)?,
            per_ip_per_hour: env_u32("FORGE_RELAY_REPORTS_PER_IP_PER_HOUR", 120)?,
            global_per_hour: env_u32("FORGE_RELAY_REPORTS_GLOBAL_PER_HOUR", 600)?,
            min_update_secs: i64::from(env_u32("FORGE_RELAY_REPORTS_MIN_UPDATE_SECS", 60)?),
        })
    }
}

type Keyed = RateLimiter<
    u64,
    governor::state::keyed::DefaultKeyedStateStore<u64>,
    governor::clock::DefaultClock,
>;
type Direct = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

fn hourly(n: u32) -> Quota {
    let burst = NonZeroU32::new(n.max(1)).expect("non-zero");
    Quota::with_period(Duration::from_secs(3600) / burst.get())
        .expect("non-zero period")
        .allow_burst(burst)
}

struct Limiters {
    per_install: Keyed,
    per_ip: Keyed,
    global: Direct,
    hasher: RandomState,
}

pub(crate) struct ReportsState {
    pub(crate) cfg: ReportsConfig,
    pub(crate) db: Option<ReportsDb>,
    pub(crate) github: Option<GitHub>,
    pub(crate) sync_lock: tokio::sync::Mutex<()>,
    pub(crate) wake: tokio::sync::Notify,
    limiters: Limiters,
    manual_seq: AtomicU64,
}

impl ReportsState {
    pub(crate) fn from_env() -> anyhow::Result<Arc<Self>> {
        let state = Self::new(ReportsConfig::from_env()?)?;
        if state.enabled() {
            tracing::info!("issue reports enabled (repo {})", state.cfg.repo);
        } else {
            tracing::warn!(
                "issue reports disabled (no FORGE_RELAY_GITHUB_TOKEN or kill switch set)"
            );
        }
        state.spawn_worker();
        Ok(state)
    }

    pub(crate) fn new(cfg: ReportsConfig) -> anyhow::Result<Arc<Self>> {
        let active = !cfg.disabled && cfg.token.is_some();
        let (db, github) = match (&cfg.token, active) {
            (Some(token), true) => (
                Some(ReportsDb::open(std::path::Path::new(&cfg.db_path))?),
                Some(GitHub::new(&cfg.api_base, &cfg.repo, token.clone())?),
            ),
            _ => (None, None),
        };
        let limiters = Limiters {
            per_install: RateLimiter::keyed(hourly(cfg.per_install_per_hour)),
            per_ip: RateLimiter::keyed(hourly(cfg.per_ip_per_hour)),
            global: RateLimiter::direct(hourly(cfg.global_per_hour)),
            hasher: RandomState::new(),
        };
        Ok(Arc::new(Self {
            cfg,
            db,
            github,
            sync_lock: tokio::sync::Mutex::new(()),
            wake: tokio::sync::Notify::new(),
            limiters,
            manual_seq: AtomicU64::new(0),
        }))
    }

    pub(crate) fn enabled(&self) -> bool {
        self.db.is_some()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Source {
    Auto,
    Manual,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Panic,
    Internal,
    ProviderProtocol,
    ToolDispatch,
    Store,
    Compaction,
    TaskDeath,
    RenderPanic,
    JsError,
    JsUnhandledRejection,
    ReactBoundary,
    TauriCommand,
    TauriPanic,
    Manual,
}

impl Kind {
    fn slug(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Surface {
    Tui,
    Headless,
    Daemon,
    Cli,
    Desktop,
    Ios,
    Android,
    Web,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Os {
    Linux,
    Macos,
    Windows,
    Ios,
    Android,
    Other,
}

impl Os {
    fn slug(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Context {
    version: String,
    git_sha: String,
    surface: Surface,
    os: Os,
    arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    install_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temper: Option<String>,
    #[serde(default)]
    flags: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Event {
    k: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ms: Option<u64>,
    ago: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Report {
    v: u8,
    source: Source,
    #[serde(skip_serializing)]
    install_id: String,
    fingerprint: String,
    error_kind: Kind,
    site: String,
    #[serde(default)]
    location: String,
    message: String,
    #[serde(default)]
    frames: Vec<String>,
    context: Context,
    #[serde(default)]
    events: Vec<Event>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    occurred_at: i64,
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn slug_ok(s: &str, max: usize, extra: &[u8]) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || extra.contains(&b))
}

fn opt(value: &Option<String>, max: usize) -> Option<String> {
    value
        .as_deref()
        .map(|v| scrub_to(v, max))
        .filter(|v| !v.is_empty())
}

/// Validate shape and limits, then scrub every free-text field. Returns the scrubbed report.
fn validate(mut r: Report) -> Result<Report, String> {
    if r.v != 1 {
        return Err("unsupported schema version".into());
    }
    if !is_hex(&r.install_id, 32) {
        return Err("install_id must be 32 lowercase hex characters".into());
    }
    if !is_hex(&r.fingerprint, 16) {
        return Err("fingerprint must be 16 lowercase hex characters".into());
    }
    if !slug_ok(&r.site, 120, b":.<>/") {
        return Err("site has an invalid format".into());
    }
    if r.frames.len() > 12 || r.events.len() > 40 || r.context.flags.len() > 16 {
        return Err("too many frames, events or flags".into());
    }
    let c = &r.context;
    let version_ok = !c.version.is_empty()
        && c.version.len() <= 32
        && c.version.split(['-', '+']).next().is_some_and(|core| {
            let parts: Vec<&str> = core.split('.').collect();
            parts.len() == 3 && parts.iter().all(|p| p.parse::<u64>().is_ok())
        })
        && c.version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
    if !version_ok {
        return Err("context.version must be a semantic version".into());
    }
    let sha_ok = c.git_sha == "unknown"
        || ((7..=12).contains(&c.git_sha.len())
            && c.git_sha.bytes().all(|b| b.is_ascii_hexdigit()));
    if !sha_ok {
        return Err("context.git_sha must be 7-12 hex characters or 'unknown'".into());
    }
    if !slug_ok(&c.arch, 16, &[]) {
        return Err("context.arch has an invalid format".into());
    }
    if r.error_kind == Kind::Manual && r.source != Source::Manual {
        return Err("manual kind requires manual source".into());
    }
    if r.source == Source::Auto && (r.user_note.is_some() || r.last_error.is_some()) {
        return Err("user_note and last_error are only allowed on manual reports".into());
    }
    let user_text =
        r.user_note.as_deref().map_or(0, str::len) + r.last_error.as_deref().map_or(0, str::len);
    if user_text > 4 * 3000 {
        return Err("user text too long".into());
    }
    for event in &r.events {
        if !slug_ok(&event.k, 32, b".:") {
            return Err("event kind has an invalid format".into());
        }
    }

    r.site = scrub_to(&r.site, 120);
    r.location = scrub_to(&r.location, 160);
    r.message = scrub_to(&r.message, 300);
    r.frames = r.frames.iter().map(|f| scrub_to(f, 120)).collect();
    r.user_note = opt(&r.user_note, 2000);
    r.last_error = opt(&r.last_error, 1000);
    let c = &mut r.context;
    c.os_version = opt(&c.os_version, 40);
    c.install_method = opt(&c.install_method, 24);
    c.provider = opt(&c.provider, 40);
    c.model = opt(&c.model, 80);
    c.temper = opt(&c.temper, 24);
    c.flags = c.flags.iter().map(|f| scrub_to(f, 40)).collect();
    Ok(r)
}

fn title_for(r: &Report) -> String {
    if r.source == Source::Manual {
        let first = r
            .user_note
            .as_deref()
            .and_then(|n| n.lines().find(|l| !l.trim().is_empty()))
            .unwrap_or("manual report");
        return format!("[user-report] {}", scrub_to(first.trim(), 80));
    }
    let place = if r.location.is_empty() {
        &r.site
    } else {
        &r.location
    };
    format!(
        "[auto-report] {}: {}",
        r.error_kind.slug(),
        scrub_to(place, 100)
    )
}

fn sha_hex(input: &str) -> String {
    Sha256::digest(input.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({ "ok": false, "error": message })),
    )
        .into_response()
}

fn retry_after(status: StatusCode, secs: u64, message: &str) -> Response {
    let mut response = json_error(status, message);
    if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/reports", post(post_report))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

async fn post_report(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let reports = &state.reports;
    let Some(db) = reports.db.as_ref().filter(|_| reports.enabled()) else {
        return retry_after(
            StatusCode::SERVICE_UNAVAILABLE,
            3600,
            "reports are disabled",
        );
    };
    let ip = crate::client_ip(&headers, &peer, state.trust_proxy_headers);
    let l = &reports.limiters;
    if l.per_ip.check_key(&l.hasher.hash_one(("ip", &ip))).is_err() {
        return retry_after(StatusCode::TOO_MANY_REQUESTS, 600, "rate limited");
    }
    let parsed: Report = match serde_json::from_slice(&body) {
        Ok(report) => report,
        Err(error) => {
            return json_error(StatusCode::BAD_REQUEST, &format!("invalid report: {error}"))
        }
    };
    let report = match validate(parsed) {
        Ok(report) => report,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, &message),
    };
    let install_hash = sha_hex(&format!("forge-report-install:{}", report.install_id));
    if l.per_install
        .check_key(&l.hasher.hash_one(("install", &install_hash)))
        .is_err()
    {
        return retry_after(StatusCode::TOO_MANY_REQUESTS, 600, "rate limited");
    }
    if l.global.check().is_err() {
        return retry_after(StatusCode::TOO_MANY_REQUESTS, 600, "relay is busy");
    }

    let manual = report.source == Source::Manual;
    let now = unix_now();
    let key = if manual {
        let n = reports.manual_seq.fetch_add(1, Ordering::Relaxed);
        format!(
            "manual-{}",
            &sha_hex(&format!("{install_hash}:{now}:{n}"))[..16]
        )
    } else {
        report.fingerprint.clone()
    };
    let representative = serde_json::to_string_pretty(&report).unwrap_or_default();
    let existed = db.problem(&key).ok().flatten().is_some();
    let new = NewReport {
        key: &key,
        manual,
        title: &title_for(&report),
        representative: &representative,
        version: &report.context.version,
        os: &report.context.os.slug(),
        install_hash: &install_hash,
        now,
    };
    if let Err(error) = db.record(&new) {
        tracing::error!("recording report failed: {error}");
        return retry_after(StatusCode::SERVICE_UNAVAILABLE, 300, "storage unavailable");
    }
    reports.wake.notify_one();
    let issue = db.problem(&key).ok().flatten().and_then(|p| p.issue_number);
    (
        StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({ "ok": true, "deduped": existed, "issue": issue })),
    )
        .into_response()
}

#[cfg(test)]
pub(crate) fn test_config(api_base: Option<&str>) -> ReportsConfig {
    ReportsConfig {
        token: api_base.map(|_| Secret::new("ghp_TESTTOKEN0123456789abcdefghijklmnop".into())),
        repo: "Adulari/forge".into(),
        api_base: api_base.unwrap_or("http://127.0.0.1:9").into(),
        db_path: ":memory:".into(),
        disabled: false,
        per_install_per_hour: 1000,
        per_ip_per_hour: 1000,
        global_per_hour: 1000,
        min_update_secs: 0,
    }
}
