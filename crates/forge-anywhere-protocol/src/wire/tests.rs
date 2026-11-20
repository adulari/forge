use ed25519_dalek::SigningKey;

use super::*;
use crate::envelope::{Envelope, EnvelopeKind, EnvelopeMetadata, RecipientKind};
use crate::sync::{SyncOperation, SyncRecord, SyncRecordKind};

fn request(body: Vec<u8>) -> BridgeRequest {
    BridgeRequest {
        request_id: [7; 16],
        route: RouteId::Upload,
        method: "POST".into(),
        parameters: vec!["session_1".into()],
        headers: vec![("content-type".into(), "application/json".into())],
        body,
        body_blob: None,
    }
}

fn response(body: Vec<u8>) -> BridgeResponse {
    BridgeResponse {
        request_id: [9; 16],
        status: 200,
        headers: vec![("content-type".into(), "application/json".into())],
        body,
        body_blob: None,
    }
}

fn frame(bytes: Vec<u8>) -> WebSocketFrame {
    WebSocketFrame {
        stream_id: [3; 16],
        direction: FrameDirection::HostToController,
        kind: WebSocketFrameKind::Data,
        text: true,
        bytes,
        bytes_blob: None,
    }
}

fn without_wire(encoded: &[u8]) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(encoded).unwrap();
    value.as_object_mut().unwrap().remove("wire");
    serde_json::to_vec(&value).unwrap()
}

fn snapshot_like() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "protocol": 10,
        "session_id": "6f25f970-81e8-4ad9-a5a6-184cc5940037",
        "rows": (0..40).map(|i| serde_json::json!({
            "seq": i, "role": "assistant", "kind": "assistant",
            "content": format!("Row {i}: the relay carries this text inside an encrypted envelope."),
        })).collect::<Vec<_>>(),
    }))
    .unwrap()
}

#[test]
fn v1_output_is_the_legacy_encoding_plus_the_advertisement() {
    let body = snapshot_like();
    let legacy_request = serde_json::to_vec(&request(body.clone())).unwrap();
    let encoded = encode_request(&request(body.clone()), Wire::V1).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&without_wire(&encoded)).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&legacy_request).unwrap()
    );
    assert_eq!(advertised(&encoded), WIRE_V2);

    let legacy_response = serde_json::to_vec(&response(body.clone())).unwrap();
    let encoded = encode_response(&response(body.clone()), Wire::V1).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&without_wire(&encoded)).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&legacy_response).unwrap()
    );

    // Frames carry no advertisement: their stream negotiated once, at open.
    assert_eq!(
        encode_frame(&frame(body.clone()), Wire::V1).unwrap(),
        serde_json::to_vec(&frame(body)).unwrap()
    );
}

#[test]
fn both_versions_round_trip_every_bridge_type() {
    for wire in [Wire::V1, Wire::V2] {
        for body in [Vec::new(), vec![0, 255, 1, 254], snapshot_like()] {
            let decoded: BridgeRequest =
                serde_json::from_slice(&encode_request(&request(body.clone()), wire).unwrap())
                    .unwrap();
            assert_eq!(decoded, request(body.clone()), "{wire:?} request");
            let decoded: BridgeResponse =
                serde_json::from_slice(&encode_response(&response(body.clone()), wire).unwrap())
                    .unwrap();
            assert_eq!(decoded, response(body.clone()), "{wire:?} response");
            let decoded: WebSocketFrame =
                serde_json::from_slice(&encode_frame(&frame(body.clone()), wire).unwrap()).unwrap();
            assert_eq!(decoded, frame(body), "{wire:?} frame");
        }
    }
}

#[test]
fn v2_writes_base64url_strings_and_blob_references_are_unchanged() {
    let mut with_blob = response(Vec::new());
    with_blob.body_blob = Some(RelayBlobReference {
        blob_id: [2; 16],
        ciphertext_bytes: 300_000,
        ciphertext_sha256: [4; 32],
        plaintext_bytes: Some(70_000),
    });
    let v2: serde_json::Value =
        serde_json::from_slice(&encode_response(&with_blob, Wire::V2).unwrap()).unwrap();
    assert_eq!(v2["request_id"], URL_SAFE_NO_PAD.encode([9; 16]));
    assert_eq!(v2["body"], "");
    assert_eq!(v2["wire"], 2);
    assert_eq!(
        v2["body_blob"],
        serde_json::to_value(with_blob.body_blob).unwrap()
    );
}

#[test]
fn a_v1_only_reader_can_parse_everything_a_v1_sender_writes() {
    // The decoder an App Store build or an older daemon runs: plain arrays, unknown fields ignored.
    #[derive(Deserialize)]
    struct LegacyResponse {
        request_id: [u8; 16],
        #[serde(default)]
        body: Vec<u8>,
    }
    let encoded = encode_response(&response(snapshot_like()), Wire::V1).unwrap();
    let legacy: LegacyResponse = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(legacy.request_id, [9; 16]);
    assert_eq!(legacy.body, snapshot_like());

    // ...and it fails closed, never silently, on a v2 message it was never meant to receive.
    let v2 = encode_response(&response(snapshot_like()), Wire::V2).unwrap();
    assert!(serde_json::from_slice::<LegacyResponse>(&v2).is_err());
}

