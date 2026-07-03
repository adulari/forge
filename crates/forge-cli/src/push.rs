//! Actionable Web Push for the `forge serve` daemon (docs/features/remote-control.md §2d).
//!
//! Self-hosted, no vendor relay of any kind: Forge holds its own VAPID (RFC 8292) keypair,
//! encrypts every payload end-to-end per RFC 8291 (aes128gcm), and POSTs the opaque ciphertext
//! DIRECTLY to the browser vendor's push endpoint the subscription names — that endpoint is how
//! Web Push works and it can't read a byte of the message. The notification fires when a session
//! **needs a decision** (permission prompt / question), finishes a turn, or fails — and carries
//! approve/deny actions so the operator can unblock the agent from the lock screen without
//! opening the page (the service worker answers `POST /<t>/api/answer` itself).
//!
//! ## Debounce
//!
//! No push is sent while ANY WebSocket client is connected to the session ([`should_push`]).
//! This is deliberately the simpler of the two designs considered (the other: have the page
//! report `document.visibilityState` over the WS): a phone that locks or backgrounds the PWA
//! drops its WS within seconds, so "a WS is connected" is a reliable proxy for "someone is
//! watching" — while visibility reporting adds a protocol field and still lies for an unattended
//! wall dashboard. Documented trade-off: a desktop tab left open in the background suppresses
//! pushes to the phone.
//!
//! ## Delivery
//!
//! Strictly best-effort and never in the turn's way: [`PushNotifier::dispatch`] spawns a
//! bounded fire-and-forget task ([`DISPATCH_TIMEOUT`]); a push endpoint that answers 404/410
//! (subscription expired/revoked) gets its row pruned from the store.

use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use anyhow::Context;
use base64::Engine;
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::Sha256;

use crate::remote::Snapshot;

/// The persisted VAPID private key's file name inside the forge config dir (hex scalar, 0600).
const VAPID_KEY_FILE: &str = "vapid-key";

/// RFC 8292 `sub` claim — a stable contact URI a push service can use to reach the operator of
/// the application server (us). Not a tracking endpoint; some services require it to be set.
const VAPID_SUBJECT: &str = "https://github.com/Adulari/forge";

/// VAPID JWT lifetime. RFC 8292 caps it at 24h; 12h keeps clock-skew margins comfortable.
const VAPID_EXP_SECS: u64 = 12 * 60 * 60;

/// How long a fire-and-forget dispatch may take, all subscriptions included. A wedged push
/// service must never pile tasks up behind a busy session.
const DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// TTL for decision pushes (permission/question): after a few minutes a stale "Allow?" on the
/// lock screen is more likely to mislead than help.
const TTL_DECISION: u32 = 300;

/// TTL for completion pushes (turn done / failed): still worth seeing an hour later.
const TTL_COMPLETION: u32 = 3600;

/// Notification body cap — push services reject large payloads (~4KB) and lock screens show
/// roughly two lines anyway.
const BODY_MAX_CHARS: usize = 160;

/// The RFC 8188 record size we write into the aes128gcm header. One record fits every payload
/// we send, so the value only needs to be ≥ the ciphertext length; 4096 is the conventional pick.
const RECORD_SIZE: u32 = 4096;

pub(crate) fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

pub(crate) fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim())
        .ok()
}

// ---------------------------------------------------------------------------
// VAPID keypair (RFC 8292)
// ---------------------------------------------------------------------------

/// The daemon's ES256 (P-256) VAPID keypair. Generated ONCE and persisted 0600 in the config
/// dir: browsers bind a push subscription to the application server key, so rotating it would
/// silently orphan every existing subscription.
pub(crate) struct VapidKey {
    secret: p256::SecretKey,
}

impl VapidKey {
    /// Read (or mint) the persisted key from the forge config dir.
    pub(crate) fn load_or_create() -> anyhow::Result<Self> {
        let dir = forge_config::config_dir()
            .ok_or_else(|| anyhow::anyhow!("no config directory on this platform"))?;
        Self::load_or_create_at(&dir.join(VAPID_KEY_FILE))
    }

