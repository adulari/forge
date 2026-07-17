//! Managed encrypted relay connector for `forge serve`.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use forge_anywhere_protocol::bridge::{
    BridgeRequest, BridgeResponse, FrameDirection, RelayBlobReference, RouteId, WebSocketFrame,
    WebSocketFrameKind,
};
use forge_anywhere_protocol::{Envelope, EnvelopeKind, EnvelopeMetadata, RecipientKind};
use futures::{SinkExt, StreamExt};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, Method, RequestBuilder, Response, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::{
    client, decode_base64_array, decode_hex_array, decode_response, ensure_access_token,
    idempotency_key, now_ms, refresh_account_epoch, send_json, DeviceList, LocalState, StateStore,
};

const MAX_INLINE_BODY: usize = 256 * 1024;
const MAX_BLOB_BYTES: u64 = 32 * 1024 * 1024;
const MAX_QUERY_LEN: usize = 4096;
const RECONNECT_DELAY: Duration = Duration::from_secs(10);

#[derive(Serialize)]
struct RelayTicketRequest<'a> {
    host_id: &'a str,
}

#[derive(Deserialize)]
struct RelayTicketResponse {
    ticket: String,
}

#[derive(Serialize)]
struct ReserveBlobRequest {
    recipient_kind: &'static str,
    recipient_id: String,
    ciphertext_bytes: u64,
    ciphertext_sha256: String,
}

#[derive(Deserialize)]
struct ReserveBlobResponse {
    blob_id: String,
    upload_url: Option<String>,
    #[serde(default)]
    required_headers: BTreeMap<String, String>,
    #[serde(default)]
    already_complete: bool,
}

#[derive(Deserialize)]
struct ClaimBlobResponse {
    download_url: String,
    ciphertext_bytes: u64,
    ciphertext_sha256: String,
    #[serde(default)]
    required_headers: BTreeMap<String, String>,
}

struct RelayBlobClient<'a> {
    http: &'a Client,
    service_url: &'a str,
    access_token: &'a str,
}

struct VerifiedBlob {
    blob_id: [u8; 16],
    plaintext: Vec<u8>,
    sequence: u64,
}

struct Identity {
    account_id: [u8; 16],
    device_id: [u8; 16],
    host_id: [u8; 16],
    data_key: [u8; 32],
    key_epoch: u32,
    signing_key: SigningKey,
}

struct StreamHandle {
    owner_device_id: [u8; 16],
    commands: mpsc::Sender<LocalSocketCommand>,
}

enum LocalSocketCommand {
    Data(Vec<u8>),
    Close,
}

enum LocalSocketEvent {
    Data {
        stream_id: [u8; 16],
        owner_device_id: [u8; 16],
        bytes: Vec<u8>,
    },
    Closed {
        stream_id: [u8; 16],
        owner_device_id: [u8; 16],
    },
}

