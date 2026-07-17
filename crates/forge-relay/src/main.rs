//! `forge-relay` — the hosted APNs push relay (ADR-0012). A self-hosted `forge serve` daemon
//! that hasn't configured its own Apple `.p8` key (`FORGE_APNS_KEY_PATH`/etc.) POSTs here
//! instead of talking to Apple directly; this process holds the operator's real Apple
//! credential and forwards on the daemon's behalf. It never sees session content, source code,
//! or a daemon's auth token — only an opaque device token, an environment string, and the
//! notification payload text (see `docs/features/remote-control.md` for the full disclosure).
//!
//! Wire protocol is a privacy-preserving substitution for Apple's API shape: `POST /3/device`
//! with the token in `x-forge-device-token`, the same
//! `apns-topic`/`apns-push-type`/`apns-priority` headers a direct-to-Apple call would carry,
//! plus a new `apns-environment` header (replacing the role the Apple bearer JWT implicitly
//! played — the caller has no JWT at all in relay mode). The legacy path-token route remains
//! for old clients. The response is Apple's real HTTP status code, proxied verbatim, so
//! `crates/forge-cli/src/apns.rs`'s existing `Ok(410) => prune` pruning logic needs zero changes
//! on the caller's side.

mod apns;
mod config;
mod ratelimit;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;

use apns::ApnsAuth;
use config::RelayConfig;
use ratelimit::RateLimiters;

/// Apple's documented maximum for alert and Live Activity payloads. Enforce it before parsing
/// or forwarding so oversized requests cannot use the relay as a bandwidth/JSON-memory sink.
const APNS_MAX_PAYLOAD_BYTES: usize = 4 * 1024;

struct AppState {
    auth: ApnsAuth,
    client: reqwest::Client,
    allowed_topics: Vec<String>,
    relay_token: Option<String>,
    trust_proxy_headers: bool,
    limiters: Arc<RateLimiters>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = RelayConfig::from_env()?;
    let auth = ApnsAuth::new(&config.key_pem, &config.team_id, &config.key_id)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()?;
    let limiters = Arc::new(RateLimiters::new(
        config.rate_limit_per_window,
        config.rate_window_secs,
        config.daily_send_cap,
    ));
    ratelimit::spawn_daily_reset(limiters.clone());

    let state = Arc::new(AppState {
        auth,
        client,
        allowed_topics: config.allowed_topics,
        relay_token: config.relay_token,
        trust_proxy_headers: config.trust_proxy_headers,
        limiters,
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/3/device", post(send_push_from_header))
        .route("/3/device/{device_token}", post(send_push))
        .layer(DefaultBodyLimit::max(APNS_MAX_PAYLOAD_BYTES))
        .with_state(state.clone());

    let addr = SocketAddr::new(config.bind_addr, config.port);
    tracing::info!("forge-relay listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "daily_sent": state.limiters.daily_sent_count(),
    }))
}

/// The real client IP: Cloudflare (recommended deploy topology, ADR-0012) sits in front and
/// sets `cf-connecting-ip`; fall back to the TCP peer address for direct/local access (e.g. the
/// operator's own smoke tests) where that header is absent.
fn client_ip(headers: &HeaderMap, connect_info: &SocketAddr, trust_proxy_headers: bool) -> String {
    if trust_proxy_headers {
        // Accept only a syntactically valid single IP. The front proxy must overwrite these
        // headers; never enable this mode on a directly reachable listener.
        for name in ["x-real-ip", "cf-connecting-ip"] {
            if let Some(ip) =
                header_str(headers, name).and_then(|value| value.parse::<std::net::IpAddr>().ok())
            {
                return ip.to_string();
            }
        }
    }
    connect_info.ip().to_string()
}

