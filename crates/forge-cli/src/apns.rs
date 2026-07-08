//! Native iOS push delivery for the `forge serve` daemon via Apple's token-based provider API
//! (APNs HTTP/2), the counterpart to [`crate::push`] (Web Push/VAPID) for the standalone PWA.
//!
//! The two modules exist because they talk to fundamentally different networks. [`crate::push`]
//! self-encrypts a payload (RFC 8291) and POSTs it to whatever push endpoint the browser vendor
//! (Chrome/Firefox/etc.) handed the page. This module instead talks DIRECTLY to Apple's own
//! provider API — `api.push.apple.com` (or its sandbox twin) — carrying the alert/Live Activity
//! payload in the request body over HTTP/2, no per-browser-vendor indirection.
//!
//! ## Two environments, two hosts
//!
//! Apple splits APNs into two entirely separate "environments": **sandbox** (the host every
//! Xcode debug build and TestFlight build talks to) and **production** (the host an App Store
//! build talks to). A device token minted under one environment is rejected outright by the
//! other host, so every stored subscription ([`forge_store::ApnsSubscription`],
//! [`forge_store::LiveActivityToken`]) carries which environment it belongs to, and [`ApnsNotifier::host`]
//! routes each send accordingly.
//!
//! ## Auth: one long-lived JWT, not one-per-push
//!
//! [`crate::push::VapidKey::authorization`] mints a fresh JWT for every single push — that's
//! how VAPID (RFC 8292) works and browsers don't mind. Apple's own docs ask for the opposite:
//! mint one ES256 JWT (`iss`=team id, `iat`=now, header `kid`=key id) and reuse it for up to
//! roughly an hour, because Apple rate-limits how often a provider may request a fresh token.
//! [`ApnsAuth`] therefore caches the signed token and only re-mints once it ages past
//! [`AUTH_TOKEN_TTL_SECS`].

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;

use crate::push::b64url;

/// This daemon serves exactly one app, `dev.adulari.forge` — the same bundle id hardcoded in
/// `mobile/app.config.ts`.
const APNS_BUNDLE_ID: &str = "dev.adulari.forge";

/// How long a signed auth JWT stays valid before this module re-mints one. Apple recommends
/// reusing a token for up to ~1h and rate-limits how often you may request a fresh one; 50
/// minutes keeps clock-skew margin.
const AUTH_TOKEN_TTL_SECS: u64 = 50 * 60;

/// How long a fire-and-forget dispatch may take, all stored tokens included — mirrors
/// [`crate::push::DISPATCH_TIMEOUT`], same "never let a wedged push service pile up work behind
/// a busy session" contract.
const DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Credentials for Apple's token-based provider API: a `.p8` signing key plus the team/key ids
/// that identify it. Loaded from the environment, never from a required config file — APNs is an
/// optional feature, and its absence must degrade cleanly rather than block startup (mirrors how
/// `state.push` in `serve.rs` is `Option<PushNotifier>`).
pub(crate) struct ApnsConfig {
    team_id: String,
    key_id: String,
    key_pem: String,
}

impl ApnsConfig {
    /// Reads `FORGE_APNS_TEAM_ID`, `FORGE_APNS_KEY_ID`, `FORGE_APNS_KEY_PATH` (a path to the
    /// Apple-issued `.p8` private key file) from the environment. `None` if any is unset or the
    /// key file can't be read — the caller should skip wiring up APNs entirely, not error out.
    pub(crate) fn from_env() -> Option<Self> {
        let team_id = std::env::var("FORGE_APNS_TEAM_ID").ok()?;
        let key_id = std::env::var("FORGE_APNS_KEY_ID").ok()?;
        let key_path = std::env::var("FORGE_APNS_KEY_PATH").ok()?;
        let key_pem = std::fs::read_to_string(key_path).ok()?;
        Some(Self {
            team_id,
            key_id,
            key_pem,
        })
    }