    /// [`Self::load_or_create`] against an explicit path (unit-testable without touching the
    /// real config). The file holds the 32-byte P-256 scalar as 64 lowercase hex chars; a
    /// corrupted file is replaced, never trusted.
    pub(crate) fn load_or_create_at(path: &std::path::Path) -> anyhow::Result<Self> {
        if let Ok(existing) = std::fs::read_to_string(path) {
            if let Some(secret) = parse_hex_scalar(existing.trim()) {
                return Ok(Self { secret });
            }
        }
        let secret = random_secret_key();
        let hex: String = secret
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, &hex)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self { secret })
    }

    /// A key from raw scalar bytes (tests + the RFC 8291 vector).
    #[cfg(test)]
    pub(crate) fn from_scalar(bytes: &[u8]) -> Self {
        Self {
            secret: p256::SecretKey::from_slice(bytes).expect("valid P-256 scalar"),
        }
    }

    /// The public key as the base64url (unpadded) uncompressed SEC1 point — exactly what
    /// `PushManager.subscribe({applicationServerKey})` wants and what `k=` carries.
    pub(crate) fn public_key_b64url(&self) -> String {
        b64url(self.secret.public_key().to_encoded_point(false).as_bytes())
    }

    /// The `Authorization: vapid t=<jwt>, k=<pub>` header value for a push `endpoint`
    /// (RFC 8292 §3). `None` when the endpoint has no parsable origin (the `aud` claim).
    pub(crate) fn authorization(&self, endpoint: &str, now_unix: u64) -> Option<String> {
        let aud = endpoint_origin(endpoint)?;
        let header = b64url(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = b64url(
            serde_json::json!({
                "aud": aud,
                "exp": now_unix + VAPID_EXP_SECS,
                "sub": VAPID_SUBJECT,
            })
            .to_string()
            .as_bytes(),
        );
        let signing_input = format!("{header}.{claims}");
        let sig: Signature = SigningKey::from(&self.secret).sign(signing_input.as_bytes());
        Some(format!(
            "vapid t={signing_input}.{}, k={}",
            b64url(&sig.to_bytes()),
            self.public_key_b64url()
        ))
    }
}

/// Parse a 64-hex-char P-256 scalar; `None` for anything malformed or out of range.
fn parse_hex_scalar(s: &str) -> Option<p256::SecretKey> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let bytes: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
        .collect();
    p256::SecretKey::from_slice(&bytes).ok()
}

/// A fresh P-256 secret from the OS CSPRNG (`rand::random` — same source as the daemon token).
/// Rejection-samples the (astronomically unlikely, probability < 2⁻³²) out-of-range scalar.
fn random_secret_key() -> p256::SecretKey {
    loop {
        let bytes: [u8; 32] = rand::random();
        if let Ok(k) = p256::SecretKey::from_slice(&bytes) {
            return k;
        }
    }
}

/// The `scheme://host[:port]` origin of a push endpoint URL — the VAPID `aud` claim. `None`
/// for non-http(s) schemes or a missing host.
pub(crate) fn endpoint_origin(endpoint: &str) -> Option<String> {
    let (scheme, rest) = endpoint.split_once("://")?;
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}"))
}

// ---------------------------------------------------------------------------
// RFC 8291 payload encryption (aes128gcm)
// ---------------------------------------------------------------------------

