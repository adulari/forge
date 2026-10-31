//! `POST /api/push/subscribe` and `/unsubscribe`: device registration for Web Push, APNs and
//! Live Activity updates. Every row is tied to the paired client that registered it so revoking
//! the client also stops its notifications.

use std::sync::Arc;

use axum::response::Response;

use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_clients::{ClientAuth, Scope};
use crate::serve_push_owners as owners;
use axum::extract::State;

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
pub(crate) enum SubscribeReq {
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
        /// The device's end-to-end notification key (P-256 public key + RFC 8291 auth secret).
        /// Optional: an app build without the Notification Service Extension omits it and keeps
        /// the generic lock-screen alert.
        #[serde(default)]
        keys: SubscribeKeys,
    },
    Webpush {
        endpoint: String,
        #[serde(default)]
        keys: SubscribeKeys,
    },
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct SubscribeKeys {
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
    if !is_push_service_endpoint(endpoint) {
        return Err("endpoint is not a known https push service");
    }
    validate_keys(keys)
}

/// The RFC 8291 key pair shape shared by Web Push subscriptions and an iOS device's end-to-end
/// notification key. The point is checked to be on the curve, so an unusable key is refused at
/// registration instead of silently downgrading every later alert to the generic text.
fn validate_keys(keys: &SubscribeKeys) -> Result<(), &'static str> {
    match crate::push::b64url_decode(&keys.p256dh) {
        Some(k)
            if k.len() == 65 && k[0] == 0x04 && p256::PublicKey::from_sec1_bytes(&k).is_ok() => {}
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
pub(crate) async fn push_subscribe(
    State(state): State<Arc<DaemonState>>,
    axum::Extension(auth): axum::Extension<ClientAuth>,
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
            if !owners::may_modify(&auth, "live", &session_id) {
                return err_response(
                    axum::http::StatusCode::FORBIDDEN,
                    "this session's live activity belongs to another device",
                );
            }
            owners::record(&auth, "live", &session_id);
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
            keys,
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
            owners::record(&auth, "apns", &device_token);
            let environment = normalize_apns_environment(&environment).to_string();
            let e2e = if keys.p256dh.is_empty() && keys.auth.is_empty() {
                None
            } else if let Err(msg) = validate_keys(&keys) {
                return err_response(axum::http::StatusCode::BAD_REQUEST, msg);
            } else {
                Some(forge_store::ApnsE2eKey {
                    p256dh: keys.p256dh,
                    auth: keys.auth,
                })
            };
            let store = state.store.clone();
            let stored = tokio::task::spawn_blocking(move || {
                store.upsert_apns_subscription_with_key(&device_token, &environment, e2e.as_ref())
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
            if !auth.allows(Scope::Prompt) {
                return err_response(
                    axum::http::StatusCode::FORBIDDEN,
                    "registering a web push endpoint needs the prompt scope",
                );
            }
            if let Err(msg) = validate_subscription(&endpoint, &keys) {
                return err_response(axum::http::StatusCode::BAD_REQUEST, msg);
            }
            owners::record(&auth, "webpush", &endpoint);
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
pub(crate) async fn push_unsubscribe(
    State(state): State<Arc<DaemonState>>,
    axum::Extension(auth): axum::Extension<ClientAuth>,
    axum::Json(req): axum::Json<SubscribeReq>,
) -> Response {
    let store = state.store.clone();
    match req {
        // `push_token` isn't needed to delete (session_id is the key) — it's only required by
        // the shared request shape's discriminator.
        SubscribeReq::LiveActivity { session_id, .. } => {
            if !owners::may_modify(&auth, "live", &session_id) {
                return err_response(
                    axum::http::StatusCode::FORBIDDEN,
                    "this session's live activity belongs to another device",
                );
            }
            owners::forget("live", &session_id);
            let _ =
                tokio::task::spawn_blocking(move || store.delete_live_activity_token(&session_id))
                    .await;
        }
        SubscribeReq::Apns { device_token, .. } => {
            if !owners::may_modify(&auth, "apns", &device_token) {
                return err_response(axum::http::StatusCode::FORBIDDEN, "not your subscription");
            }
            owners::forget("apns", &device_token);
            let _ =
                tokio::task::spawn_blocking(move || store.delete_apns_subscription(&device_token))
                    .await;
        }
        SubscribeReq::Webpush { endpoint, .. } => {
            if !owners::may_modify(&auth, "webpush", &endpoint) {
                return err_response(axum::http::StatusCode::FORBIDDEN, "not your subscription");
            }
            owners::forget("webpush", &endpoint);
            let _ = tokio::task::spawn_blocking(move || store.delete_push_subscription(&endpoint))
                .await;
        }
    }
    json_response(&serde_json::json!({ "ok": true }))
}

/// Hosts that run browser push services. A subscription endpoint is attacker-chosen text the
/// daemon later POSTs to, so anything else (loopback, LAN, link-local, IP literals, plain http)
/// is refused.
pub(crate) fn is_push_service_endpoint(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') || authority.contains('[') {
        return false;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if port.is_some_and(|port| port != "443") {
        return false;
    }
    let host = host.to_ascii_lowercase();
    const EXACT: &[&str] = &[
        "fcm.googleapis.com",
        "android.googleapis.com",
        "updates.push.services.mozilla.com",
        "web.push.apple.com",
    ];
    const SUFFIXES: &[&str] = &[
        ".push.services.mozilla.com",
        ".push.apple.com",
        ".notify.windows.com",
    ];
    EXACT.contains(&host.as_str()) || SUFFIXES.iter().any(|suffix| host.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::is_push_service_endpoint;

    #[test]
    fn only_https_push_services_are_accepted() {
        for ok in [
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://updates.push.services.mozilla.com/wpush/v2/x",
            "https://web.push.apple.com/QZ",
            "https://wns2-par02p.notify.windows.com/w/?token=x",
        ] {
            assert!(is_push_service_endpoint(ok), "{ok}");
        }
        for bad in [
            "http://fcm.googleapis.com/x",
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://169.254.169.254/latest",
            "https://192.168.1.1/apply.cgi",
            "https://fcm.googleapis.com.evil.example/x",
            "https://fcm.googleapis.com@evil.example/x",
            "https://evil.example/fcm.googleapis.com",
            "https://fcm.googleapis.com:8443/x",
            "https://[::1]/x",
            "https://notify.windows.com.evil.example/x",
        ] {
            assert!(!is_push_service_endpoint(bad), "{bad}");
        }
    }
}