    /// Build directly from an in-memory PEM string, bypassing environment variables and the
    /// filesystem entirely — tests only, so a real `.p8` file/account isn't needed to exercise
    /// the subscribe/unsubscribe routes end-to-end.
    #[cfg(test)]
    pub(crate) fn from_pem_for_test(key_pem: &str, team_id: &str, key_id: &str) -> Self {
        Self {
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            key_pem: key_pem.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Auth JWT (ES256), cached
// ---------------------------------------------------------------------------

/// Signs and caches the `authorization: bearer <jwt>` token every APNs request carries. Unlike
/// [`crate::push::VapidKey`] (a fresh JWT per push), this mints one and reuses it — see the
/// module docs for why.
pub(crate) struct ApnsAuth {
    signing_key: SigningKey,
    team_id: String,
    key_id: String,
    cached: std::sync::Mutex<Option<(String, u64)>>,
}

impl ApnsAuth {
    /// Parse the `.p8` PEM key out of an [`ApnsConfig`].
    pub(crate) fn new(config: &ApnsConfig) -> anyhow::Result<Self> {
        let secret = p256::SecretKey::from_pkcs8_pem(&config.key_pem)
            .map_err(|e| anyhow::anyhow!("invalid APNs .p8 key: {e}"))?;
        Ok(Self {
            signing_key: SigningKey::from(secret),
            team_id: config.team_id.clone(),
            key_id: config.key_id.clone(),
            cached: std::sync::Mutex::new(None),
        })
    }

    /// An auth from a raw scalar, bypassing PEM parsing entirely (tests only — mirrors
    /// [`crate::push::VapidKey::from_scalar`]).
    #[cfg(test)]
    pub(crate) fn from_scalar(bytes: &[u8], team_id: &str, key_id: &str) -> Self {
        Self {
            signing_key: SigningKey::from(
                p256::SecretKey::from_slice(bytes).expect("valid P-256 scalar"),
            ),
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            cached: std::sync::Mutex::new(None),
        }
    }

    /// The `authorization: bearer <jwt>` header value, minting a fresh JWT only when the cached
    /// one has aged past [`AUTH_TOKEN_TTL_SECS`].
    pub(crate) fn bearer_token(&self, now_unix: u64) -> String {
        let mut cached = self.cached.lock().unwrap();
        if let Some((tok, minted_at)) = cached.as_ref() {
            if now_unix.saturating_sub(*minted_at) < AUTH_TOKEN_TTL_SECS {
                return tok.clone();
            }
        }
        let header = b64url(format!(r#"{{"alg":"ES256","kid":"{}"}}"#, self.key_id).as_bytes());
        let claims = b64url(
            serde_json::json!({ "iss": self.team_id, "iat": now_unix })
                .to_string()
                .as_bytes(),
        );
        let signing_input = format!("{header}.{claims}");
        let sig: Signature = self.signing_key.sign(signing_input.as_bytes());
        let token = format!("{signing_input}.{}", b64url(&sig.to_bytes()));
        *cached = Some((token.clone(), now_unix));
        token
    }
}

// ---------------------------------------------------------------------------
// Payload builders
// ---------------------------------------------------------------------------

/// The APNs alert payload for a [`crate::push::PushMessage`] — same trigger events (new
/// permission/question, turn done/failed) as Web Push, different wire format.
pub(crate) fn alert_payload(msg: &crate::push::PushMessage) -> serde_json::Value {
    serde_json::json!({
        "aps": {
            "alert": { "title": msg.title, "body": msg.body },
            "sound": "default",
            "mutable-content": 1,
        },
        "session": msg.session,
        "kind": msg.kind,
        "seq": msg.seq,
    })
}

/// Content-state pushed to an active Live Activity — kept intentionally small and stable;
/// changing these field names/types requires updating the matching Swift
/// `ActivityAttributes.ContentState` in the mobile app's widget extension (`mobile/targets/widget`)
/// in lockstep, since there is no shared schema between the two languages.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct LiveActivityContentState {
    pub busy: bool,
    pub waiting: bool,
    pub cost_usd: f64,
    pub context_tokens: u64,
}

/// The APNs `event: update` payload for one [`LiveActivityContentState`], per Apple's
/// `content-state` push format.
pub(crate) fn live_activity_payload(
    content_state: &LiveActivityContentState,
    now_unix: u64,
) -> serde_json::Value {
    serde_json::json!({
        "aps": {
            "timestamp": now_unix,
            "event": "update",
            "content-state": content_state,
        }
    })
}

// ---------------------------------------------------------------------------
// The sender
// ---------------------------------------------------------------------------

/// Owns the cached auth JWT, the store (device tokens + Live Activity tokens), and an HTTP
/// client; delivers alert pushes and Live Activity updates, pruning dead tokens exactly as
/// [`crate::push::PushNotifier`] prunes dead Web Push subscriptions.
pub(crate) struct ApnsNotifier {
    auth: ApnsAuth,
    store: std::sync::Arc<forge_store::Store>,
    client: reqwest::Client,
}

impl ApnsNotifier {
    pub(crate) fn new(
        store: std::sync::Arc<forge_store::Store>,
        config: ApnsConfig,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            auth: ApnsAuth::new(&config)?,
            store,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .map_err(|e| anyhow::anyhow!("building reqwest client for apns notifier: {e}"))?,
        })
    }

    /// The host for a stored subscription's environment — "production" routes to the App Store
    /// host, anything else (including "sandbox" and any unrecognized value) routes to sandbox,
    /// since a misrouted sandbox token merely fails rather than reaching the wrong audience.
    fn host(environment: &str) -> &'static str {
        if environment == "production" {
            "https://api.push.apple.com"
        } else {
            "https://api.sandbox.push.apple.com"
        }
    }

    /// Fire-and-forget delivery of an alert [`crate::push::PushMessage`] to every stored device
    /// token — same "never block/delay/fail the turn" contract as
    /// [`crate::push::PushNotifier::dispatch`].
    pub(crate) fn dispatch_alert(self: &std::sync::Arc<Self>, msg: crate::push::PushMessage) {
        let this = self.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(DISPATCH_TIMEOUT, this.send_alert_all(msg)).await;
        });
    }

    async fn send_alert_all(&self, msg: crate::push::PushMessage) {
        let store = self.store.clone();
        let subs = tokio::task::spawn_blocking(move || {
            store.list_apns_subscriptions().unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let payload = alert_payload(&msg);
        for sub in subs {
            match self
                .send_one(
                    &sub.device_token,
                    &sub.environment,
                    "alert",
                    &payload,
                    APNS_BUNDLE_ID,
                )
                .await
            {
                // 410 (Unregistered/BadDeviceToken): the token is gone — prune it, mirroring
                // push.rs's 404/410 pruning of dead Web Push subscriptions.
                Ok(410) => {
                    let store = self.store.clone();
                    let token = sub.device_token.clone();
                    let _ =
                        tokio::task::spawn_blocking(move || store.delete_apns_subscription(&token))
                            .await;
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("apns alert to {} failed: {e}", sub.device_token),
            }
        }
    }

    /// Fire-and-forget Live Activity content-state update for one session, if it has a stored
    /// activity push token ([`forge_store::LiveActivityToken`]) — a no-op when it doesn't.
    pub(crate) fn dispatch_live_activity(
        self: &std::sync::Arc<Self>,
        session_id: String,
        content_state: LiveActivityContentState,
    ) {
        let this = self.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                DISPATCH_TIMEOUT,
                this.send_live_activity(session_id, content_state),
            )
            .await;
        });
    }

    async fn send_live_activity(
        &self,
        session_id: String,
        content_state: LiveActivityContentState,
    ) {
        let store = self.store.clone();
        let sid = session_id.clone();
        let token = tokio::task::spawn_blocking(move || {
            store.get_live_activity_token(&sid).unwrap_or(None)
        })
        .await
        .unwrap_or(None);
        let Some(token) = token else { return };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let payload = live_activity_payload(&content_state, now);
        let topic = format!("{APNS_BUNDLE_ID}.push-type.liveactivity");
        match self
            .send_one(
                &token.push_token,
                &token.environment,
                "liveactivity",
                &payload,
                &topic,
            )
            .await
        {
            Ok(410) => {
                let store = self.store.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    store.delete_live_activity_token(&session_id)
                })
                .await;
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("apns live activity to session {session_id} failed: {e}"),
        }
    }

    async fn send_one(
        &self,
        device_token: &str,
        environment: &str,
        push_type: &str,
        payload: &serde_json::Value,
        topic: &str,
    ) -> anyhow::Result<u16> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let url = format!("{}/3/device/{device_token}", Self::host(environment));
        let resp = self
            .client
            .post(&url)
            .header(
                "authorization",
                format!("bearer {}", self.auth.bearer_token(now)),
            )
            .header("apns-topic", topic)
            .header("apns-push-type", push_type)
            .header("apns-priority", "10")
            .json(payload)
            .send()
            .await?;
        Ok(resp.status().as_u16())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;

    fn b64d(s: &str) -> Vec<u8> {
        crate::push::b64url_decode(s).expect("valid base64url")
    }

    /// The APNs auth JWT must verify with the corresponding public key and carry the right
    /// claims — this is exactly the check Apple's servers perform before accepting the POST.
    /// Mirrors push.rs's `vapid_jwt_is_verifiable_with_the_public_key`.
    #[test]
    fn bearer_token_is_verifiable_with_the_public_key() {
        let scalar = [9u8; 32];
        let auth = ApnsAuth::from_scalar(&scalar, "TEAM123456", "KEY7890AB");
        let token = auth.bearer_token(1_700_000_000);

        let mut parts = token.split('.');
        let (h, c, s) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        assert!(parts.next().is_none(), "exactly three JWT segments");

        let header: serde_json::Value = serde_json::from_slice(&b64d(h)).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "KEY7890AB");

        let claims: serde_json::Value = serde_json::from_slice(&b64d(c)).unwrap();
        assert_eq!(claims["iss"], "TEAM123456");
        assert_eq!(claims["iat"], 1_700_000_000u64);

        let sig_bytes = b64d(s);
        let sig = Signature::from_slice(&sig_bytes).expect("64-byte r||s signature");
        let secret = p256::SecretKey::from_slice(&scalar).unwrap();
        let vk = VerifyingKey::from(secret.public_key());
        vk.verify(format!("{h}.{c}").as_bytes(), &sig)
            .expect("signature verifies with the advertised key");
    }

    #[test]
    fn bearer_token_is_cached_and_reminted_after_ttl() {
        let auth = ApnsAuth::from_scalar(&[3u8; 32], "TEAM", "KEY");
        let first = auth.bearer_token(1_000_000);
        let same = auth.bearer_token(1_000_000 + AUTH_TOKEN_TTL_SECS - 1);
        assert_eq!(first, same, "reused while within the TTL");
        let fresh = auth.bearer_token(1_000_000 + AUTH_TOKEN_TTL_SECS);
        assert_ne!(first, fresh, "re-minted once the cached token goes stale");
    }

    #[test]
    fn host_routes_production_and_everything_else_to_sandbox() {
        assert_eq!(
            ApnsNotifier::host("production"),
            "https://api.push.apple.com"
        );
        assert_eq!(
            ApnsNotifier::host("sandbox"),
            "https://api.sandbox.push.apple.com"
        );
        assert_eq!(
            ApnsNotifier::host("garbage"),
            "https://api.sandbox.push.apple.com"
        );
    }

    #[test]
    fn alert_payload_carries_the_message_fields() {
        let msg = crate::push::PushMessage {
            kind: "permission",
            session: "sess-1".into(),
            title: "fix the parser".into(),
            body: "allow write_file".into(),
            seq: 7,
            ttl: 300,
        };
        let v = alert_payload(&msg);
        assert_eq!(v["aps"]["alert"]["title"], "fix the parser");
        assert_eq!(v["aps"]["alert"]["body"], "allow write_file");
        assert_eq!(v["session"], "sess-1");
        assert_eq!(v["kind"], "permission");
        assert_eq!(v["seq"], 7);
    }

    #[test]
    fn live_activity_payload_round_trips_the_content_state() {
        let content_state = LiveActivityContentState {
            busy: true,
            waiting: false,
            cost_usd: 1.23,
            context_tokens: 4567,
        };
        let v = live_activity_payload(&content_state, 1_700_000_123);
        assert_eq!(v["aps"]["event"], "update");
        assert_eq!(v["aps"]["timestamp"], 1_700_000_123u64);
        let cs = &v["aps"]["content-state"];
        assert_eq!(cs["busy"], true);
        assert_eq!(cs["waiting"], false);
        assert_eq!(cs["cost_usd"], 1.23);
        assert_eq!(cs["context_tokens"], 4567);
    }

    /// `ApnsConfig::from_env` must degrade to `None` — never panic or error — when the feature
    /// isn't configured; saves/restores the three vars so this doesn't leak state into whatever
    /// else runs in this test binary.
    #[test]
    fn config_from_env_is_none_without_all_three_vars() {
        let vars = [
            "FORGE_APNS_TEAM_ID",
            "FORGE_APNS_KEY_ID",
            "FORGE_APNS_KEY_PATH",
        ];
        let saved: Vec<Option<String>> = vars.iter().map(|v| std::env::var(v).ok()).collect();
        for v in vars {
            std::env::remove_var(v);
        }

        assert!(ApnsConfig::from_env().is_none());

        for (v, val) in vars.iter().zip(saved) {
            match val {
                Some(val) => std::env::set_var(v, val),
                None => std::env::remove_var(v),
            }
        }
    }
}