/// Encrypt `plaintext` for the subscription identified by `ua_public` (the browser's `p256dh`,
/// a 65-byte uncompressed P-256 point) + `auth_secret` (its 16-byte `auth`), per RFC 8291.
/// A fresh ephemeral sender key + salt per message, so no two payloads share key material.
pub(crate) fn encrypt_payload(
    ua_public: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> anyhow::Result<Vec<u8>> {
    let salt: [u8; 16] = rand::random();
    encrypt_with(
        &random_secret_key(),
        &salt,
        ua_public,
        auth_secret,
        plaintext,
    )
}

/// [`encrypt_payload`] with an explicit sender key + salt — the seam that lets the RFC 8291 §5
/// test vector (fixed `as_private`, fixed salt) verify this implementation byte-for-byte.
fn encrypt_with(
    as_secret: &p256::SecretKey,
    salt: &[u8; 16],
    ua_public: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> anyhow::Result<Vec<u8>> {
    let ua_pub = p256::PublicKey::from_sec1_bytes(ua_public)
        .map_err(|e| anyhow::anyhow!("invalid p256dh key: {e}"))?;
    let as_public = as_secret.public_key().to_encoded_point(false);
    let as_public = as_public.as_bytes();
    let ecdh = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_pub.as_affine());

    // IKM = HKDF-Expand(HKDF-Extract(auth_secret, ecdh_secret),
    //                   "WebPush: info" || 0x00 || ua_public || as_public, 32)
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth_secret), ecdh.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|e| anyhow::anyhow!("hkdf ikm: {e}"))?;

    // CEK/NONCE from the per-message salt (RFC 8188 key schedule).
    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|e| anyhow::anyhow!("hkdf cek: {e}"))?;
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|e| anyhow::anyhow!("hkdf nonce: {e}"))?;

    // One record: plaintext || 0x02 (the last-record delimiter), AES-128-GCM sealed.
    let mut record = Vec::with_capacity(plaintext.len() + 1);
    record.extend_from_slice(plaintext);
    record.push(0x02);
    let cipher = Aes128Gcm::new_from_slice(&cek).expect("cek is 16 bytes");
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), record.as_slice())
        .map_err(|e| anyhow::anyhow!("aes128gcm seal: {e}"))?;

    // aes128gcm header: salt(16) || rs(4) || idlen(1) || keyid(as_public, 65) || ciphertext.
    let mut out = Vec::with_capacity(16 + 4 + 1 + 65 + ciphertext.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(as_public);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// The browser side of [`encrypt_payload`], test-only: decrypt an aes128gcm message with the
/// receiver's private key + auth secret, returning the plaintext. Lets the e2e test assert the
/// bytes that left the daemon really decrypt to the intended notification payload.
#[cfg(test)]
pub(crate) fn decrypt_payload(
    ua_secret: &p256::SecretKey,
    auth_secret: &[u8],
    message: &[u8],
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(message.len() > 16 + 4 + 1 + 65 + 16, "message too short");
    let salt = &message[..16];
    let idlen = message[20] as usize;
    anyhow::ensure!(idlen == 65, "keyid must be an uncompressed P-256 point");
    let as_public = &message[21..21 + 65];
    let ciphertext = &message[21 + 65..];

    let as_pub = p256::PublicKey::from_sec1_bytes(as_public)?;
    let ua_public = ua_secret.public_key().to_encoded_point(false);
    let ecdh = p256::ecdh::diffie_hellman(ua_secret.to_nonzero_scalar(), as_pub.as_affine());
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(ua_public.as_bytes());
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth_secret), ecdh.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|e| anyhow::anyhow!("hkdf ikm: {e}"))?;
    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|e| anyhow::anyhow!("hkdf cek: {e}"))?;
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|e| anyhow::anyhow!("hkdf nonce: {e}"))?;
    let cipher = Aes128Gcm::new_from_slice(&cek).expect("cek is 16 bytes");
    let mut record = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext)
        .map_err(|e| anyhow::anyhow!("aes128gcm open: {e}"))?;
    // Strip the record delimiter (0x02 for the last record) + any zero padding after it.
    while record.last() == Some(&0) {
        record.pop();
    }
    anyhow::ensure!(record.pop() == Some(0x02), "missing last-record delimiter");
    Ok(record)
}

// ---------------------------------------------------------------------------
// Triggers + debounce (pure decision fns — unit-tested)
// ---------------------------------------------------------------------------

/// One notification-worthy event, ready to encrypt and send.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PushMessage {
    /// `"permission"` | `"question"` | `"done"` | `"failed"` — the SW picks actions from this.
    pub kind: &'static str,
    pub session: String,
    /// The session's display name (title, or the short id).
    pub title: String,
    pub body: String,
    /// The [`Snapshot::prompt_seq`] the approve/deny actions must echo (decision kinds only).
    pub seq: u64,
    /// Push-service retention (`TTL` header), seconds.
    pub ttl: u32,
}

impl PushMessage {
    /// The cleartext JSON the service worker's `push` handler reads (encrypted on the wire).
    pub(crate) fn payload_json(&self) -> String {
        serde_json::json!({
            "kind": self.kind,
            "session": self.session,
            "title": self.title,
            "body": self.body,
            "seq": self.seq,
        })
        .to_string()
    }
}