#[test]
fn negotiation_falls_back_to_v1_for_old_and_unknown_peers() {
    let legacy = serde_json::to_vec(&request(Vec::new())).unwrap();
    assert_eq!(advertised(&legacy), WIRE_V1);
    assert_eq!(Wire::negotiate(advertised(&legacy)), Wire::V1);
    assert_eq!(advertised(b"not json"), WIRE_V1);
    assert_eq!(advertised(br#"{"wire":0}"#), WIRE_V1);
    assert_eq!(Wire::negotiate(advertised(br#"{"wire":2}"#)), Wire::V2);
    // A future v3 peer still gets v2, the newest form this build writes.
    assert_eq!(Wire::negotiate(advertised(br#"{"wire":3}"#)), Wire::V2);
    assert_eq!(Wire::V1.max_outbound_inline(), 64 * 1024);
    assert_eq!(Wire::V2.max_outbound_inline(), 192 * 1024);
}

#[test]
fn malformed_byte_fields_fail_closed() {
    let mut value = serde_json::to_value(response(Vec::new())).unwrap();
    value["body"] = serde_json::json!("not base64 !!");
    assert!(serde_json::from_value::<BridgeResponse>(value.clone()).is_err());
    value["body"] = serde_json::json!([256]);
    assert!(serde_json::from_value::<BridgeResponse>(value.clone()).is_err());
    value["body"] = serde_json::json!("AAEC");
    value["request_id"] = serde_json::json!(URL_SAFE_NO_PAD.encode([1; 15]));
    assert!(serde_json::from_value::<BridgeResponse>(value.clone()).is_err());
    value["request_id"] = serde_json::json!(URL_SAFE_NO_PAD.encode([1; 16]));
    let decoded: BridgeResponse = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.body, vec![0, 1, 2]);
}

#[test]
fn sync_records_written_by_old_peers_and_v2_peers_both_decode() {
    let record = SyncRecord {
        stable_id: "message/1/42".into(),
        kind: SyncRecordKind::Message,
        revision: 3,
        logical_clock: 99,
        device_id: [5; 16],
        operation: SyncOperation::Upsert,
        base_hash: Some([6; 32]),
        content_hash: [8; 32],
        payload: snapshot_like(),
    };
    let v1 = serde_json::to_vec(&record).unwrap();
    assert_eq!(serde_json::from_slice::<SyncRecord>(&v1).unwrap(), record);

    let mut v2: serde_json::Value = serde_json::from_slice(&v1).unwrap();
    v2["device_id"] = serde_json::json!(URL_SAFE_NO_PAD.encode([5; 16]));
    v2["base_hash"] = serde_json::json!(URL_SAFE_NO_PAD.encode([6; 32]));
    v2["content_hash"] = serde_json::json!(URL_SAFE_NO_PAD.encode([8; 32]));
    v2["payload"] = serde_json::json!(URL_SAFE_NO_PAD.encode(snapshot_like()));
    assert_eq!(serde_json::from_value::<SyncRecord>(v2).unwrap(), record);

    // Tombstones written before `base_hash` and `payload` were always present still decode.
    let mut sparse: serde_json::Value = serde_json::from_slice(&v1).unwrap();
    let object = sparse.as_object_mut().unwrap();
    object.remove("base_hash");
    object.remove("payload");
    let decoded: SyncRecord = serde_json::from_value(sparse).unwrap();
    assert_eq!(decoded.base_hash, None);
    assert!(decoded.payload.is_empty());
}

#[test]
fn the_encryption_envelope_is_identical_across_wire_versions() {
    let signing_key = SigningKey::from_bytes(&[0x55; 32]);
    let data_key = [0x66; 32];
    let metadata = EnvelopeMetadata {
        kind: EnvelopeKind::BridgeResponse,
        flags: 0,
        account_id: [1; 16],
        sender_device_id: [2; 16],
        recipient_kind: RecipientKind::Device,
        recipient_id: [3; 16],
        key_epoch: 4,
        sequence: 5,
        created_at_ms: 1_750_000_000_000,
        nonce: [6; 24],
    };
    let body = snapshot_like();
    let mut sizes = Vec::new();
    for wire in [Wire::V1, Wire::V2] {
        let plaintext = encode_response(&response(body.clone()), wire).unwrap();
        let sealed = Envelope::seal(metadata.clone(), &plaintext, &data_key, &signing_key)
            .unwrap()
            .encode()
            .unwrap();
        // Same header layout, same AEAD, same signature: only the sealed plaintext shrinks.
        assert_eq!(&sealed[..4], b"FANY");
        assert_eq!(sealed[4], crate::envelope::VERSION);
        let opened = Envelope::decode(&sealed)
            .unwrap()
            .open(&data_key, &signing_key.verifying_key())
            .unwrap();
        let decoded: BridgeResponse = serde_json::from_slice(&opened).unwrap();
        assert_eq!(decoded.body, body);
        sizes.push(sealed.len());
    }
    assert!(
        sizes[0] as f64 / sizes[1] as f64 > 2.5,
        "v2 should be well over 2.5x smaller on JSON bodies: {sizes:?}"
    );
}
