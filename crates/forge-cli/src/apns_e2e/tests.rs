use super::*;
use crate::push::decrypt_payload;

fn device() -> (p256::SecretKey, [u8; 16], ApnsE2eKey) {
    let secret = p256::SecretKey::from_slice(&[7u8; 32]).unwrap();
    let auth = [3u8; 16];
    let public =
        p256::elliptic_curve::sec1::ToSec1Point::to_sec1_point(&secret.public_key(), false);
    let key = ApnsE2eKey {
        p256dh: b64url(public.as_bytes()),
        auth: b64url(&auth),
    };
    (secret, auth, key)
}

fn open(secret: &p256::SecretKey, auth: &[u8], blob: &str) -> serde_json::Value {
    let bytes = b64url_decode(blob).expect("enc is base64url");
    serde_json::from_slice(&decrypt_payload(secret, auth, &bytes).unwrap()).unwrap()
}

#[test]
fn a_sealed_notice_opens_with_the_device_key_and_only_with_it() {
    let (secret, auth, key) = device();
    let blob = seal(&key, "Build", "All green", "sess-1", 9, "done").unwrap();
    let opened = open(&secret, &auth, &blob);
    assert_eq!(opened["v"], 1);
    assert_eq!(opened["title"], "Build");
    assert_eq!(opened["message"], "All green");
    assert_eq!(opened["session_id"], "sess-1");
    assert_eq!(opened["seq"], 9);
    assert_eq!(opened["kind"], "done");
    assert!(
        !blob.contains("All green"),
        "ciphertext carries no plaintext"
    );

    let other = p256::SecretKey::from_slice(&[8u8; 32]).unwrap();
    let bytes = b64url_decode(&blob).unwrap();
    assert!(decrypt_payload(&other, &auth, &bytes).is_err());
}

#[test]
fn every_seal_uses_fresh_key_material() {
    let (_, _, key) = device();
    let a = seal(&key, "t", "m", "s", 1, "done").unwrap();
    let b = seal(&key, "t", "m", "s", 1, "done").unwrap();
    assert_ne!(a, b);
}

#[test]
fn an_unusable_key_yields_no_blob_so_the_generic_alert_stays() {
    let (_, _, good) = device();
    for key in [
        ApnsE2eKey {
            p256dh: "not base64 !!".into(),
            auth: good.auth.clone(),
        },
        ApnsE2eKey {
            p256dh: b64url(&[4u8; 65]),
            auth: good.auth.clone(),
        },
        ApnsE2eKey {
            p256dh: good.p256dh.clone(),
            auth: b64url(&[1u8; 8]),
        },
        ApnsE2eKey {
            p256dh: b64url(&[1u8; 10]),
            auth: good.auth.clone(),
        },
    ] {
        assert!(seal(&key, "t", "m", "s", 1, "done").is_none());
    }
}

#[test]
fn attach_without_a_key_leaves_the_payload_untouched() {
    let mut payload = serde_json::json!({"aps": {}});
    let before = payload.clone();
    attach(&mut payload, None, "t", "m", "s", 1, "done");
    assert_eq!(payload, before);
}

#[test]
fn the_worst_case_payload_stays_under_the_apns_limit() {
    let (secret, auth, key) = device();
    // 4-byte characters and JSON escapes are the largest expansion per character.
    let message: String = "\u{1F525}\"\\\n".repeat(200);
    let title: String = "\u{1F525}".repeat(300);
    let session = "s".repeat(120);
    let mut payload = serde_json::json!({
        "aps": {"alert": {"title": "Forge", "body": "Open Forge to view an update."},
                "sound": "default", "mutable-content": 1},
        "session": "forge-notification", "kind": "done", "seq": 0,
    });
    attach(
        &mut payload,
        Some(&key),
        &title,
        &message,
        &session,
        u64::MAX,
        "permission",
    );
    let blob = payload[ENC_KEY]
        .as_str()
        .expect("still sealed when oversized");
    let size = serde_json::to_vec(&payload).unwrap().len();
    assert!(
        size < 3072,
        "payload is {size} bytes, relay and APNs cap at 4096"
    );
    let opened = open(&secret, &auth, blob);
    assert_eq!(opened["seq"], u64::MAX);
    assert!(opened["message"].as_str().unwrap().len() < message.len());
    assert!(opened["message"].as_str().unwrap().starts_with('\u{1F525}'));
}

#[test]
fn an_overlong_session_id_is_dropped_not_sent() {
    let (secret, auth, key) = device();
    let blob = seal(&key, "t", "m", &"x".repeat(500), 1, "done").unwrap();
    assert_eq!(open(&secret, &auth, &blob)["session_id"], "");
}

/// Shared with `mobile/src/lib/push/e2e.test.ts`: the same fixture must open in Rust and in the
/// TypeScript mirror of the Swift extension, so the three implementations cannot drift apart.
#[test]
fn the_cross_language_vector_opens() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../protocol/fixtures/push-notice-vector.json"
    ))
    .unwrap();
    let scalar = fixture["receiver_private_hex"].as_str().unwrap();
    let secret = p256::SecretKey::from_slice(&hex_to_bytes(scalar)).unwrap();
    let auth = b64url_decode(fixture["auth"].as_str().unwrap()).unwrap();
    let opened = open(&secret, &auth, fixture["enc"].as_str().unwrap());
    assert_eq!(opened, fixture["plaintext"]);
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// Regenerates `protocol/fixtures/push-notice-vector.json`:
/// `FORGE_WRITE_PUSH_VECTOR=1 cargo test -p forge-agent regenerate_the_cross_language_vector`.
#[test]
fn regenerate_the_cross_language_vector() {
    if std::env::var_os("FORGE_WRITE_PUSH_VECTOR").is_none() {
        return;
    }
    let (_, auth, key) = device();
    let enc = seal(
        &key,
        "Build finished",
        "Tests pass \u{2705} on main",
        "sess-42",
        7,
        "done",
    )
    .unwrap();
    let fixture = serde_json::json!({
        "receiver_private_hex": "07".repeat(32),
        "p256dh": key.p256dh,
        "auth": b64url(&auth),
        "enc": enc,
        "plaintext": {"v": 1, "title": "Build finished", "message": "Tests pass \u{2705} on main",
                      "session_id": "sess-42", "seq": 7, "kind": "done"},
    });
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol/fixtures/push-notice-vector.json"
    );
    std::fs::write(path, serde_json::to_string_pretty(&fixture).unwrap() + "\n").unwrap();
}
