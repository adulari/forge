use ed25519_dalek::SigningKey;
use forge_anywhere_protocol::{Envelope, EnvelopeKind, EnvelopeMetadata, RecipientKind};

fn main() {
    let signing_key = SigningKey::from_bytes(&[0x55; 32]);
    let encryption_key = [0x66; 32];
    let metadata = EnvelopeMetadata {
        kind: EnvelopeKind::BridgeRequest,
        flags: 0x0102,
        account_id: [0x11; 16],
        sender_device_id: [0x22; 16],
        recipient_kind: RecipientKind::Host,
        recipient_id: [0x33; 16],
        key_epoch: 7,
        sequence: 42,
        created_at_ms: 1_750_000_000_123,
        nonce: [0x44; 24],
    };
    let plaintext = b"typed bridge payload";
    let envelope = Envelope::seal(metadata, plaintext, &encryption_key, &signing_key)
        .expect("fixture inputs are valid");
    println!(
        "signing_public_key_hex={}",
        hex::encode(signing_key.verifying_key().to_bytes())
    );
    println!("plaintext_hex={}", hex::encode(plaintext));
    println!(
        "envelope_hex={}",
        hex::encode(envelope.encode().expect("encode fixture"))
    );
}
