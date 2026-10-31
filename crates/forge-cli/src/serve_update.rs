//! `GET/POST /api/update`: the daemon's own update, for clients (the desktop app) that want to
//! tell the user the Forge CLI behind them is out of date and let them fix it in one click.
//!
//! `GET` reports the running version, the newest release, and how this install updates. `POST`
//! runs the self-update for a standalone install (a package-manager install answers `409` with
//! the exact command instead) and then restarts the background service, but only once no session
//! is mid-turn. `POST` is loopback-only: the token gates every route, but replacing the binary is
//! the one action that should never be reachable through a tunnel or LAN listener.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::serve::{err_response, json_response, DaemonState, SessionRegistry};
use crate::update::{detect_install_method, InstallMethod};
use crate::update_state::{disabled, now, UpdateState};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// A forced re-check is honoured at most this often, so a client cannot spend the API quota.
const FORCED_CHECK_GAP_SECS: u64 = 60;
/// How long a restart waits for sessions to finish their turns before giving up.
const IDLE_WAIT: Duration = Duration::from_secs(30 * 60);

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(
        &format!("{base}/api/update"),
        get(get_update).post(post_update),
    )
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct UpdateStatus {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub notes: String,
    pub url: String,
    pub method: &'static str,
    pub command: Option<String>,
    pub can_self_update: bool,
    pub busy_sessions: usize,
    pub service_managed: bool,
    pub checked_at: u64,
    /// The daemon installs updates itself when no session is mid-turn; clients need not.
    pub auto_update: bool,
}

/// The wire status for what is known. Pure, so every state is testable without a daemon.
pub(crate) fn status_from(
    state: &UpdateState,
    current: &str,
    method: InstallMethod,
    busy_sessions: usize,
    service_managed: bool,
) -> UpdateStatus {
    let offer = state.offer(current);
    let known = offer.or(state.latest.as_ref());
    UpdateStatus {
        current: current.to_string(),
        latest: known.map(|r| r.version.clone()),
        update_available: offer.is_some(),
        notes: offer.map(|r| r.notes.clone()).unwrap_or_default(),
        url: known.map(|r| r.url.clone()).unwrap_or_default(),
        method: method.wire_name(),
        command: method.upgrade_command(),
        can_self_update: method.upgrade_command().is_none(),
        busy_sessions,
        service_managed,
        checked_at: state.last_check,
        auto_update: false,
    }
}

/// Whether a request reached the daemon directly from this machine: a loopback `Host`, and none of
/// the headers a tunnel or reverse proxy adds when it forwards someone else's request.
fn is_local_request(headers: &HeaderMap) -> bool {
    const FORWARDED: [&str; 6] = [
        "x-forwarded-for",
        "x-forwarded-host",
        "forwarded",
        "cf-connecting-ip",
        "cf-ray",
        "x-real-ip",
    ];
    if FORWARDED.iter().any(|h| headers.contains_key(*h)) {
        return false;
    }
    let Some(host) = headers.get("host").and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Whether a request truly came from this machine. The `Host` header alone is not enough: the
/// desktop app's pinned loopback proxy forwards a remote daemon's traffic with `Host: 127.0.0.1`,
/// and any paired client can type that header. The TCP peer cannot be forged, so it must be
/// loopback too. A request served without connection info is refused.
pub(crate) struct Direct(pub bool);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Direct {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let peer_is_loopback = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .is_some_and(|c| c.0.ip().is_loopback());
        Ok(Direct(peer_is_loopback && is_local_request(&parts.headers)))
    }
}

async fn busy_sessions(registry: &SessionRegistry) -> usize {
    registry
        .all()
        .await
        .iter()
        .filter(|h| h.snapshot_rx.borrow().snapshot.busy)
        .count()
}

fn service_managed() -> bool {
    crate::cli::commands::service::service_is_running()
}

#[derive(Deserialize)]
struct StatusQuery {
    #[serde(default)]
    refresh: Option<String>,
}