pub(super) fn spawn(local_base_url: String) -> tokio::task::JoinHandle<()> {
    let _sync_worker = super::sync::spawn();
    tokio::spawn(async move {
        let mut last_error = String::new();
        loop {
            match connect_once(&local_base_url).await {
                Ok(()) => last_error.clear(),
                Err(error) => {
                    let message = format!("{error:#}");
                    if message != last_error {
                        eprintln!(
                            "⚠ Forge Anywhere connector offline — local/direct Forge is unaffected: {message}"
                        );
                        last_error = message;
                    }
                }
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    })
}

async fn connect_once(local_base_url: &str) -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let host_id = state
        .host_id
        .clone()
        .context("host is not registered; run `forge anywhere enable`")?;
    let access_token = ensure_access_token(&store, &mut state).await?;
    let config = forge_config::load()?;
    let service_url = config.anywhere.service_url().to_owned();
    refresh_account_epoch(&store, &mut state, &service_url, &access_token).await?;
    let identity = identity_from_state(&state)?;
    let http = client()?;
    let devices = active_device_keys(&http, &service_url, &access_token).await?;
    let ticket: RelayTicketResponse = send_json(
        http.post(format!("{service_url}/v1/relay/tickets"))
            .bearer_auth(&access_token)
            .json(&RelayTicketRequest { host_id: &host_id }),
    )
    .await
    .context("request one-time relay ticket")?;

    let relay_url = relay_url(&service_url, &ticket.ticket)?;
    let (relay, _) = tokio_tungstenite::connect_async(relay_url.as_str())
        .await
        .context("connect encrypted Anywhere relay")?;
    println!("⚒ Forge Anywhere connector online");
    let (mut relay_write, mut relay_read) = relay.split();
    let (local_events_tx, mut local_events_rx) = mpsc::channel(128);
    let mut streams = HashMap::<[u8; 16], StreamHandle>::new();
    let blobs = RelayBlobClient {
        http: &http,
        service_url: &service_url,
        access_token: &access_token,
    };

    loop {
        tokio::select! {
            relay_message = relay_read.next() => {
                let Some(relay_message) = relay_message else {
                    bail!("relay closed the connection");
                };
                match relay_message.context("read relay frame")? {
                    Message::Binary(bytes) => {
                        handle_relay_envelope(
                            &store,
                            &identity,
                            &devices,
                            local_base_url,
                            bytes.as_ref(),
                            &mut streams,
                            &local_events_tx,
                            &blobs,
                            &mut relay_write,
                        ).await?;
                    }
                    Message::Ping(bytes) => relay_write.send(Message::Pong(bytes)).await?,
                    Message::Pong(_) => {}
                    Message::Close(_) => bail!("relay closed the connection"),
                    Message::Text(_) | Message::Frame(_) => bail!("relay sent a non-binary data frame"),
                }
            }
            event = local_events_rx.recv() => {
                let Some(event) = event else {
                    bail!("local WebSocket event channel closed");
                };
                let (stream_id, owner_device_id, kind, bytes) = match event {
                    LocalSocketEvent::Data { stream_id, owner_device_id, bytes } => {
                        (stream_id, owner_device_id, WebSocketFrameKind::Data, bytes)
                    }
                    LocalSocketEvent::Closed { stream_id, owner_device_id } => {
                        streams.remove(&stream_id);
                        (stream_id, owner_device_id, WebSocketFrameKind::Close, Vec::new())
                    }
                };
                let mut frame = WebSocketFrame {
                    stream_id,
                    direction: FrameDirection::HostToController,
                    kind,
                    bytes,
                    bytes_blob: None,
                };
                if frame.bytes.len() > MAX_INLINE_BODY {
                    let reference = upload_blob(
                        &store,
                        &blobs,
                        owner_device_id,
                        &frame.bytes,
                    ).await.context("externalize local WebSocket frame")?;
                    frame.bytes.clear();
                    frame.bytes_blob = Some(reference);
                }
                let plaintext = serde_json::to_vec(&frame).context("encode local WebSocket frame")?;
                let encoded = seal_outbound(
                    &store,
                    EnvelopeKind::WebSocketFrame,
                    owner_device_id,
                    &plaintext,
                )?;
                relay_write.send(Message::Binary(encoded.into())).await?;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_relay_envelope<S>(
    store: &StateStore,
    identity: &Identity,
    devices: &HashMap<[u8; 16], VerifyingKey>,
    local_base_url: &str,
    encoded: &[u8],
    streams: &mut HashMap<[u8; 16], StreamHandle>,
    local_events: &mpsc::Sender<LocalSocketEvent>,
    blobs: &RelayBlobClient<'_>,
    relay_write: &mut S,
) -> Result<()>
where
    S: futures::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let envelope = Envelope::decode(encoded).context("decode relay envelope")?;
    validate_inbound_metadata(&envelope, identity)?;
    let sender_device_id = envelope.metadata.sender_device_id;
    let verifying_key = devices
        .get(&sender_device_id)
        .context("relay envelope came from an unknown or revoked device")?;
    let plaintext = envelope
        .open(&identity.data_key, verifying_key)
        .context("authenticate and decrypt relay envelope")?;

    match envelope.metadata.kind {
        EnvelopeKind::BridgeRequest => {
            let mut request: BridgeRequest =
                serde_json::from_slice(&plaintext).context("decode typed bridge request")?;
            let verified_blob = resolve_inbound_blob(
                blobs,
                identity,
                verifying_key,
                sender_device_id,
                request.body_blob,
                &request.body,
                "bridge request body",
            )
            .await?;
            accept_inbound_envelopes(
                store,
                sender_device_id,
                envelope.metadata.key_epoch,
                verified_blob.as_ref().map(|blob| blob.sequence),
                envelope.metadata.sequence,
            )?;
            let consumed_blob_id = verified_blob.as_ref().map(|blob| blob.blob_id);
            if let Some(blob) = verified_blob {
                request.body = blob.plaintext;
                request.body_blob = None;
            }

            let mut response = if request.route == RouteId::WebSocket {
                open_stream(
                    local_base_url,
                    &request,
                    sender_device_id,
                    streams,
                    local_events,
                )
                .await
            } else {
                dispatch_http(local_base_url, &request).await
            };
            if let Some(blob_id) = consumed_blob_id {
                consume_blob_after_delivery(blobs, blob_id).await;
            }
            if response.body.len() > MAX_INLINE_BODY {
                match upload_blob(store, blobs, sender_device_id, &response.body).await {
                    Ok(reference) => {
                        response.body.clear();
                        response.body_blob = Some(reference);
                    }
                    Err(_) => {
                        response = bridge_error(
                            request.request_id,
                            502,
                            "local response could not be transferred as a temporary blob",
                        );
                    }
                }
            }
            let plaintext = serde_json::to_vec(&response).context("encode bridge response")?;
            let encoded = seal_outbound(
                store,
                EnvelopeKind::BridgeResponse,
                sender_device_id,
                &plaintext,
            )?;
            relay_write.send(Message::Binary(encoded.into())).await?;
        }
        EnvelopeKind::WebSocketFrame => {
            let mut frame: WebSocketFrame =
                serde_json::from_slice(&plaintext).context("decode typed WebSocket frame")?;
            if frame.direction != FrameDirection::ControllerToHost {
                bail!("controller sent a WebSocket frame with the wrong direction");
            }
            if frame.kind == WebSocketFrameKind::Close && frame.bytes_blob.is_some() {
                bail!("WebSocket close frame contains a blob reference");
            }
            let verified_blob = resolve_inbound_blob(
                blobs,
                identity,
                verifying_key,
                sender_device_id,
                frame.bytes_blob,
                &frame.bytes,
                "WebSocket frame bytes",
            )
            .await?;
            accept_inbound_envelopes(
                store,
                sender_device_id,
                envelope.metadata.key_epoch,
                verified_blob.as_ref().map(|blob| blob.sequence),
                envelope.metadata.sequence,
            )?;
            let consumed_blob_id = verified_blob.as_ref().map(|blob| blob.blob_id);
            if let Some(blob) = verified_blob {
                frame.bytes = blob.plaintext;
                frame.bytes_blob = None;
            }
            let handle = streams
                .get(&frame.stream_id)
                .context("WebSocket frame refers to an unopened stream")?;
            if handle.owner_device_id != sender_device_id {
                bail!("WebSocket stream belongs to another controller device");
            }
            match frame.kind {
                WebSocketFrameKind::Data => handle
                    .commands
                    .send(LocalSocketCommand::Data(frame.bytes))
                    .await
                    .context("forward controller frame to local daemon")?,
                WebSocketFrameKind::Close => {
                    if let Some(handle) = streams.remove(&frame.stream_id) {
                        let _ = handle.commands.send(LocalSocketCommand::Close).await;
                    }
                }
            }
            if let Some(blob_id) = consumed_blob_id {
                consume_blob_after_delivery(blobs, blob_id).await;
            }
        }
        _ => bail!("relay envelope kind is not valid on a host connection"),
    }
    Ok(())
}

fn validate_inbound_metadata(envelope: &Envelope, identity: &Identity) -> Result<()> {
    let metadata = &envelope.metadata;
    if metadata.account_id != identity.account_id {
        bail!("relay envelope account does not match this host");
    }
    if metadata.recipient_kind != RecipientKind::Host || metadata.recipient_id != identity.host_id {
        bail!("relay envelope is not addressed to this host");
    }
    if metadata.key_epoch != identity.key_epoch {
        bail!("relay envelope uses an unavailable account key epoch");
    }
    Ok(())
}

async fn resolve_inbound_blob(
    blobs: &RelayBlobClient<'_>,
    identity: &Identity,
    verifying_key: &VerifyingKey,
    sender_device_id: [u8; 16],
    reference: Option<RelayBlobReference>,
    inline: &[u8],
    field: &str,
) -> Result<Option<VerifiedBlob>> {
    let Some(reference) = reference else {
        if inline.len() > MAX_INLINE_BODY {
            bail!("inline {field} exceeds 256 KiB");
        }
        return Ok(None);
    };
    if !inline.is_empty() {
        bail!("{field} contains both inline bytes and a blob reference");
    }
    if reference.ciphertext_bytes > MAX_BLOB_BYTES {
        bail!("encrypted relay blob exceeds 32 MiB");
    }

    let claim: ClaimBlobResponse = send_json(
        blobs
            .http
            .get(format!(
                "{}/v1/relay/blobs/{}",
                blobs.service_url,
                hex::encode(reference.blob_id)
            ))
            .bearer_auth(blobs.access_token),
    )
    .await
    .context("claim encrypted relay blob")?;
    let claimed_hash =
        decode_base64_array::<32>(&claim.ciphertext_sha256, "relay blob ciphertext hash")?;
    if claim.ciphertext_bytes != reference.ciphertext_bytes
        || claimed_hash != reference.ciphertext_sha256
    {
        bail!("claimed relay blob metadata does not match its typed reference");
    }
    let bytes = download_blob(
        blobs.http,
        &claim.download_url,
        &claim.required_headers,
        claim.ciphertext_bytes,
    )
    .await?;
    verify_blob_object(identity, verifying_key, sender_device_id, reference, &bytes).map(Some)
}

async fn download_blob(
    http: &Client,
    download_url: &str,
    required_headers: &BTreeMap<String, String>,
    expected_bytes: u64,
) -> Result<Vec<u8>> {
    if expected_bytes > MAX_BLOB_BYTES {
        bail!("encrypted relay blob exceeds 32 MiB");
    }
    let mut download = http.get(download_url);
    for (name, value) in required_headers {
        download = download.header(
            HeaderName::from_bytes(name.as_bytes()).context("invalid blob download header name")?,
            HeaderValue::from_str(value).context("invalid blob download header value")?,
        );
    }
    let response = download
        .send()
        .await
        .context("download encrypted relay blob")?
        .error_for_status()
        .context("object storage rejected encrypted relay blob download")?;
    let capacity =
        usize::try_from(expected_bytes).context("relay blob length does not fit usize")?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read encrypted relay blob")?;
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .context("encrypted relay blob length overflow")?;
        if next_len > capacity || next_len as u64 > MAX_BLOB_BYTES {
            bail!("encrypted relay blob download exceeds its declared length");
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != capacity {
        bail!("encrypted relay blob download length does not match its reference");
    }
    Ok(bytes)
}

fn verify_blob_object(
    identity: &Identity,
    verifying_key: &VerifyingKey,
    sender_device_id: [u8; 16],
    reference: RelayBlobReference,
    bytes: &[u8],
) -> Result<VerifiedBlob> {
    if bytes.len() as u64 != reference.ciphertext_bytes {
        bail!("encrypted relay blob length does not match its reference");
    }
    let actual_hash: [u8; 32] = Sha256::digest(bytes).into();
    if actual_hash != reference.ciphertext_sha256 {
        bail!("encrypted relay blob hash does not match its reference");
    }
    let envelope = Envelope::decode(bytes).context("decode encrypted relay blob envelope")?;
    let metadata = &envelope.metadata;
    if metadata.kind != EnvelopeKind::Blob
        || metadata.account_id != identity.account_id
        || metadata.sender_device_id != sender_device_id
        || metadata.recipient_kind != RecipientKind::Host
        || metadata.recipient_id != identity.host_id
        || metadata.key_epoch != identity.key_epoch
    {
        bail!("encrypted relay blob routing metadata does not match its reference");
    }
    let sequence = metadata.sequence;
    let plaintext = envelope
        .open(&identity.data_key, verifying_key)
        .context("authenticate and decrypt relay blob")?;
    Ok(VerifiedBlob {
        blob_id: reference.blob_id,
        plaintext,
        sequence,
    })
}

fn accept_inbound_envelopes(
    store: &StateStore,
    sender_device_id: [u8; 16],
    key_epoch: u32,
    blob_sequence: Option<u64>,
    control_sequence: u64,
) -> Result<()> {
    let namespace = format!("{}:{key_epoch}", hex::encode(sender_device_id));
    store.update(|state| {
        let first_sequence = blob_sequence.unwrap_or(control_sequence);
        if blob_sequence.is_some_and(|sequence| sequence >= control_sequence) {
            bail!("relay blob sequence must precede its referencing envelope");
        }
        if state
            .accepted_sequences
            .get(&namespace)
            .is_some_and(|last| first_sequence <= *last)
        {
            bail!("replayed or out-of-order relay sequence");
        }
        state.accepted_sequences.insert(namespace, control_sequence);
        Ok(())
    })?;
    Ok(())
}

async fn upload_blob(
    store: &StateStore,
    blobs: &RelayBlobClient<'_>,
    recipient_device_id: [u8; 16],
    plaintext: &[u8],
) -> Result<RelayBlobReference> {
    let encoded = seal_for_recipient(
        store,
        EnvelopeKind::Blob,
        RecipientKind::Device,
        recipient_device_id,
        plaintext,
    )?;
    let ciphertext_bytes = encoded.len() as u64;
    if ciphertext_bytes > MAX_BLOB_BYTES {
        bail!("encrypted relay blob exceeds 32 MiB");
    }
    let ciphertext_sha256: [u8; 32] = Sha256::digest(&encoded).into();
    let reserve_key = idempotency_key();
    let reserve_request = || {
        blobs
            .http
            .post(format!("{}/v1/relay/blobs", blobs.service_url))
            .bearer_auth(blobs.access_token)
            .header("Idempotency-Key", &reserve_key)
            .json(&ReserveBlobRequest {
                recipient_kind: "device",
                recipient_id: hex::encode(recipient_device_id),
                ciphertext_bytes,
                ciphertext_sha256: URL_SAFE_NO_PAD.encode(ciphertext_sha256),
            })
    };
    let reservation: ReserveBlobResponse = decode_response(
        send_transport_retry_once(reserve_request, "reserve encrypted relay blob").await?,
    )
    .await
    .context("reserve encrypted relay blob")?;
    let blob_id = decode_hex_array::<16>(&reservation.blob_id, "relay blob id")?;
    if reservation.already_complete {
        return Ok(RelayBlobReference {
            blob_id,
            ciphertext_bytes,
            ciphertext_sha256,
        });
    }
    let upload_url = reservation
        .upload_url
        .as_deref()
        .context("relay blob reservation omitted its upload URL")?;
    let mut upload = blobs.http.put(upload_url).body(encoded);
    for (name, value) in &reservation.required_headers {
        upload = upload.header(
            HeaderName::from_bytes(name.as_bytes()).context("invalid blob upload header name")?,
            HeaderValue::from_str(value).context("invalid blob upload header value")?,
        );
    }
    upload
        .send()
        .await
        .context("upload encrypted relay blob")?
        .error_for_status()
        .context("object storage rejected encrypted relay blob upload")?;
    let completion_key = idempotency_key();
    let complete_request = || {
        blobs
            .http
            .post(format!(
                "{}/v1/relay/blobs/{}/complete",
                blobs.service_url,
                hex::encode(blob_id)
            ))
            .bearer_auth(blobs.access_token)
            .header("Idempotency-Key", &completion_key)
    };
    send_transport_retry_once(complete_request, "complete encrypted relay blob")
        .await?
        .error_for_status()
        .context("service rejected encrypted relay blob completion")?;
    Ok(RelayBlobReference {
        blob_id,
        ciphertext_bytes,
        ciphertext_sha256,
    })
}

async fn consume_blob(blobs: &RelayBlobClient<'_>, blob_id: [u8; 16]) -> Result<()> {
    blobs
        .http
        .delete(format!(
            "{}/v1/relay/blobs/{}",
            blobs.service_url,
            hex::encode(blob_id)
        ))
        .bearer_auth(blobs.access_token)
        .header("Idempotency-Key", idempotency_key())
        .send()
        .await
        .context("consume encrypted relay blob")?
        .error_for_status()
        .context("service rejected encrypted relay blob consumption")?;
    Ok(())
}

async fn send_transport_retry_once(
    mut request: impl FnMut() -> RequestBuilder,
    operation: &'static str,
) -> Result<Response> {
    match request().send().await {
        Ok(response) if !response.status().is_server_error() => Ok(response),
        Ok(_) | Err(_) => request().send().await.with_context(|| operation),
    }
}

async fn consume_blob_after_delivery(blobs: &RelayBlobClient<'_>, blob_id: [u8; 16]) {
    if consume_blob(blobs, blob_id).await.is_err() {
        eprintln!(
            "⚠ Forge Anywhere delivered a verified relay blob but could not remove its temporary ciphertext; it will expire automatically"
        );
    }
}

fn seal_outbound(
    store: &StateStore,
    kind: EnvelopeKind,
    recipient_device_id: [u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    seal_for_recipient(
        store,
        kind,
        RecipientKind::Device,
        recipient_device_id,
        plaintext,
    )
}

fn seal_for_recipient(
    store: &StateStore,
    kind: EnvelopeKind,
    recipient_kind: RecipientKind,
    recipient_id: [u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let (state, sequence) = store.reserve_sequences(1)?;
    let identity = identity_from_state(&state)?;
    let envelope = Envelope::seal(
        EnvelopeMetadata {
            kind,
            flags: 0,
            account_id: identity.account_id,
            sender_device_id: identity.device_id,
            recipient_kind,
            recipient_id,
            key_epoch: identity.key_epoch,
            sequence,
            created_at_ms: now_ms(),
            nonce: rand::random::<[u8; 24]>(),
        },
        plaintext,
        &identity.data_key,
        &identity.signing_key,
    )?;
    Ok(envelope.encode()?)
}

async fn active_device_keys(
    client: &Client,
    service_url: &str,
    access_token: &str,
) -> Result<HashMap<[u8; 16], VerifyingKey>> {
    let list: DeviceList = send_json(
        client
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(access_token),
    )
    .await
    .context("load active device signing keys")?;
    let mut keys = HashMap::new();
    for device in list.devices {
        let Some(encoded_key) = device.signing_public_key.as_deref() else {
            continue;
        };
        let id = decode_hex_array::<16>(&device.id, "device id")?;
        let key = VerifyingKey::from_bytes(&decode_base64_array(
            encoded_key,
            "device signing public key",
        )?)?;
        keys.insert(id, key);
    }
    if keys.is_empty() {
        bail!("service returned no active device signing keys");
    }
    Ok(keys)
}

fn identity_from_state(state: &LocalState) -> Result<Identity> {
    Ok(Identity {
        account_id: decode_hex_array(
            state
                .account_id
                .as_deref()
                .context("Anywhere account id is missing")?,
            "account id",
        )?,
        device_id: decode_hex_array(
            state
                .device_id
                .as_deref()
                .context("Anywhere device id is missing")?,
            "device id",
        )?,
        host_id: decode_hex_array(
            state
                .host_id
                .as_deref()
                .context("Anywhere host id is missing")?,
            "host id",
        )?,
        data_key: decode_base64_array(
            state
                .account_data_key
                .as_deref()
                .context("Anywhere Account Data Key is missing")?,
            "Account Data Key",
        )?,
        key_epoch: state.key_epoch.context("Anywhere key epoch is missing")?,
        signing_key: SigningKey::from_bytes(&decode_base64_array(
            state
                .signing_private_key
                .as_deref()
                .context("Anywhere signing key is missing")?,
            "device signing private key",
        )?),
    })
}

fn relay_url(service_url: &str, ticket: &str) -> Result<Url> {
    let mut url = Url::parse(service_url).context("parse Anywhere service URL")?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => bail!("Anywhere service URL must use http or https"),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("set Anywhere relay URL scheme"))?;
    url.set_path("/v1/relay");
    url.set_query(None);
    url.query_pairs_mut().append_pair("ticket", ticket);
    Ok(url)
}

async fn dispatch_http(local_base_url: &str, request: &BridgeRequest) -> BridgeResponse {
    match dispatch_http_inner(local_base_url, request).await {
        Ok(response) => response,
        Err(error) => bridge_error(request.request_id, 400, &format!("{error:#}")),
    }
}

async fn dispatch_http_inner(
    local_base_url: &str,
    request: &BridgeRequest,
) -> Result<BridgeResponse> {
    if request.route == RouteId::Health {
        return Ok(BridgeResponse {
            request_id: request.request_id,
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
            body_blob: None,
        });
    }
    let target = route_target(request)?;
    let supplied_method =
        Method::from_bytes(request.method.as_bytes()).context("invalid method")?;
    if supplied_method != target.method {
        bail!("bridge method does not match its allowlisted route");
    }
    let mut url = Url::parse(local_base_url).context("parse local daemon URL")?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}{}", target.path));
    url.set_query(target.query.as_deref());

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("build local bridge client")?;
    let mut outgoing = client
        .request(target.method, url)
        .body(request.body.clone());
    for (name, value) in &request.headers {
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "accept" | "content-type"
        ) {
            outgoing = outgoing.header(name, value);
        }
    }
    let response = outgoing.send().await.context("call local Forge daemon")?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "content-type" | "content-disposition" | "etag" | "cache-control"
            )
        })
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.to_string(), value.to_owned()))
        })
        .collect();
    let body = response
        .bytes()
        .await
        .context("read local Forge daemon response")?;
    Ok(BridgeResponse {
        request_id: request.request_id,
        status,
        headers,
        body: body.to_vec(),
        body_blob: None,
    })
}