/// iOS hands out device tokens as lowercase hex — reject anything else outright rather than
/// silently lowercasing, so a malformed/tampered token never quietly gets accepted.
fn is_valid_device_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn relay_token_matches(actual: Option<&str>, expected: &str) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    // Compare fixed-size digests so matching does not stop at the first different secret byte.
    let actual = Sha256::digest(actual.as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    actual
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

/// Accept only the two payload shapes Forge itself emits. The relay remains forward-compatible
/// with additional fields, but cannot be used as a generic arbitrary-notification proxy.
fn validate_payload(payload: &serde_json::Value, push_type: &str) -> Result<(), &'static str> {
    let root = payload.as_object().ok_or("payload must be a JSON object")?;
    let aps = root
        .get("aps")
        .and_then(serde_json::Value::as_object)
        .ok_or("payload.aps must be a JSON object")?;

    if push_type == "alert" {
        let alert = aps
            .get("alert")
            .and_then(serde_json::Value::as_object)
            .ok_or("alert payload requires aps.alert")?;
        let title = alert
            .get("title")
            .and_then(serde_json::Value::as_str)
            .ok_or("alert title must be a string")?;
        let body = alert
            .get("body")
            .and_then(serde_json::Value::as_str)
            .ok_or("alert body must be a string")?;
        if title.chars().count() > 512 || body.chars().count() > 512 {
            return Err("alert title/body is too long");
        }
        if !matches!(
            root.get("kind").and_then(serde_json::Value::as_str),
            Some("permission" | "question" | "done" | "failed")
        ) {
            return Err("alert kind is not allowed");
        }
        if root
            .get("session")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|session| session.is_empty() || session.len() > 256)
            || root
                .get("seq")
                .and_then(serde_json::Value::as_u64)
                .is_none()
        {
            return Err("alert session/seq is invalid");
        }
    } else {
        if aps.get("event").and_then(serde_json::Value::as_str) != Some("update")
            || aps
                .get("timestamp")
                .and_then(serde_json::Value::as_u64)
                .is_none()
        {
            return Err("Live Activity event/timestamp is invalid");
        }
        let content = aps
            .get("content-state")
            .and_then(serde_json::Value::as_object)
            .ok_or("Live Activity content-state is required")?;
        if content
            .get("busy")
            .and_then(serde_json::Value::as_bool)
            .is_none()
            || content
                .get("waiting")
                .and_then(serde_json::Value::as_bool)
                .is_none()
            || content
                .get("cost_usd")
                .and_then(serde_json::Value::as_f64)
                .is_none()
            || content
                .get("context_tokens")
                .and_then(serde_json::Value::as_u64)
                .is_none()
            || content
                .get("context_limit")
                .is_some_and(|limit| limit.as_u64().is_none())
        {
            return Err("Live Activity content-state is invalid");
        }
    }
    Ok(())
}

async fn send_push(
    State(state): State<Arc<AppState>>,
    ConnectInfo(connect_info): ConnectInfo<SocketAddr>,
    Path(device_token): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    // --- validation, before anything touches Apple or the rate limiters ---
    if let Some(expected) = &state.relay_token {
        if !relay_token_matches(header_str(&headers, "x-forge-relay-token"), expected) {
            return (StatusCode::UNAUTHORIZED, "valid relay token required").into_response();
        }
    }
    if !is_valid_device_token(&device_token) {
        return (StatusCode::BAD_REQUEST, "device_token must be 64 hex chars").into_response();
    }
    let topic = match header_str(&headers, "apns-topic") {
        Some(t) => t.to_string(),
        None => return (StatusCode::BAD_REQUEST, "apns-topic header required").into_response(),
    };
    if !state.allowed_topics.iter().any(|t| t == &topic) {
        tracing::warn!("rejected disallowed topic: {topic}");
        return (StatusCode::BAD_REQUEST, "topic not allowed on this relay").into_response();
    }
    let push_type = header_str(&headers, "apns-push-type").unwrap_or("alert");
    if push_type != "alert" && push_type != "liveactivity" {
        return (
            StatusCode::BAD_REQUEST,
            "apns-push-type must be alert or liveactivity",
        )
            .into_response();
    }
    if (push_type == "liveactivity") != topic.ends_with(".push-type.liveactivity") {
        return (
            StatusCode::BAD_REQUEST,
            "liveactivity pushes require a .push-type.liveactivity topic",
        )
            .into_response();
    }
    let environment = header_str(&headers, "apns-environment").unwrap_or("sandbox");
    if environment != "production" && environment != "sandbox" {
        return (
            StatusCode::BAD_REQUEST,
            "apns-environment must be production or sandbox",
        )
            .into_response();
    }
    if let Err(reason) = validate_payload(&payload, push_type) {
        return (StatusCode::BAD_REQUEST, reason).into_response();
    }
    if !matches!(
        header_str(&headers, "apns-priority"),
        None | Some("5" | "10")
    ) {
        return (StatusCode::BAD_REQUEST, "apns-priority must be 5 or 10").into_response();
    }

    // --- abuse prevention ---
    let ip = client_ip(&headers, &connect_info, state.trust_proxy_headers);
    if let Err(reason) = state.limiters.check_and_reserve(&ip, &device_token) {
        // APNs device tokens are bearer-like routing secrets. Never retain one in logs.
        tracing::warn!("rate limited ({reason:?}) ip={ip}");
        return (StatusCode::TOO_MANY_REQUESTS, format!("{reason:?}")).into_response();
    }

    // --- forward to Apple, proxying its real status code back verbatim ---
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let url = format!("{}/3/device/{device_token}", apns::host(environment));
    let result = state
        .client
        .post(&url)
        .header(
            "authorization",
            format!("bearer {}", state.auth.bearer_token(now)),
        )
        .header("apns-topic", &topic)
        .header("apns-push-type", push_type)
        .header(
            "apns-priority",
            header_str(&headers, "apns-priority").unwrap_or("10"),
        )
        .json(&payload)
        .send()
        .await
        // The device token is part of Apple's URL. Never let a transport error retain it.
        .map_err(reqwest::Error::without_url);

    match result {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                body,
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("upstream Apple call failed: {e}");
            (StatusCode::BAD_GATEWAY, "upstream APNs call failed").into_response()
        }
    }
}