async fn get_update(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<StatusQuery>,
) -> Response {
    let config = forge_config::load().unwrap_or_default();
    let known = UpdateState::load();
    let forced = query.refresh.is_some() && known.due(now(), FORCED_CHECK_GAP_SECS);
    let update = if disabled(&config) {
        known
    } else {
        crate::update_check::refresh_state(forced).await
    };
    let busy = busy_sessions(&state.registry).await;
    let managed = tokio::task::spawn_blocking(service_managed)
        .await
        .unwrap_or(false);
    let mut status = status_from(&update, CURRENT, detect_install_method(), busy, managed);
    status.auto_update = status.can_self_update && crate::update_auto::auto_enabled(&config);
    json_response(&status)
}

/// The seams of [`run_update`], so the decision logic is testable without replacing a binary or
/// restarting a service.
pub(crate) struct UpdateEffects {
    pub apply: Box<dyn FnOnce() -> anyhow::Result<Option<String>> + Send>,
    pub service_managed: bool,
    pub schedule_restart: Box<dyn FnOnce() + Send>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct UpdateOutcome {
    pub status: &'static str,
    pub version: String,
    pub restart: &'static str,
    pub command: Option<String>,
}

/// Run the update. `Err` carries the HTTP status and message for the caller to relay.
pub(crate) fn run_update(
    method: InstallMethod,
    busy: usize,
    effects: UpdateEffects,
) -> Result<UpdateOutcome, (StatusCode, UpdateOutcome)> {
    if let Some(command) = method.upgrade_command() {
        return Err((
            StatusCode::CONFLICT,
            UpdateOutcome {
                status: "manual",
                version: CURRENT.to_string(),
                restart: "manual",
                command: Some(command),
            },
        ));
    }
    match (effects.apply)() {
        Ok(Some(version)) => {
            let restart = if !effects.service_managed {
                "manual"
            } else {
                (effects.schedule_restart)();
                if busy == 0 {
                    "scheduled"
                } else {
                    "waiting_for_idle"
                }
            };
            Ok(UpdateOutcome {
                status: "updated",
                version,
                restart,
                command: None,
            })
        }
        Ok(None) => Ok(UpdateOutcome {
            status: "already_current",
            version: CURRENT.to_string(),
            restart: "not_needed",
            command: None,
        }),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            UpdateOutcome {
                status: "failed",
                version: CURRENT.to_string(),
                restart: "not_needed",
                command: None,
            },
        )),
    }
}

