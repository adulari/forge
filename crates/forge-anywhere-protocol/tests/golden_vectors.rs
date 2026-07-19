use ed25519_dalek::{SigningKey, VerifyingKey};
use forge_anywhere_protocol::{Envelope, EnvelopeKind, EnvelopeMetadata, RecipientKind};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    signing_private_key_hex: String,
    signing_public_key_hex: String,
    encryption_key_hex: String,
    kind: u8,
    flags: u16,
    account_id_hex: String,
    sender_device_id_hex: String,
    recipient_kind: u8,
    recipient_id_hex: String,
    key_epoch: u32,
    sequence: u64,
    created_at_ms: u64,
    nonce_hex: String,
    plaintext_hex: String,
    envelope_hex: String,
}

fn bytes<const N: usize>(value: &str) -> [u8; N] {
    hex::decode(value)
        .expect("fixture hex")
        .try_into()
        .unwrap_or_else(|_| panic!("fixture field must be {N} bytes"))
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol/fixtures/anywhere-v1/envelope-bridge-request.json"
    )))
    .expect("valid fixture JSON")
}

#[test]
fn rust_matches_normative_bridge_request_vector() {
    let fixture = fixture();
    let signing_key = SigningKey::from_bytes(&bytes(&fixture.signing_private_key_hex));
    let verifying_key = VerifyingKey::from_bytes(&bytes(&fixture.signing_public_key_hex))
        .expect("valid fixture public key");
    assert_eq!(signing_key.verifying_key(), verifying_key);

    let metadata = EnvelopeMetadata {
        kind: EnvelopeKind::try_from(fixture.kind).expect("known fixture kind"),
        flags: fixture.flags,
        account_id: bytes(&fixture.account_id_hex),
        sender_device_id: bytes(&fixture.sender_device_id_hex),
        recipient_kind: RecipientKind::try_from(fixture.recipient_kind)
            .expect("known fixture recipient kind"),
        recipient_id: bytes(&fixture.recipient_id_hex),
        key_epoch: fixture.key_epoch,
        sequence: fixture.sequence,
        created_at_ms: fixture.created_at_ms,
        nonce: bytes(&fixture.nonce_hex),
    };
    let plaintext = hex::decode(&fixture.plaintext_hex).expect("fixture plaintext hex");
    let encryption_key = bytes(&fixture.encryption_key_hex);
    let envelope =
        Envelope::seal(metadata, &plaintext, &encryption_key, &signing_key).expect("seal fixture");
    assert_eq!(
        hex::encode(envelope.encode().expect("encode fixture")),
        fixture.envelope_hex
    );

    let decoded =
        Envelope::decode(&hex::decode(&fixture.envelope_hex).expect("fixture envelope hex"))
            .expect("decode fixture");
    assert_eq!(decoded.open(&encryption_key, &verifying_key), Ok(plaintext));
}

#[test]
fn fixture_rejects_non_canonical_and_tampered_bytes() {
    let fixture = fixture();
    let verifying_key = VerifyingKey::from_bytes(&bytes(&fixture.signing_public_key_hex))
        .expect("valid fixture public key");
    let encryption_key = bytes(&fixture.encryption_key_hex);
    let original = hex::decode(&fixture.envelope_hex).expect("fixture envelope hex");

    let mut trailing = original.clone();
    trailing.push(0);
    assert!(Envelope::decode(&trailing).is_err());

    for offset in [6, 77, 105, original.len() - 1] {
        let mut tampered = original.clone();
        tampered[offset] ^= 1;
        if let Ok(envelope) = Envelope::decode(&tampered) {
            assert!(envelope.open(&encryption_key, &verifying_key).is_err());
        }
    }
}