struct RouteTarget {
    method: Method,
    path: String,
    query: Option<String>,
}

fn route_target(request: &BridgeRequest) -> Result<RouteTarget> {
    let exact = |method, path: &str| -> Result<RouteTarget> {
        Ok(RouteTarget {
            method,
            path: path.to_owned(),
            query: query_parameter(&request.parameters, 0)?,
        })
    };
    match request.route {
        RouteId::ListSessions => exact(Method::GET, "/api/sessions"),
        RouteId::CreateSession => exact(Method::POST, "/api/sessions"),
        RouteId::SessionHistory => exact(Method::GET, "/api/history"),
        RouteId::PastSessions => exact(Method::GET, "/api/sessions/past"),
        RouteId::SessionTree => exact(Method::GET, "/api/sessions/tree"),
        RouteId::ListProjects => exact(Method::GET, "/api/projects"),
        RouteId::BrowseProjects => exact(Method::GET, "/api/projects/browse"),
        RouteId::Upload => exact(Method::POST, "/api/upload"),
        RouteId::VoiceTranscribe => exact(Method::POST, "/api/voice/transcribe"),
        RouteId::ListSkills => exact(Method::GET, "/api/skills"),
        RouteId::ListModels => exact(Method::GET, "/api/models"),
        RouteId::ReadConfig => exact(Method::GET, "/api/config"),
        RouteId::UpdateConfig => exact(Method::PUT, "/api/config"),
        RouteId::ListHooks => exact(Method::GET, "/api/hooks"),
        RouteId::ListPlans => exact(Method::GET, "/api/plans"),
        RouteId::ReadMcp => exact(Method::GET, "/api/mcp"),
        RouteId::UpdateMcp => exact(Method::POST, "/api/mcp"),
        RouteId::Usage => exact(Method::GET, "/api/usage"),
        RouteId::Answer => exact(Method::POST, "/api/answer"),
        RouteId::PushKey => exact(Method::GET, "/api/push/key"),
        RouteId::PushSubscribe => exact(Method::POST, "/api/push/subscribe"),
        RouteId::PushUnsubscribe => exact(Method::POST, "/api/push/unsubscribe"),
        RouteId::ArchiveSession
        | RouteId::ForkSession
        | RouteId::MergeSession
        | RouteId::DiscardSession => {
            if request.parameters.is_empty() || request.parameters.len() > 2 {
                bail!("session route requires one path parameter and an optional query");
            }
            let id = safe_path_segment(&request.parameters[0])?;
            let operation = match request.route {
                RouteId::ArchiveSession => "archive",
                RouteId::ForkSession => "fork",
                RouteId::MergeSession => "merge",
                RouteId::DiscardSession => "discard",
                _ => unreachable!(),
            };
            Ok(RouteTarget {
                method: Method::POST,
                path: format!("/api/sessions/{id}/{operation}"),
                query: query_parameter(&request.parameters, 1)?,
            })
        }
        RouteId::Health | RouteId::SessionSnapshot | RouteId::SessionInput | RouteId::WebSocket => {
            bail!("route is not an HTTP bridge route")
        }
    }
}