async fn post_update(State(state): State<Arc<DaemonState>>, Direct(local): Direct) -> Response {
    if !local {
        return err_response(
            StatusCode::FORBIDDEN,
            "updating the daemon is only allowed from the machine it runs on",
        );
    }
    let method = detect_install_method();
    let busy = busy_sessions(&state.registry).await;
    let registry = state.registry.clone();
    let managed = tokio::task::spawn_blocking(service_managed)
        .await
        .unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || {
        run_update(
            method,
            busy,
            UpdateEffects {
                apply: Box::new(|| crate::update::apply_in_place(false)),
                service_managed: managed,
                schedule_restart: Box::new(move || {
                    tokio::spawn(restart_when_idle(registry));
                }),
            },
        )
    })
    .await;
    match result {
        Ok(Ok(outcome)) => json_response(&outcome),
        Ok(Err((status, outcome))) => {
            let mut response = json_response(&outcome);
            *response.status_mut() = status;
            response
        }
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "the update task failed"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonAction {
    /// Nothing to do, or not yet: the daemon is busy or the update was already applied.
    Wait,
    /// Replace the binary, then restart once idle.
    Apply,
    /// The binary on disk is already newer than the running daemon: just restart once idle.
    RestartOnly,
}

pub(crate) struct DaemonFacts {
    pub auto: bool,
    pub method: InstallMethod,
    pub newer_known: bool,
    pub binary_replaced: bool,
    pub busy_sessions: usize,
    /// This run already put that version in place.
    pub already_applied: bool,
}

/// What the daemon's own updater does now. A running turn is never interrupted: any busy
/// session means wait. Only a standalone install updates itself; package managers keep their own.
pub(crate) fn daemon_action(f: &DaemonFacts) -> DaemonAction {
    if !f.auto || f.method != InstallMethod::Binary || f.busy_sessions > 0 {
        return DaemonAction::Wait;
    }
    if f.binary_replaced {
        DaemonAction::RestartOnly
    } else if f.newer_known && !f.already_applied {
        DaemonAction::Apply
    } else {
        DaemonAction::Wait
    }
}

pub(crate) fn start_auto(registry: &Arc<SessionRegistry>) {
    tokio::spawn(auto_update_loop(registry.clone()));
}

/// The daemon updates itself: on start and every check interval, when nothing is mid-turn,
/// replace the binary and restart the service. A foreground `forge serve` (no service) keeps
/// running the old build until it is restarted by hand.
pub(crate) async fn auto_update_loop(registry: Arc<SessionRegistry>) {
    let mut applied = false;
    loop {
        let config = forge_config::load().unwrap_or_default();
        if crate::update_auto::auto_enabled(&config) {
            let state = crate::update_check::refresh_state(false).await;
            let action = daemon_action(&DaemonFacts {
                auto: true,
                method: detect_install_method(),
                newer_known: state.offer(CURRENT).is_some(),
                binary_replaced: forge_types::self_exe::running_binary_replaced(),
                busy_sessions: busy_sessions(&registry).await,
                already_applied: applied,
            });
            let ready = match action {
                DaemonAction::Wait => false,
                DaemonAction::RestartOnly => true,
                DaemonAction::Apply => {
                    let done =
                        tokio::task::spawn_blocking(|| crate::update::apply_in_place(false)).await;
                    applied = matches!(done, Ok(Ok(Some(_))));
                    applied
                }
            };
            let managed = tokio::task::spawn_blocking(service_managed)
                .await
                .unwrap_or(false);
            if ready && managed {
                restart_when_idle(registry.clone()).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(
            crate::update_state::check_interval().clamp(30, 6 * 3600),
        ))
        .await;
    }
}

/// Wait for every session this daemon hosts to finish its turn, then restart the service.
async fn restart_when_idle(registry: Arc<SessionRegistry>) {
    let deadline = tokio::time::Instant::now() + IDLE_WAIT;
    while busy_sessions(&registry).await > 0 {
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    // Let the HTTP response that announced the restart leave first.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = tokio::task::spawn_blocking(crate::cli::commands::service::restart_service).await;
}

/// Restart the background service from outside it (the TUI after an update), refusing while any
/// session it hosts is mid-turn.
pub(crate) async fn restart_service_when_idle() -> Result<(), String> {
    let base = crate::attach::resolve_base_url(None);
    let token = crate::attach::resolve_token(None).map_err(|e| format!("{e:#}"))?;
    let http = forge_provider::bundled_http_client();
    let sessions = crate::attach::fetch_sessions(&http, &base, &token)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let busy = sessions.iter().filter(|s| s.busy).count();
    if busy > 0 {
        return Err(format!("{busy} session(s) are mid-turn"));
    }
    tokio::task::spawn_blocking(crate::cli::commands::service::restart_service)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update_feed::Release;

    fn state_with(latest: &str) -> UpdateState {
        let mut s = UpdateState::default();
        s.record_attempt(99);
        s.record_release(Some(Release {
            version: latest.into(),
            notes: "- Faster".into(),
            url: "https://example.test/r".into(),
        }));
        s
    }

    #[test]
    fn status_reports_a_newer_release_and_how_to_install_it() {
        let s = status_from(
            &state_with("9.9.9"),
            "2.23.6",
            InstallMethod::Binary,
            2,
            true,
        );
        assert!(s.update_available && s.can_self_update);
        assert_eq!(s.latest.as_deref(), Some("9.9.9"));
        assert_eq!(
            (s.busy_sessions, s.service_managed, s.checked_at),
            (2, true, 99)
        );
        assert_eq!(s.notes, "- Faster");
        let brew = status_from(
            &state_with("9.9.9"),
            "2.23.6",
            InstallMethod::Homebrew,
            0,
            false,
        );
        assert!(!brew.can_self_update);
        assert_eq!(brew.command.as_deref(), Some("brew upgrade forge"));
        assert_eq!(brew.method, "homebrew");
    }

    #[test]
    fn status_is_quiet_when_current_skipped_or_unknown() {
        let current = status_from(
            &state_with("2.23.6"),
            "2.23.6",
            InstallMethod::Binary,
            0,
            false,
        );
        assert!(!current.update_available);
        assert_eq!(current.latest.as_deref(), Some("2.23.6"));
        let mut skipped = state_with("9.9.9");
        skipped.skip("9.9.9");
        let s = status_from(&skipped, "2.23.6", InstallMethod::Binary, 0, false);
        assert!(!s.update_available);
        let none = status_from(
            &UpdateState::default(),
            "2.23.6",
            InstallMethod::Binary,
            0,
            false,
        );
        assert_eq!((none.latest, none.update_available), (None, false));
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn only_a_direct_loopback_request_may_update() {
        assert!(is_local_request(&headers(&[("host", "127.0.0.1:7420")])));
        assert!(is_local_request(&headers(&[("host", "localhost:7420")])));
        assert!(is_local_request(&headers(&[("host", "[::1]:7420")])));
        assert!(!is_local_request(&headers(&[("host", "192.168.1.5:7420")])));
        assert!(!is_local_request(&headers(&[(
            "host",
            "forge.example.com"
        )])));
        assert!(!is_local_request(&headers(&[])), "no Host: refuse");
        for fwd in [
            "x-forwarded-for",
            "cf-connecting-ip",
            "forwarded",
            "x-real-ip",
        ] {
            assert!(
                !is_local_request(&headers(&[("host", "127.0.0.1:7420"), (fwd, "1.2.3.4")])),
                "{fwd} marks a tunnelled request"
            );
        }
    }

    fn effects(
        apply: anyhow::Result<Option<String>>,
        managed: bool,
        restarted: &Arc<std::sync::atomic::AtomicBool>,
    ) -> UpdateEffects {
        let flag = restarted.clone();
        UpdateEffects {
            apply: Box::new(move || apply),
            service_managed: managed,
            schedule_restart: Box::new(move || {
                flag.store(true, std::sync::atomic::Ordering::SeqCst)
            }),
        }
    }

    #[test]
    fn a_standalone_update_restarts_a_managed_service_once_idle() {
        let restarted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = run_update(
            InstallMethod::Binary,
            0,
            effects(Ok(Some("9.9.9".into())), true, &restarted),
        )
        .unwrap();
        assert_eq!(
            (done.status, done.version.as_str(), done.restart),
            ("updated", "9.9.9", "scheduled")
        );
        assert!(restarted.load(std::sync::atomic::Ordering::SeqCst));

        let busy = run_update(
            InstallMethod::Binary,
            3,
            effects(Ok(Some("9.9.9".into())), true, &restarted),
        )
        .unwrap();
        assert_eq!(busy.restart, "waiting_for_idle");
    }

    #[test]
    fn an_unmanaged_daemon_is_never_restarted_for_the_user() {
        let restarted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = run_update(
            InstallMethod::Binary,
            0,
            effects(Ok(Some("9.9.9".into())), false, &restarted),
        )
        .unwrap();
        assert_eq!(done.restart, "manual");
        assert!(!restarted.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn a_package_manager_install_answers_conflict_with_its_command_and_touches_nothing() {
        let restarted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (status, outcome) = run_update(
            InstallMethod::Homebrew,
            0,
            UpdateEffects {
                apply: Box::new(|| panic!("a brew install must never be self-replaced")),
                service_managed: true,
                schedule_restart: Box::new(|| panic!("nothing to restart")),
            },
        )
        .unwrap_err();
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(outcome.status, "manual");
        assert_eq!(outcome.command.as_deref(), Some("brew upgrade forge"));
        assert!(!restarted.load(std::sync::atomic::Ordering::SeqCst));
    }

    fn router() -> Router {
        let state = Arc::new(DaemonState {
            registry: Arc::new(SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: "/tmp".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        crate::serve::daemon_router(state)
    }

    #[tokio::test]
    async fn the_route_is_token_scoped_reports_status_and_refuses_a_forwarded_post() {
        use tower::ServiceExt;
        // The status lookup must not reach the network from a test.
        std::env::set_var("FORGE_NO_UPDATE_CHECK", "1");
        let router = router();
        let call_from =
            |peer: [u8; 4], method: &'static str, uri: &'static str, host: Option<&'static str>| {
                let mut req = axum::http::Request::builder().method(method).uri(uri);
                if let Some(host) = host {
                    req = req.header("host", host);
                }
                let mut req = req.body(axum::body::Body::empty()).unwrap();
                req.extensions_mut().insert(axum::extract::ConnectInfo(
                    std::net::SocketAddr::from((peer, 50000)),
                ));
                router.clone().oneshot(req)
            };
        let call = |method: &'static str, uri: &'static str, host: Option<&'static str>| {
            call_from([127, 0, 0, 1], method, uri, host)
        };

        let wrong_token = call("GET", "/api/update", None).await.unwrap();
        assert_eq!(wrong_token.status(), StatusCode::NOT_FOUND);

        let ok = call("GET", "/tok/api/update", Some("127.0.0.1:7420"))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(ok.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["current"], CURRENT);
        assert_eq!(body["busy_sessions"], 0);
        for key in [
            "update_available",
            "method",
            "can_self_update",
            "service_managed",
        ] {
            assert!(body.get(key).is_some(), "{key} missing: {body}");
        }

        for host in [Some("192.168.1.5:7420"), Some("tunnel.example.com"), None] {
            let refused = call("POST", "/tok/api/update", host).await.unwrap();
            assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{host:?}");
        }

        // The desktop's pinned loopback proxy relays a remote client with `Host: 127.0.0.1`.
        let proxied = call_from(
            [192, 168, 1, 9],
            "POST",
            "/tok/api/update",
            Some("127.0.0.1:7420"),
        )
        .await
        .unwrap();
        assert_eq!(proxied.status(), StatusCode::FORBIDDEN);
    }

    fn daemon_facts() -> DaemonFacts {
        DaemonFacts {
            auto: true,
            method: InstallMethod::Binary,
            newer_known: true,
            binary_replaced: false,
            busy_sessions: 0,
            already_applied: false,
        }
    }

    #[test]
    fn the_daemon_updates_itself_only_when_idle_and_enabled() {
        assert_eq!(daemon_action(&daemon_facts()), DaemonAction::Apply);
        let busy = DaemonFacts {
            busy_sessions: 1,
            ..daemon_facts()
        };
        assert_eq!(daemon_action(&busy), DaemonAction::Wait, "never mid-turn");
        let off = DaemonFacts {
            auto: false,
            ..daemon_facts()
        };
        assert_eq!(daemon_action(&off), DaemonAction::Wait);
        let brew = DaemonFacts {
            method: InstallMethod::Homebrew,
            ..daemon_facts()
        };
        assert_eq!(
            daemon_action(&brew),
            DaemonAction::Wait,
            "package managers own it"
        );
        let nothing_new = DaemonFacts {
            newer_known: false,
            ..daemon_facts()
        };
        assert_eq!(daemon_action(&nothing_new), DaemonAction::Wait);
    }

    #[test]
    fn a_replaced_binary_only_needs_a_restart_and_an_applied_update_is_not_repeated() {
        let replaced = DaemonFacts {
            binary_replaced: true,
            ..daemon_facts()
        };
        assert_eq!(daemon_action(&replaced), DaemonAction::RestartOnly);
        let busy = DaemonFacts {
            busy_sessions: 2,
            ..replaced
        };
        assert_eq!(daemon_action(&busy), DaemonAction::Wait);
        let applied = DaemonFacts {
            already_applied: true,
            ..daemon_facts()
        };
        assert_eq!(daemon_action(&applied), DaemonAction::Wait);
    }

    #[test]
    fn already_current_and_failed_updates_never_restart() {
        let restarted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let same = run_update(
            InstallMethod::Binary,
            0,
            effects(Ok(None), true, &restarted),
        )
        .unwrap();
        assert_eq!(
            (same.status, same.restart),
            ("already_current", "not_needed")
        );
        let (status, failed) = run_update(
            InstallMethod::Binary,
            0,
            effects(Err(anyhow::anyhow!("disk full")), true, &restarted),
        )
        .unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(failed.status, "failed");
        assert!(!restarted.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn a_source_build_daemon_never_updates_or_restarts_itself() {
        let f = DaemonFacts {
            auto: true,
            method: InstallMethod::Source,
            newer_known: true,
            binary_replaced: true,
            busy_sessions: 0,
            already_applied: false,
        };
        assert_eq!(daemon_action(&f), DaemonAction::Wait);
    }
}
