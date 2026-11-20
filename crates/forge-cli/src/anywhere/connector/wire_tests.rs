//! Wire negotiation through the real host relay handler: a request that advertises wire v2 is
//! answered in v2, a request from an older app (no advertisement) is answered in the exact v1
//! form it has always parsed. The local daemon is never reached: a stream-open with bad
//! parameters is answered by the connector itself, which is all the encoding needs.

use std::convert::Infallible;

use super::*;

const SENDER: [u8; 16] = [0x64; 16];

fn identity() -> Identity {
    Identity {
        account_id: [0x11; 16],
        device_id: [0x22; 16],
        host_id: [0x55; 16],
        data_key: [0x44; 32],
        data_key_epochs: BTreeMap::from([(3, [0x44; 32])]),
        key_epoch: 3,
        signing_key: SigningKey::from_bytes(&[0x33; 32]),
    }
}

fn store(dir: &std::path::Path) -> StateStore {
    let store = StateStore {
        path: dir.join("anywhere/state.json"),
    };
    store
        .save(&LocalState {
            version: super::super::STATE_VERSION,
            account_id: Some(hex::encode([0x11; 16])),
            device_id: Some(hex::encode([0x22; 16])),
            signing_private_key: Some(URL_SAFE_NO_PAD.encode([0x33; 32])),
            account_data_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
            key_epoch: Some(3),
            host_id: Some(hex::encode([0x55; 16])),
            ..LocalState::default()
        })
        .unwrap();
    store
}

fn stream_open(sequence: u64, plaintext: &[u8]) -> Vec<u8> {
    Envelope::seal(
        EnvelopeMetadata {
            kind: EnvelopeKind::BridgeRequest,
            flags: 0,
            account_id: [0x11; 16],
            sender_device_id: SENDER,
            recipient_kind: RecipientKind::Host,
            recipient_id: [0x55; 16],
            key_epoch: 3,
            sequence,
            created_at_ms: now_ms(),
            nonce: rand::random(),
        },
        plaintext,
        &[0x44; 32],
        &SigningKey::from_bytes(&[0x77; 32]),
    )
    .unwrap()
    .encode()
    .unwrap()
}

/// Feed one sealed request to the host handler and return the plaintext of its answer.
async fn answer(dir: &std::path::Path, sequence: u64, plaintext: &[u8]) -> serde_json::Value {
    let store = store(dir);
    let identity = identity();
    let devices = HashMap::from([(SENDER, SigningKey::from_bytes(&[0x77; 32]).verifying_key())]);
    let http = Client::new();
    let blobs = RelayBlobClient {
        http: &http,
        service_url: "https://example.invalid",
        store: &store,
    };
    let (events, _events_rx) = mpsc::channel(1);
    let mut streams = HashMap::new();
    let mut sent: Vec<Message> = Vec::new();
    let mut sink = (&mut sent).sink_map_err(
        |never: Infallible| -> tokio_tungstenite::tungstenite::Error { match never {} },
    );
    let pending = handle_relay_envelope(
        &store,
        &identity,
        &devices,
        "http://127.0.0.1:9",
        &stream_open(sequence, plaintext),
        &mut streams,
        &events,
        &blobs,
        &mut sink,
    )
    .await
    .unwrap();
    assert!(
        pending.is_none(),
        "a stream open is answered by the connector"
    );
    let [Message::Binary(reply)] = sent.as_slice() else {
        panic!("expected exactly one binary reply, got {}", sent.len());
    };
    let opened = Envelope::decode(reply)
        .unwrap()
        .open(&[0x44; 32], &identity.signing_key.verifying_key())
        .unwrap();
    serde_json::from_slice(&opened).unwrap()
}

fn bad_stream_open() -> BridgeRequest {
    BridgeRequest {
        request_id: [0xab; 16],
        route: RouteId::WebSocket,
        method: "GET".into(),
        // A session stream needs exactly two parameters; one makes the connector refuse it.
        parameters: vec!["only-one".into()],
        headers: Vec::new(),
        body: Vec::new(),
        body_blob: None,
    }
}

#[tokio::test]
async fn an_app_that_advertises_v2_gets_a_v2_answer() {
    let dir = tempfile::tempdir().unwrap();
    let request = wire::encode_request(&bad_stream_open(), Wire::V1).unwrap();
    let reply = answer(dir.path(), 10, &request).await;
    assert_eq!(reply["status"], 400);
    assert_eq!(reply["wire"], 2);
    assert_eq!(reply["request_id"], URL_SAFE_NO_PAD.encode([0xab; 16]));
    assert!(reply["body"].is_string(), "v2 body is base64url: {reply}");
    let decoded: BridgeResponse = serde_json::from_value(reply).unwrap();
    assert!(String::from_utf8_lossy(&decoded.body).contains("parameters"));
}

#[tokio::test]
async fn an_app_store_build_that_says_nothing_gets_the_v1_form_it_parses() {
    let dir = tempfile::tempdir().unwrap();
    // Exactly what a current App Store build seals: the derive output, no `wire` field.
    let request = serde_json::to_vec(&bad_stream_open()).unwrap();
    let reply = answer(dir.path(), 11, &request).await;
    assert_eq!(reply["status"], 400);
    assert_eq!(
        reply["request_id"],
        serde_json::to_value([0xab_u8; 16]).unwrap()
    );
    assert!(
        reply["body"].is_array(),
        "v1 body is a number array: {reply}"
    );
    // The advertisement rides along; the app's JSON.parse ignores fields it does not know.
    assert_eq!(reply["wire"], 2);
}

#[tokio::test]
async fn a_v2_encoded_request_from_a_new_app_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let request = wire::encode_request(&bad_stream_open(), Wire::V2).unwrap();
    let reply = answer(dir.path(), 12, &request).await;
    assert_eq!(reply["status"], 400);
    assert_eq!(reply["request_id"], URL_SAFE_NO_PAD.encode([0xab; 16]));
}

/// The relay's own limit, from forge-anywhere-service/src/relay.rs.
const RELAY_ENVELOPE_LIMIT: usize = 300 * 1024;

#[test]
fn each_wire_send_threshold_fits_a_relay_envelope_and_the_receivers_inline_limit() {
    let overhead = HEADER_LEN + TAG_LEN + SIGNATURE_LEN;
    for wire in [Wire::V1, Wire::V2] {
        let response = BridgeResponse {
            request_id: [1; 16],
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            // Worst case for v1: every byte needs three digits and a comma.
            body: vec![0xff; wire.max_outbound_inline()],
            body_blob: None,
        };
        let sealed = wire::encode_response(&response, wire).unwrap().len() + overhead;
        assert!(
            sealed < RELAY_ENVELOPE_LIMIT,
            "{wire:?}: an inline body at the threshold seals into {sealed} bytes"
        );
        assert!(wire.max_outbound_inline() <= MAX_INLINE_BODY);
    }
    // v1 still cannot send the v2 threshold inline, which is why negotiation gates it.
    let v1_at_v2_threshold = BridgeResponse {
        request_id: [1; 16],
        status: 200,
        headers: Vec::new(),
        body: vec![0xff; Wire::V2.max_outbound_inline()],
        body_blob: None,
    };
    assert!(
        wire::encode_response(&v1_at_v2_threshold, Wire::V1)
            .unwrap()
            .len()
            > RELAY_ENVELOPE_LIMIT
    );
}