fn query_parameter(parameters: &[String], index: usize) -> Result<Option<String>> {
    if parameters.len() > index + 1 {
        bail!("bridge route contains unexpected parameters");
    }
    let Some(query) = parameters.get(index) else {
        return Ok(None);
    };
    if query.is_empty() {
        return Ok(None);
    }
    if query.len() > MAX_QUERY_LEN || !query.starts_with('?') || query.contains('#') {
        bail!("invalid bridge query parameter");
    }
    Ok(Some(query.trim_start_matches('?').to_owned()))
}

fn safe_path_segment(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("invalid session id path parameter");
    }
    Ok(value)
}

async fn open_stream(
    local_base_url: &str,
    request: &BridgeRequest,
    owner_device_id: [u8; 16],
    streams: &mut HashMap<[u8; 16], StreamHandle>,
    local_events: &mpsc::Sender<LocalSocketEvent>,
) -> BridgeResponse {
    match open_stream_inner(
        local_base_url,
        request,
        owner_device_id,
        streams,
        local_events,
    )
    .await
    {
        Ok(()) => BridgeResponse {
            request_id: request.request_id,
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            body_blob: None,
        },
        Err(error) => bridge_error(request.request_id, 400, &format!("{error:#}")),
    }
}

async fn open_stream_inner(
    local_base_url: &str,
    request: &BridgeRequest,
    owner_device_id: [u8; 16],
    streams: &mut HashMap<[u8; 16], StreamHandle>,
    local_events: &mpsc::Sender<LocalSocketEvent>,
) -> Result<()> {
    if !request.method.eq_ignore_ascii_case("GET")
        || !request.headers.is_empty()
        || !request.body.is_empty()
        || request.parameters.len() != 2
    {
        bail!("WebSocket open request has invalid fields");
    }
    if streams.contains_key(&request.request_id) {
        bail!("WebSocket stream id is already open");
    }
    let session_id = safe_path_segment(&request.parameters[0])?;
    let revision = request.parameters[1]
        .parse::<u64>()
        .context("invalid WebSocket revision")?;
    let mut url = Url::parse(local_base_url).context("parse local WebSocket URL")?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => bail!("local daemon URL must use HTTP or HTTPS"),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("set local WebSocket scheme"))?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/ws"));
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("session", session_id)
        .append_pair("rev", &revision.to_string());

    let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .context("open local Forge session WebSocket")?;
    let (commands_tx, mut commands_rx) = mpsc::channel(64);
    let events = local_events.clone();
    let stream_id = request.request_id;
    tokio::spawn(async move {
        let (mut write, mut read) = socket.split();
        loop {
            tokio::select! {
                command = commands_rx.recv() => match command {
                    Some(LocalSocketCommand::Data(bytes)) => {
                        if write.send(Message::Binary(bytes.into())).await.is_err() { break; }
                    }
                    Some(LocalSocketCommand::Close) | None => {
                        let _ = write.send(Message::Close(None)).await;
                        break;
                    }
                },
                message = read.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        if events.send(LocalSocketEvent::Data {
                            stream_id,
                            owner_device_id,
                            bytes: text.as_bytes().to_vec(),
                        }).await.is_err() { break; }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        if events.send(LocalSocketEvent::Data {
                            stream_id,
                            owner_device_id,
                            bytes: bytes.to_vec(),
                        }).await.is_err() { break; }
                    }
                    Some(Ok(Message::Ping(bytes))) => {
                        if write.send(Message::Pong(bytes)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Frame(_))) => {}
                }
            }
        }
        let _ = events
            .send(LocalSocketEvent::Closed {
                stream_id,
                owner_device_id,
            })
            .await;
    });
    streams.insert(
        stream_id,
        StreamHandle {
            owner_device_id,
            commands: commands_tx,
        },
    );
    Ok(())
}

