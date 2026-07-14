//! `forge-relay` — the hosted APNs push relay (ADR-0012). A self-hosted `forge serve` daemon
//! that hasn't configured its own Apple `.p8` key (`FORGE_APNS_KEY_PATH`/etc.) POSTs here
//! instead of talking to Apple directly; this process holds the operator's real Apple
//! credential and forwards on the daemon's behalf. It never sees session content, source code,
//! or a daemon's auth token — only an opaque device token, an environment string, and the
//! notification payload text (see `docs/features/remote-control.md` for the full disclosure).
//!
//! Wire protocol is a drop-in substitution for Apple's own API shape: `POST
//! /3/device/{device_token}` with the same `apns-topic`/`apns-push-type`/`apns-priority`
//! headers a direct-to-Apple call would carry, plus a new `apns-environment` header (replacing
//! the role the Apple bearer JWT implicitly played — the caller has no JWT at all in relay
//! mode). The response is Apple's real HTTP status code, proxied verbatim, so
//! `crates/forge-cli/src/apns.rs`'s existing `Ok(410) => prune` pruning logic needs zero changes
//! on the caller's side.

mod apns;
mod config;
mod ratelimit;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::net::SocketAddr;
use std::sync::Arc;

use apns::ApnsAuth;
use config::RelayConfig;
use ratelimit::RateLimiters;

struct AppState {
    auth: ApnsAuth,
    client: reqwest::Client,
    allowed_topics: Vec<String>,
    relay_token: Option<String>,
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
        limiters,
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/3/device/{device_token}", post(send_push))
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
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
fn client_ip(headers: &HeaderMap, connect_info: &SocketAddr) -> String {
    headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| connect_info.ip().to_string())
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

async fn send_push(
    State(state): State<Arc<AppState>>,
    ConnectInfo(connect_info): ConnectInfo<SocketAddr>,
    Path(device_token): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    // --- validation, before anything touches Apple or the rate limiters ---
    if let Some(expected) = &state.relay_token {
        if header_str(&headers, "x-forge-relay-token") != Some(expected) {
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

    // --- abuse prevention ---
    let ip = client_ip(&headers, &connect_info);
    if let Err(reason) = state.limiters.check(&ip, &device_token) {
        tracing::warn!("rate limited ({reason:?}) ip={ip} device_token={device_token}");
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
        .await;

    match result {
        Ok(resp) => {
            state.limiters.record_sent();
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[test]
    fn device_token_validation() {
        assert!(is_valid_device_token(&"a".repeat(64)));
        assert!(!is_valid_device_token(&"a".repeat(63)));
        assert!(!is_valid_device_token(&"g".repeat(64)));
        assert!(!is_valid_device_token(
            "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef01234567"
        ));
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
