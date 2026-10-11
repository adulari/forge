//! End-to-end sealed notification text for iOS lock screens (ADR-0012, "End-to-end sealed alerts").
//!
//! The hosted relay and Apple must never see a notification's words, yet a generic "Open Forge to
//! view an update." on every lock screen is a poor product. The app generates a P-256 key and an
//! RFC 8291 auth secret on the phone, keeps the private half in a keychain access group its
//! Notification Service Extension can read, and registers only the public half with the daemon.
//! The daemon seals `{title, message, session_id, seq, kind}` to that key with the same RFC 8291
//! aes128gcm construction Web Push already uses ([`crate::push::encrypt_payload`]) and rides the
//! ciphertext in the APNs payload as `enc`, next to the generic alert. The extension decrypts and
//! swaps the text in; on any failure iOS keeps the generic alert, so the worst case is today's.

use forge_store::ApnsE2eKey;
use serde::Serialize;

use crate::push::{b64url, b64url_decode, encrypt_payload};

/// The APNs payload key carrying the sealed blob. The relay and the Swift extension both name it.
pub(crate) const ENC_KEY: &str = "enc";

/// Cap on the serialized plaintext. Ciphertext adds 103 bytes (RFC 8188 header, record delimiter,
/// GCM tag) and base64url expands by 4/3, so a full blob is about 2.2 KB: inside the 4 KiB APNs
/// limit that the relay also enforces on the whole request body, with the generic alert and
/// routing fields still to spare.
const MAX_PLAINTEXT_BYTES: usize = 1500;
const MAX_TITLE_CHARS: usize = 100;
const MAX_SESSION_ID_BYTES: usize = 128;

#[derive(Serialize)]
struct Sealed<'a> {
    v: u8,
    title: &'a str,
    message: &'a str,
    session_id: &'a str,
    seq: u64,
    kind: &'a str,
}

fn truncate_to_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Seal one notice to a device key. `None` when the key is unusable, so the caller falls back to
/// the generic alert rather than failing the push.
pub(crate) fn seal(
    key: &ApnsE2eKey,
    title: &str,
    message: &str,
    session_id: &str,
    seq: u64,
    kind: &str,
) -> Option<String> {
    let public = b64url_decode(&key.p256dh)?;
    let auth = b64url_decode(&key.auth)?;
    if public.len() != 65 || auth.len() != 16 {
        return None;
    }
    let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
    let session_id = if session_id.len() > MAX_SESSION_ID_BYTES {
        ""
    } else {
        session_id
    };
    let render = |message: &str| {
        serde_json::to_vec(&Sealed {
            v: 1,
            title: &title,
            message,
            session_id,
            seq,
            kind,
        })
        .ok()
    };
    let mut message = message;
    let mut plaintext = render(message)?;
    while plaintext.len() > MAX_PLAINTEXT_BYTES && !message.is_empty() {
        let over = plaintext.len() - MAX_PLAINTEXT_BYTES;
        // Escapes can make the JSON longer than the raw text, so shrink in steps and re-measure.
        message = truncate_to_bytes(message, message.len().saturating_sub(over.max(1)));
        plaintext = render(message)?;
    }
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return None;
    }
    encrypt_payload(&public, &auth, &plaintext)
        .ok()
        .map(|sealed| b64url(&sealed))
}

/// Add the sealed blob to an alert payload. A no-op for a device without a usable key.
pub(crate) fn attach(
    payload: &mut serde_json::Value,
    key: Option<&ApnsE2eKey>,
    title: &str,
    message: &str,
    session_id: &str,
    seq: u64,
    kind: &str,
) {
    let Some(key) = key else { return };
    if let Some(sealed) = seal(key, title, message, session_id, seq, kind) {
        payload[ENC_KEY] = serde_json::Value::String(sealed);
    }
}

#[cfg(test)]
mod tests;