/// Privacy-preserving relay protocol used by current clients: keep the opaque APNs token out of
/// the URL so it cannot enter CDN/proxy path analytics. The path-token route remains above for
/// backward compatibility with already-released Forge daemons.
async fn send_push_from_header(
    State(state): State<Arc<AppState>>,
    ConnectInfo(connect_info): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(device_token) = header_str(&headers, "x-forge-device-token").map(str::to_string)
    else {
        return (
            StatusCode::BAD_REQUEST,
            "x-forge-device-token header required",
        )
            .into_response();
    };
    send_push(
        State(state),
        ConnectInfo(connect_info),
        Path(device_token),
        headers,
        Json(payload),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    #[test]
    fn device_token_validation() {
        assert!(is_valid_device_token(&"a".repeat(64)));
        assert!(!is_valid_device_token(&"a".repeat(63)));
        assert!(!is_valid_device_token(&"g".repeat(64)));
        assert!(!is_valid_device_token(
            "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef01234567"
        ));
    }

    #[test]
    fn proxy_ip_headers_are_explicit_and_validated() {
        let peer: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.8".parse().unwrap());
        assert_eq!(client_ip(&headers, &peer, false), "127.0.0.1");
        assert_eq!(client_ip(&headers, &peer, true), "203.0.113.8");

        headers.insert("x-real-ip", "spoofed, 203.0.113.8".parse().unwrap());
        assert_eq!(client_ip(&headers, &peer, true), "127.0.0.1");
    }

    #[test]
    fn relay_token_comparison_requires_an_exact_value() {
        assert!(relay_token_matches(Some("correct horse"), "correct horse"));
        assert!(!relay_token_matches(Some("correct house"), "correct horse"));
        assert!(!relay_token_matches(None, "correct horse"));
    }

    #[test]
    fn payload_validation_accepts_only_forge_shapes() {
        let alert = serde_json::json!({
            "aps": { "alert": { "title": "Forge", "body": "Done" } },
            "session": "session-1",
            "kind": "done",
            "seq": 7,
        });
        assert!(validate_payload(&alert, "alert").is_ok());
        assert!(validate_payload(&serde_json::json!({ "aps": {} }), "alert").is_err());
        assert!(validate_payload(
            &serde_json::json!({
                "aps": { "alert": { "title": "Spam", "body": "arbitrary" } },
                "session": "session-1", "kind": "marketing", "seq": 1
            }),
            "alert"
        )
        .is_err());

        let live_activity = serde_json::json!({
            "aps": {
                "timestamp": 1_700_000_000u64,
                "event": "update",
                "content-state": {
                    "busy": true,
                    "waiting": false,
                    "cost_usd": 0.02,
                    "context_tokens": 1000,
                    "context_limit": 200_000,
                }
            }
        });
        assert!(validate_payload(&live_activity, "liveactivity").is_ok());
    }

    /// Builds real `AppState` (a real `ApnsAuth` from a test scalar, same trick
    /// `apns.rs`'s own tests use) so these exercise the actual `send_push` handler, not a
    /// stand-in — every case here must reject BEFORE the handler ever reaches
    /// `state.client.post(...)`, so no real network call happens in this test suite.
    fn test_state(allowed_topics: &[&str]) -> Arc<AppState> {
        Arc::new(AppState {
            auth: ApnsAuth::from_scalar(&[7u8; 32], "TEAM", "KEY"),
            client: reqwest::Client::new(),
            allowed_topics: allowed_topics.iter().map(|s| s.to_string()).collect(),
            relay_token: None,
            trust_proxy_headers: false,
            limiters: Arc::new(RateLimiters::new(1000, 60, 1_000_000)),
        })
    }

    fn peer() -> SocketAddr {
        "127.0.0.1:12345".parse().unwrap()
    }

    async fn body_text(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn rejects_payloads_over_apns_limit_before_handler() {
        let app = Router::new()
            .route("/3/device/{device_token}", post(send_push))
            .layer(DefaultBodyLimit::max(APNS_MAX_PAYLOAD_BYTES))
            .with_state(test_state(&["dev.adulari.forge"]));
        let mut req = axum::http::Request::post(format!("/3/device/{}", "a".repeat(64)))
            .header("content-type", "application/json")
            .header("apns-topic", "dev.adulari.forge")
            .body(axum::body::Body::from(format!(
                r#"{{"aps":{{"alert":{{"title":"Forge","body":"{}"}}}},"session":"s","kind":"done","seq":1}}"#,
                "x".repeat(APNS_MAX_PAYLOAD_BYTES)
            )))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(peer()));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn rejects_invalid_device_token_before_any_network_call() {
        let state = test_state(&["dev.adulari.forge"]);
        let mut headers = HeaderMap::new();
        headers.insert("apns-topic", "dev.adulari.forge".parse().unwrap());
        let resp = send_push(
            State(state),
            ConnectInfo(peer()),
            Path("not-hex".to_string()),
            headers,
            Json(serde_json::json!({})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(body_text(resp).await.contains("device_token"));
    }

    #[tokio::test]
    async fn rejects_missing_topic_header() {
        let state = test_state(&["dev.adulari.forge"]);
        let resp = send_push(
            State(state),
            ConnectInfo(peer()),
            Path("a".repeat(64)),
            HeaderMap::new(),
            Json(serde_json::json!({})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(body_text(resp).await.contains("apns-topic"));
    }

    #[tokio::test]
    async fn rejects_disallowed_topic_without_calling_upstream() {
        let state = test_state(&["dev.adulari.forge"]);
        let mut headers = HeaderMap::new();
        headers.insert("apns-topic", "com.someone.else".parse().unwrap());
        let resp = send_push(
            State(state),
            ConnectInfo(peer()),
            Path("a".repeat(64)),
            headers,
            Json(serde_json::json!({})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(body_text(resp).await.contains("not allowed"));
    }

    #[tokio::test]
    async fn rejects_bad_push_type() {
        let state = test_state(&["dev.adulari.forge"]);
        let mut headers = HeaderMap::new();
        headers.insert("apns-topic", "dev.adulari.forge".parse().unwrap());
        headers.insert("apns-push-type", "carrier-pigeon".parse().unwrap());
        let resp = send_push(
            State(state),
            ConnectInfo(peer()),
            Path("a".repeat(64)),
            headers,
            Json(serde_json::json!({})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(body_text(resp).await.contains("apns-push-type"));
    }

    #[tokio::test]
    async fn rejects_bad_environment() {
        let state = test_state(&["dev.adulari.forge"]);
        let mut headers = HeaderMap::new();
        headers.insert("apns-topic", "dev.adulari.forge".parse().unwrap());
        headers.insert("apns-environment", "moon-base".parse().unwrap());
        let resp = send_push(
            State(state),
            ConnectInfo(peer()),
            Path("a".repeat(64)),
            headers,
            Json(serde_json::json!({})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(body_text(resp).await.contains("apns-environment"));
    }
}