fn bridge_error(request_id: [u8; 16], status: u16, message: &str) -> BridgeResponse {
    BridgeResponse {
        request_id,
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::json!({ "error": message })
            .to_string()
            .into_bytes(),
        body_blob: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn request(route: RouteId, method: &str, parameters: &[&str]) -> BridgeRequest {
        BridgeRequest {
            request_id: [1; 16],
            route,
            method: method.into(),
            parameters: parameters.iter().map(|value| (*value).into()).collect(),
            headers: Vec::new(),
            body: Vec::new(),
            body_blob: None,
        }
    }

    #[test]
    fn route_table_rejects_method_mismatch_and_path_injection() {
        let list = request(RouteId::ListSessions, "POST", &[""]);
        let target = route_target(&list).expect("typed route exists");
        assert_ne!(Method::POST, target.method);

        let injected = request(RouteId::ArchiveSession, "POST", &["../token", ""]);
        assert!(route_target(&injected).is_err());
    }

    #[test]
    fn route_table_maps_only_explicit_local_paths() {
        let archive = request(RouteId::ArchiveSession, "POST", &["session-1", "?force=0"]);
        let target = route_target(&archive).expect("archive route");
        assert_eq!(target.method, Method::POST);
        assert_eq!(target.path, "/api/sessions/session-1/archive");
        assert_eq!(target.query.as_deref(), Some("force=0"));
        assert!(route_target(&request(RouteId::SessionInput, "POST", &[])).is_err());
    }

    #[test]
    fn relay_url_keeps_ticket_out_of_paths() {
        let url = relay_url("https://app.forge.adulari.dev", "ticket-value").expect("relay URL");
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.path(), "/v1/relay");
        assert_eq!(url.query(), Some("ticket=ticket-value"));
    }

    #[test]
    fn outbound_sequence_is_reserved_before_envelope_use_and_replay_is_rejected() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = StateStore {
            path: temp.path().join("anywhere/state.json"),
        };
        let signing_seed = [0x44; 32];
        let data_key = [0x55; 32];
        store
            .save(&LocalState {
                version: super::super::STATE_VERSION,
                account_id: Some(hex::encode([0x11; 16])),
                device_id: Some(hex::encode([0x22; 16])),
                signing_private_key: Some(URL_SAFE_NO_PAD.encode(signing_seed)),
                account_data_key: Some(URL_SAFE_NO_PAD.encode(data_key)),
                key_epoch: Some(3),
                host_id: Some(hex::encode([0x33; 16])),
                ..LocalState::default()
            })
            .expect("save state");

        let encoded = seal_outbound(
            &store,
            EnvelopeKind::BridgeResponse,
            [0x66; 16],
            b"response",
        )
        .expect("seal response");
        assert_eq!(store.load().expect("reload state").next_sequence, 1);
        let envelope = Envelope::decode(&encoded).expect("decode response");
        assert_eq!(envelope.metadata.sequence, 0);
        assert_eq!(envelope.metadata.recipient_id, [0x66; 16]);
        assert_eq!(
            envelope
                .open(
                    &data_key,
                    &SigningKey::from_bytes(&signing_seed).verifying_key()
                )
                .expect("open response"),
            b"response"
        );

        accept_inbound_envelopes(&store, [0x77; 16], 3, None, 9).expect("accept sequence");
        assert!(accept_inbound_envelopes(&store, [0x77; 16], 3, None, 9).is_err());
        assert!(accept_inbound_envelopes(&store, [0x77; 16], 3, None, 8).is_err());
        accept_inbound_envelopes(&store, [0x77; 16], 3, Some(10), 11)
            .expect("accept blob and referencing sequence atomically");
        assert!(accept_inbound_envelopes(&store, [0x77; 16], 3, Some(10), 12).is_err());
        assert!(accept_inbound_envelopes(&store, [0x77; 16], 3, Some(13), 13).is_err());
    }

    #[test]
    fn relay_blob_verification_binds_hash_sender_recipient_and_plaintext() {
        let sender_signing_key = SigningKey::from_bytes(&[0x44; 32]);
        let identity = Identity {
            account_id: [0x11; 16],
            device_id: [0x22; 16],
            host_id: [0x33; 16],
            data_key: [0x55; 32],
            key_epoch: 3,
            signing_key: SigningKey::from_bytes(&[0x66; 32]),
        };
        let envelope = Envelope::seal(
            EnvelopeMetadata {
                kind: EnvelopeKind::Blob,
                flags: 0,
                account_id: identity.account_id,
                sender_device_id: [0x77; 16],
                recipient_kind: RecipientKind::Host,
                recipient_id: identity.host_id,
                key_epoch: identity.key_epoch,
                sequence: 8,
                created_at_ms: 1,
                nonce: [0x88; 24],
            },
            b"externalized bytes",
            &identity.data_key,
            &sender_signing_key,
        )
        .expect("seal blob");
        let encoded = envelope.encode().expect("encode blob");
        let reference = RelayBlobReference {
            blob_id: [0x99; 16],
            ciphertext_bytes: encoded.len() as u64,
            ciphertext_sha256: Sha256::digest(&encoded).into(),
        };

        let verified = verify_blob_object(
            &identity,
            &sender_signing_key.verifying_key(),
            [0x77; 16],
            reference,
            &encoded,
        )
        .expect("verify blob");
        assert_eq!(verified.sequence, 8);
        assert_eq!(verified.plaintext, b"externalized bytes");

        let mut wrong_hash = reference;
        wrong_hash.ciphertext_sha256[0] ^= 1;
        assert!(verify_blob_object(
            &identity,
            &sender_signing_key.verifying_key(),
            [0x77; 16],
            wrong_hash,
            &encoded,
        )
        .is_err());
        assert!(verify_blob_object(
            &identity,
            &sender_signing_key.verifying_key(),
            [0x78; 16],
            reference,
            &encoded,
        )
        .is_err());
    }
}