/// True when a push may fire at all: only with ZERO WebSocket clients attached to the session
/// (see the module docs for why "any client connected" is the chosen debounce signal).
pub(crate) fn should_push(connected_ws_clients: usize) -> bool {
    connected_ws_clients == 0
}

/// Decide whether the `prev → cur` snapshot transition is notification-worthy. Pure, so the
/// trigger rules are unit-testable:
/// - a NEW permission prompt / question (a bumped `prompt_seq` — a replaced prompt re-fires,
///   the same pending prompt never does),
/// - a turn finishing (`busy` falling edge with nothing pending) — `"failed"` when the turn
///   surfaced a [`forge_tui::PresenterEvent::Error`] (`turn_error`), else `"done"`,
/// - never on `closed` frames (archiving isn't news).
pub(crate) fn detect_trigger(
    prev: Option<&Snapshot>,
    cur: &Snapshot,
    turn_error: Option<&str>,
) -> Option<PushMessage> {
    if cur.closed {
        return None;
    }
    let title = if cur.title.is_empty() {
        let short: String = cur.session_id.chars().take(8).collect();
        format!("session {short}")
    } else {
        cur.title.clone()
    };
    let seq_is_new = prev.is_none_or(|p| p.prompt_seq != cur.prompt_seq);
    if seq_is_new {
        if let Some(q) = &cur.question {
            return Some(PushMessage {
                kind: "question",
                session: cur.session_id.clone(),
                title,
                body: truncate_chars(q, BODY_MAX_CHARS),
                seq: cur.prompt_seq,
                ttl: TTL_DECISION,
            });
        }
        if let Some(p) = &cur.permission_prompt {
            return Some(PushMessage {
                kind: "permission",
                session: cur.session_id.clone(),
                title,
                body: truncate_chars(p, BODY_MAX_CHARS),
                seq: cur.prompt_seq,
                ttl: TTL_DECISION,
            });
        }
    }
    if prev.is_some_and(|p| p.busy)
        && !cur.busy
        && cur.permission_prompt.is_none()
        && cur.question.is_none()
    {
        let (kind, body) = match turn_error {
            Some(err) => ("failed", truncate_chars(err, BODY_MAX_CHARS)),
            None => (
                "done",
                truncate_chars(
                    cur.transcript
                        .iter()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .map(String::as_str)
                        .unwrap_or("turn complete"),
                    BODY_MAX_CHARS,
                ),
            ),
        };
        return Some(PushMessage {
            kind,
            session: cur.session_id.clone(),
            title,
            body,
            seq: cur.prompt_seq,
            ttl: TTL_COMPLETION,
        });
    }
    None
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

// ---------------------------------------------------------------------------
// The sender
// ---------------------------------------------------------------------------

/// Owns the VAPID key, the store (subscription list), and an HTTP client; encrypts and delivers
/// one [`PushMessage`] to every stored subscription, pruning the dead ones.
pub(crate) struct PushNotifier {
    vapid: VapidKey,
    store: Arc<forge_store::Store>,
    client: reqwest::Client,
}

impl PushNotifier {
    pub(crate) fn new(store: Arc<forge_store::Store>) -> anyhow::Result<Self> {
        Self::with_key(store, VapidKey::load_or_create()?)
    }

    pub(crate) fn with_key(
        store: Arc<forge_store::Store>,
        vapid: VapidKey,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            vapid,
            store,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .context("building reqwest client for push notifier")?,
        })
    }

    pub(crate) fn public_key_b64url(&self) -> String {
        self.vapid.public_key_b64url()
    }

    /// Fire-and-forget delivery to every subscription: spawned, time-boxed, and swallowing every
    /// error — a push must NEVER block, delay, or fail the turn that triggered it.
    pub(crate) fn dispatch(self: &Arc<Self>, msg: PushMessage) {
        let this = self.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(DISPATCH_TIMEOUT, this.send_all(msg)).await;
        });
    }

    async fn send_all(&self, msg: PushMessage) {
        let store = self.store.clone();
        let subs = tokio::task::spawn_blocking(move || {
            store.list_push_subscriptions().unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let payload = msg.payload_json();
        for sub in subs {
            match self.send_one(&sub, payload.as_bytes(), msg.ttl).await {
                // 404/410: the subscription is gone (uninstalled / permission revoked / rotated
                // by the browser) — prune it so we stop knocking.
                Ok(status) if status == 404 || status == 410 => {
                    let store = self.store.clone();
                    let endpoint = sub.endpoint.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.delete_push_subscription(&endpoint)
                    })
                    .await;
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("push to {} failed: {e}", sub.endpoint),
            }
        }
    }

    async fn send_one(
        &self,
        sub: &forge_store::PushSubscription,
        payload: &[u8],
        ttl: u32,
    ) -> anyhow::Result<u16> {
        let ua_public = b64url_decode(&sub.p256dh)
            .ok_or_else(|| anyhow::anyhow!("subscription p256dh is not base64url"))?;
        let auth = b64url_decode(&sub.auth)
            .ok_or_else(|| anyhow::anyhow!("subscription auth is not base64url"))?;
        let body = encrypt_payload(&ua_public, &auth, payload)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let authz = self
            .vapid
            .authorization(&sub.endpoint, now)
            .ok_or_else(|| anyhow::anyhow!("endpoint has no parsable origin"))?;
        let resp = self
            .client
            .post(&sub.endpoint)
            .header("Authorization", authz)
            .header("TTL", ttl.to_string())
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("Urgency", "high")
            .body(body)
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
        b64url_decode(s).expect("valid base64url")
    }

    /// RFC 8291 §5 / Appendix A: with the vector's fixed sender key + salt, our encryption must
    /// reproduce the published message byte-for-byte. Matching 144 bytes of ciphertext leaves no
    /// room for a wrong HKDF info string, key schedule, delimiter, or header layout.
    #[test]
    fn rfc8291_test_vector_reproduces_exactly() {
        let ua_public = b64d(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let auth_secret = b64d("BTBZMqHH6r4Tts7J_aSIgg");
        let as_private = b64d("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw");
        let salt: [u8; 16] = b64d("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let plaintext = b"When I grow up, I want to be a watermelon";
        let expected = b64d(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLoc\
             InmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLV\
             WGNWQexSgSxsj_Qulcy4a-fN",
        );
        let as_secret = p256::SecretKey::from_slice(&as_private).unwrap();
        let got = encrypt_with(&as_secret, &salt, &ua_public, &auth_secret, plaintext).unwrap();
        assert_eq!(got, expected, "RFC 8291 §5 message must match exactly");
    }

    /// The full round trip through both directions of the RFC 8291 construction: what
    /// [`encrypt_payload`] seals, the browser-side [`decrypt_payload`] opens.
    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let ua_secret = p256::SecretKey::from_slice(&[11u8; 32]).unwrap();
        let ua_public = ua_secret.public_key().to_encoded_point(false);
        let auth: [u8; 16] = [5u8; 16];
        let plaintext = br#"{"kind":"permission","seq":3}"#;
        let sealed = encrypt_payload(ua_public.as_bytes(), &auth, plaintext).unwrap();
        assert_eq!(
            decrypt_payload(&ua_secret, &auth, &sealed).unwrap(),
            plaintext
        );
        // A tampered byte must fail authentication, never decrypt quietly.
        let mut bad = sealed.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(decrypt_payload(&ua_secret, &auth, &bad).is_err());
    }

    /// Every real send uses a fresh ephemeral key + salt — same plaintext, different ciphertext,
    /// and the output stays structurally valid (header fields where RFC 8188 puts them).
    #[test]
    fn encrypt_payload_is_randomized_and_well_formed() {
        let ua_secret = p256::SecretKey::from_slice(&[7u8; 32]).unwrap();
        let ua_public = ua_secret.public_key().to_encoded_point(false);
        let auth = [3u8; 16];
        let a = encrypt_payload(ua_public.as_bytes(), &auth, b"hi").unwrap();
        let b = encrypt_payload(ua_public.as_bytes(), &auth, b"hi").unwrap();
        assert_ne!(a, b, "fresh salt + ephemeral key per message");
        // Header layout: salt(16) || rs(4) || idlen(1)=65 || keyid(65) || ct(2+1+16).
        assert_eq!(u32::from_be_bytes(a[16..20].try_into().unwrap()), 4096);
        assert_eq!(a[20], 65);
        assert_eq!(a[21], 0x04, "keyid is an uncompressed point");
        assert_eq!(a.len(), 16 + 4 + 1 + 65 + (2 + 1 + 16));
    }

    /// The VAPID JWT must verify with the advertised public key and carry the right claims —
    /// this is exactly the check a push service performs before accepting the POST.
    #[test]
    fn vapid_jwt_is_verifiable_with_the_public_key() {
        let key = VapidKey::from_scalar(&[9u8; 32]);
        let authz = key
            .authorization("https://push.example.net/w/abc123?x=1", 1_700_000_000)
            .unwrap();
        let t = authz
            .strip_prefix("vapid t=")
            .and_then(|r| r.split(", k=").next())
            .expect("t= present");
        let k = authz.split(", k=").nth(1).expect("k= present");
        assert_eq!(k, key.public_key_b64url());

        let mut parts = t.split('.');
        let (h, c, s) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        assert!(parts.next().is_none(), "exactly three JWT segments");
        let header: serde_json::Value = serde_json::from_slice(&b64d(h)).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["typ"], "JWT");
        let claims: serde_json::Value = serde_json::from_slice(&b64d(c)).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["exp"], 1_700_000_000u64 + VAPID_EXP_SECS);
        assert_eq!(claims["sub"], VAPID_SUBJECT);

        let sig_bytes = b64d(s);
        let sig = Signature::from_slice(&sig_bytes).expect("64-byte r||s signature");
        let vk = VerifyingKey::from_sec1_bytes(&b64d(&key.public_key_b64url())).unwrap();
        vk.verify(format!("{h}.{c}").as_bytes(), &sig)
            .expect("signature verifies with the advertised key");
    }

    #[test]
    fn vapid_key_persists_and_replaces_corruption() {
        let dir = std::env::temp_dir().join(format!("forge-vapid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vapid-key");
        let k1 = VapidKey::load_or_create_at(&path).unwrap();
        let k2 = VapidKey::load_or_create_at(&path).unwrap();
        assert_eq!(
            k1.public_key_b64url(),
            k2.public_key_b64url(),
            "the key is stable across restarts — subscriptions are bound to it"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "private key file is owner-only");
        }
        std::fs::write(&path, "junk").unwrap();
        let k3 = VapidKey::load_or_create_at(&path).unwrap();
        assert_ne!(
            k1.public_key_b64url(),
            k3.public_key_b64url(),
            "a corrupted key file is replaced, never trusted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn endpoint_origin_extracts_scheme_and_host() {
        assert_eq!(
            endpoint_origin("https://fcm.googleapis.com/fcm/send/xyz:APA91").as_deref(),
            Some("https://fcm.googleapis.com")
        );
        assert_eq!(
            endpoint_origin("http://127.0.0.1:8901/push/1").as_deref(),
            Some("http://127.0.0.1:8901")
        );
        assert_eq!(endpoint_origin("ftp://x.example/1"), None);
        assert_eq!(endpoint_origin("https:///nohost"), None);
        assert_eq!(endpoint_origin("no-scheme"), None);
    }

    #[test]
    fn should_push_only_with_zero_clients() {
        assert!(should_push(0));
        assert!(!should_push(1), "a connected client debounces the push");
        assert!(!should_push(3));
    }

    fn snap() -> Snapshot {
        Snapshot {
            session_id: "sess12345678".into(),
            ..Default::default()
        }
    }

    #[test]
    fn trigger_fires_on_a_new_permission_prompt_and_not_on_the_same_one() {
        let prev = Snapshot {
            busy: true,
            ..snap()
        };
        let cur = Snapshot {
            busy: true,
            permission_prompt: Some("allow write_file (Write) [y/n]".into()),
            prompt_seq: 1,
            ..snap()
        };
        let msg = detect_trigger(Some(&prev), &cur, None).expect("new prompt pushes");
        assert_eq!(msg.kind, "permission");
        assert_eq!(msg.seq, 1);
        assert_eq!(msg.ttl, TTL_DECISION);
        assert!(msg.body.contains("write_file"));
        assert_eq!(msg.title, "session sess1234", "short id when untitled");
        // The SAME pending prompt (seq unchanged) never re-fires on later frames.
        let cur2 = Snapshot {
            streaming: "…".into(),
            ..cur.clone()
        };
        assert_eq!(detect_trigger(Some(&cur), &cur2, None), None);
        // A REPLACED prompt (seq bumped) fires again — it may be more dangerous.
        let cur3 = Snapshot {
            permission_prompt: Some("allow shell (Shell) [y/n]".into()),
            prompt_seq: 2,
            ..cur.clone()
        };
        let msg3 = detect_trigger(Some(&cur2), &cur3, None).expect("replaced prompt re-fires");
        assert_eq!(msg3.seq, 2);
        // No prev frame at all (driver restart): a pending prompt still pushes.
        assert!(detect_trigger(None, &cur, None).is_some());
    }

    #[test]
    fn trigger_fires_on_a_question_with_its_title() {
        let prev = Snapshot {
            busy: true,
            ..snap()
        };
        let cur = Snapshot {
            busy: true,
            title: "fix the parser".into(),
            question: Some("Which approach?".into()),
            prompt_seq: 3,
            ..snap()
        };
        let msg = detect_trigger(Some(&prev), &cur, None).unwrap();
        assert_eq!(msg.kind, "question");
        assert_eq!(msg.title, "fix the parser");
        assert_eq!(msg.body, "Which approach?");
    }

    #[test]
    fn trigger_fires_done_and_failed_on_the_busy_falling_edge() {
        let busy = Snapshot {
            busy: true,
            ..snap()
        };
        let idle = Snapshot {
            busy: false,
            transcript: vec!["forge: all tests pass".into(), "  ".into()],
            ..snap()
        };
        let msg = detect_trigger(Some(&busy), &idle, None).unwrap();
        assert_eq!(msg.kind, "done");
        assert_eq!(msg.body, "forge: all tests pass", "last non-empty line");
        assert_eq!(msg.ttl, TTL_COMPLETION);
        // A latched turn error turns the same edge into "failed".
        let msg = detect_trigger(Some(&busy), &idle, Some("provider hard-fail")).unwrap();
        assert_eq!(msg.kind, "failed");
        assert_eq!(msg.body, "provider hard-fail");
        // No edge (idle → idle, busy → busy) → no push.
        assert_eq!(detect_trigger(Some(&idle), &idle, None), None);
        assert_eq!(detect_trigger(Some(&busy), &busy, None), None);
        // The prompt-resolution frame (busy stays true) is not a completion.
        let resolved = Snapshot {
            busy: true,
            prompt_seq: 1,
            ..snap()
        };
        let pending = Snapshot {
            busy: true,
            permission_prompt: Some("allow x".into()),
            prompt_seq: 1,
            ..snap()
        };
        assert_eq!(detect_trigger(Some(&pending), &resolved, None), None);
        // Archive frames are never news.
        let closed = Snapshot {
            closed: true,
            ..idle.clone()
        };
        assert_eq!(detect_trigger(Some(&busy), &closed, None), None);
    }

    #[test]
    fn push_message_payload_is_the_wire_json() {
        let msg = PushMessage {
            kind: "permission",
            session: "abc".into(),
            title: "t".into(),
            body: "allow write_file".into(),
            seq: 7,
            ttl: 300,
        };
        let v: serde_json::Value = serde_json::from_str(&msg.payload_json()).unwrap();
        assert_eq!(v["kind"], "permission");
        assert_eq!(v["session"], "abc");
        assert_eq!(v["title"], "t");
        assert_eq!(v["body"], "allow write_file");
        assert_eq!(v["seq"], 7);
    }

    #[test]
    fn bodies_are_truncated_on_char_boundaries() {
        let long: String = "é".repeat(500);
        let t = truncate_chars(&long, BODY_MAX_CHARS);
        assert!(t.chars().count() <= BODY_MAX_CHARS);
        assert!(t.ends_with('…'));
        assert_eq!(truncate_chars("short", BODY_MAX_CHARS), "short");
    }
}
