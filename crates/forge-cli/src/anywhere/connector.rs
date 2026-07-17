//! Managed encrypted relay connector for `forge serve`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use forge_anywhere_protocol::bridge::{
    BridgeRequest, BridgeResponse, FrameDirection, RelayBlobReference, RouteId, WebSocketFrame,
    WebSocketFrameKind,
};
use forge_anywhere_protocol::{
    CommandAcknowledgement, CommandErrorCode, CommandId, CommandResult, Envelope, EnvelopeKind,
    EnvelopeMetadata, QueuedCommandList, QueuedCommandMetadata, RecipientKind,
    COMMAND_LIST_VERSION,
};
use futures::{SinkExt, StreamExt};
use reqwest::header::{HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Client, Method, RequestBuilder, Response, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::{
    client, decode_base64_array, decode_hex_array, decode_response, ensure_access_token,
    idempotency_key, now_ms, refresh_account_epoch, send_json, CommandJournalEntry,
    CommandJournalState, DeviceList, LocalState, StateStore,
};

const MAX_INLINE_BODY: usize = 256 * 1024;
const MAX_BLOB_BYTES: u64 = 32 * 1024 * 1024;
const MAX_QUERY_LEN: usize = 4096;
const RECONNECT_DELAY: Duration = Duration::from_secs(10);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_secs(5);
const COMMAND_WORKER_LEASE_MS: u64 = 2 * 60 * 1000;
const COMMAND_JOURNAL_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const OCTET_STREAM: &str = "application/octet-stream";

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

struct DurableCommandClient<'a> {
    store: &'a StateStore,
    identity: &'a Identity,
    devices: &'a HashMap<[u8; 16], VerifyingKey>,
    local_base_url: &'a str,
    http: &'a Client,
    service_url: &'a str,
    access_token: &'a str,
    host_id: &'a str,
    worker_id: &'a str,
}

struct VerifiedBlob {
    blob_id: [u8; 16],
    plaintext: Vec<u8>,
    sequence: u64,
}

struct VerifiedCommand {
    sender_device_id: [u8; 16],
    key_epoch: u32,
    sequence: u64,
    plaintext: Vec<u8>,
}

#[derive(Clone)]
struct PendingAcknowledgement {
    envelope: Vec<u8>,
    idempotency_key: String,
}

enum CommandJournalStatus {
    New,
    Claimed,
    Busy,
    DispatchUncertain,
    AcknowledgementReady(PendingAcknowledgement),
    Acked,
}

struct Identity {
    account_id: [u8; 16],
    device_id: [u8; 16],
    host_id: [u8; 16],
    data_key: [u8; 32],
    data_key_epochs: BTreeMap<u32, [u8; 32]>,
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
        let command_base_url = local_base_url.clone();
        tokio::join!(
            relay_connector_loop(&local_base_url),
            command_worker_loop(&command_base_url),
            super::handoff::capsule_worker_loop()
        );
    })
}

async fn relay_connector_loop(local_base_url: &str) {
    let mut last_error = String::new();
    loop {
        match connect_once(local_base_url).await {
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
}

async fn command_worker_loop(local_base_url: &str) {
    let worker_id = hex::encode(rand::random::<[u8; 16]>());
    let mut last_error = String::new();
    let mut interval = tokio::time::interval(COMMAND_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match poll_durable_commands(local_base_url, &worker_id).await {
            Ok(()) => last_error.clear(),
            Err(error) => {
                let message = format!("{error:#}");
                if message != last_error {
                    eprintln!(
                        "⚠ Forge Anywhere remote jobs unavailable — live relay and local/direct Forge are unaffected: {message}"
                    );
                    last_error = message;
                }
            }
        }
    }
}

async fn poll_durable_commands(local_base_url: &str, worker_id: &str) -> Result<()> {
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
    DurableCommandClient {
        store: &store,
        identity: &identity,
        devices: &devices,
        local_base_url,
        http: &http,
        service_url: &service_url,
        access_token: &access_token,
        host_id: &host_id,
        worker_id,
    }
    .poll()
    .await
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

impl DurableCommandClient<'_> {
    async fn poll(&self) -> Result<()> {
        let list: QueuedCommandList = send_json(
            self.http
                .get(format!(
                    "{}/v1/hosts/{}/commands",
                    self.service_url, self.host_id
                ))
                .bearer_auth(self.access_token),
        )
        .await
        .context("list durable Anywhere commands")?;
        if list.version != COMMAND_LIST_VERSION {
            bail!("service returned an unsupported durable command list version");
        }
        list.validate()
            .context("validate durable command list metadata")?;

        let mut command_ids = HashSet::with_capacity(list.commands.len());
        for metadata in list.commands {
            if !command_ids.insert(metadata.command_id) {
                bail!("service returned a duplicate durable command id");
            }
            self.process(metadata).await?;
        }
        Ok(())
    }

    async fn process(&self, metadata: QueuedCommandMetadata) -> Result<()> {
        let now = now_ms();
        if metadata.expires_at_ms <= now {
            return Ok(());
        }
        prune_command_journal(self.store, now)?;
        match command_journal_status(self.store, &metadata, self.worker_id, now)? {
            CommandJournalStatus::AcknowledgementReady(acknowledgement) => {
                return self
                    .post_and_mark_acked(metadata.command_id, &acknowledgement)
                    .await;
            }
            CommandJournalStatus::DispatchUncertain => {
                let acknowledgement = ensure_command_acknowledgement(
                    self.store,
                    metadata.command_id,
                    CommandResult::Error {
                        code: CommandErrorCode::ExecutionFailed,
                        retryable: false,
                    },
                )?;
                return self
                    .post_and_mark_acked(metadata.command_id, &acknowledgement)
                    .await;
            }
            CommandJournalStatus::Busy | CommandJournalStatus::Acked => return Ok(()),
            CommandJournalStatus::Claimed => {
                bail!("durable command was claimed without verified ciphertext")
            }
            CommandJournalStatus::New => {}
        }

        let encoded = self.fetch_command(&metadata).await?;
        let verified = verify_command_envelope(&metadata, &encoded, self.identity, self.devices)?;
        match begin_command(self.store, &metadata, &verified, self.worker_id, now_ms())? {
            CommandJournalStatus::AcknowledgementReady(acknowledgement) => {
                return self
                    .post_and_mark_acked(metadata.command_id, &acknowledgement)
                    .await;
            }
            CommandJournalStatus::DispatchUncertain => {
                let acknowledgement = ensure_command_acknowledgement(
                    self.store,
                    metadata.command_id,
                    CommandResult::Error {
                        code: CommandErrorCode::ExecutionFailed,
                        retryable: false,
                    },
                )?;
                return self
                    .post_and_mark_acked(metadata.command_id, &acknowledgement)
                    .await;
            }
            CommandJournalStatus::Busy | CommandJournalStatus::Acked => return Ok(()),
            CommandJournalStatus::Claimed => {}
            CommandJournalStatus::New => bail!("durable command claim was not persisted"),
        }

        let result = match decode_command_request(&verified.plaintext) {
            Ok(request) => match validate_command_request(&request) {
                Ok(()) => match dispatch_http_inner(self.local_base_url, &request).await {
                    Ok(response) => command_result_for_status(response.status),
                    Err(_) => CommandResult::Error {
                        code: CommandErrorCode::HostUnavailable,
                        retryable: true,
                    },
                },
                Err(code) => CommandResult::Error {
                    code,
                    retryable: false,
                },
            },
            Err(_) => CommandResult::Error {
                code: CommandErrorCode::InvalidCommand,
                retryable: false,
            },
        };
        let acknowledgement =
            ensure_command_acknowledgement(self.store, metadata.command_id, result)?;
        self.post_and_mark_acked(metadata.command_id, &acknowledgement)
            .await
    }

    async fn fetch_command(&self, metadata: &QueuedCommandMetadata) -> Result<Vec<u8>> {
        let response = self
            .http
            .get(format!(
                "{}/v1/hosts/{}/commands/{}",
                self.service_url, self.host_id, metadata.command_id
            ))
            .bearer_auth(self.access_token)
            .header(ACCEPT, OCTET_STREAM)
            .send()
            .await
            .context("fetch durable Anywhere command")?
            .error_for_status()
            .context("service rejected durable command fetch")?;
        if response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            != Some(OCTET_STREAM)
        {
            bail!("durable command response has the wrong content type");
        }
        if response
            .content_length()
            .is_some_and(|length| length != metadata.ciphertext_bytes)
        {
            bail!("durable command length does not match its metadata");
        }

        let capacity = usize::try_from(metadata.ciphertext_bytes)
            .context("durable command length exceeds this platform")?;
        let mut bytes = Vec::with_capacity(capacity);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("read durable Anywhere command")?;
            let next_len = bytes
                .len()
                .checked_add(chunk.len())
                .context("durable command length overflow")?;
            if next_len > capacity {
                bail!("durable command exceeds its declared length");
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != capacity {
            bail!("durable command length does not match its metadata");
        }
        Ok(bytes)
    }

    async fn post_acknowledgement(
        &self,
        command_id: CommandId,
        acknowledgement: &PendingAcknowledgement,
    ) -> Result<()> {
        let request = || {
            self.http
                .post(format!(
                    "{}/v1/hosts/{}/commands/{}/ack",
                    self.service_url, self.host_id, command_id
                ))
                .bearer_auth(self.access_token)
                .header(CONTENT_TYPE, OCTET_STREAM)
                .header("Idempotency-Key", &acknowledgement.idempotency_key)
                .body(acknowledgement.envelope.clone())
        };
        send_transport_retry_once(request, "post durable command acknowledgement")
            .await?
            .error_for_status()
            .context("service rejected durable command acknowledgement")?;
        Ok(())
    }

    async fn post_and_mark_acked(
        &self,
        command_id: CommandId,
        acknowledgement: &PendingAcknowledgement,
    ) -> Result<()> {
        self.post_acknowledgement(command_id, acknowledgement)
            .await?;
        mark_command_acked(self.store, command_id, now_ms())
    }
}

fn verify_command_envelope(
    metadata: &QueuedCommandMetadata,
    encoded: &[u8],
    identity: &Identity,
    devices: &HashMap<[u8; 16], VerifyingKey>,
) -> Result<VerifiedCommand> {
    if encoded.len() as u64 != metadata.ciphertext_bytes {
        bail!("durable command length does not match its metadata");
    }
    let envelope = Envelope::decode(encoded).context("decode durable command envelope")?;
    if envelope.metadata.kind != EnvelopeKind::Command {
        bail!("durable command envelope has the wrong kind");
    }
    validate_inbound_routing(&envelope, identity)?;
    if envelope.metadata.sender_device_id != metadata.sender_device_id
        || envelope.metadata.created_at_ms != metadata.created_at_ms
    {
        bail!("durable command envelope does not match its queue metadata");
    }
    let sender_device_id = envelope.metadata.sender_device_id;
    let verifying_key = devices
        .get(&sender_device_id)
        .context("durable command came from an unknown or revoked device")?;
    let data_key = identity
        .data_key_epochs
        .get(&envelope.metadata.key_epoch)
        .context("durable command uses an unavailable Account Data Key epoch")?;
    let plaintext = envelope
        .open(data_key, verifying_key)
        .context("authenticate and decrypt durable command")?;
    Ok(VerifiedCommand {
        sender_device_id,
        key_epoch: envelope.metadata.key_epoch,
        sequence: envelope.metadata.sequence,
        plaintext,
    })
}

fn decode_command_request(plaintext: &[u8]) -> Result<BridgeRequest> {
    let request: BridgeRequest =
        serde_json::from_slice(plaintext).context("decode durable bridge request")?;
    if serde_json::to_vec(&request).context("canonicalize durable bridge request")? != plaintext {
        bail!("durable bridge request is not canonical JSON");
    }
    Ok(request)
}

fn validate_command_request(request: &BridgeRequest) -> std::result::Result<(), CommandErrorCode> {
    if request.body_blob.is_some() {
        return Err(CommandErrorCode::InvalidCommand);
    }
    if request.headers.iter().any(|(name, _)| {
        !matches!(
            name.to_ascii_lowercase().as_str(),
            "accept" | "content-type"
        )
    }) {
        return Err(CommandErrorCode::PermissionDenied);
    }
    if request.route == RouteId::WebSocket {
        return Err(CommandErrorCode::PermissionDenied);
    }
    if request.route == RouteId::Health {
        if request.method != Method::GET.as_str()
            || !request.parameters.is_empty()
            || !request.headers.is_empty()
            || !request.body.is_empty()
        {
            return Err(CommandErrorCode::InvalidCommand);
        }
        return Ok(());
    }
    let target = route_target(request).map_err(|_| CommandErrorCode::PermissionDenied)?;
    let method = Method::from_bytes(request.method.as_bytes())
        .map_err(|_| CommandErrorCode::InvalidCommand)?;
    if method != target.method {
        return Err(CommandErrorCode::PermissionDenied);
    }
    Ok(())
}

fn command_result_for_status(status: u16) -> CommandResult {
    if status < 400 {
        CommandResult::Success
    } else {
        CommandResult::Error {
            code: CommandErrorCode::ExecutionFailed,
            retryable: status >= 500,
        }
    }
}

fn command_journal_status(
    store: &StateStore,
    metadata: &QueuedCommandMetadata,
    worker_id: &str,
    now: u64,
) -> Result<CommandJournalStatus> {
    let state = store.load()?;
    let Some(entry) = state.command_journal.get(&metadata.command_id.to_string()) else {
        return Ok(CommandJournalStatus::New);
    };
    validate_journal_metadata(entry, metadata)?;
    journal_entry_status(entry, worker_id, now)
}

fn journal_entry_status(
    entry: &CommandJournalEntry,
    _worker_id: &str,
    now: u64,
) -> Result<CommandJournalStatus> {
    match &entry.state {
        CommandJournalState::DispatchStarted { lease_until_ms, .. } if *lease_until_ms > now => {
            Ok(CommandJournalStatus::Busy)
        }
        CommandJournalState::DispatchStarted { .. } => Ok(CommandJournalStatus::DispatchUncertain),
        CommandJournalState::AcknowledgementReady {
            envelope,
            idempotency_key,
            ..
        } => Ok(CommandJournalStatus::AcknowledgementReady(
            PendingAcknowledgement {
                envelope: URL_SAFE_NO_PAD
                    .decode(envelope)
                    .context("decode persisted command acknowledgement")?,
                idempotency_key: idempotency_key.clone(),
            },
        )),
        CommandJournalState::Acked { .. } => Ok(CommandJournalStatus::Acked),
    }
}

fn validate_journal_metadata(
    entry: &CommandJournalEntry,
    metadata: &QueuedCommandMetadata,
) -> Result<()> {
    if entry.sender_device_id != hex::encode(metadata.sender_device_id)
        || entry.created_at_ms != metadata.created_at_ms
        || entry.expires_at_ms != metadata.expires_at_ms
        || entry.ciphertext_bytes != metadata.ciphertext_bytes
    {
        bail!("durable command metadata changed after it was journaled");
    }
    Ok(())
}

fn begin_command(
    store: &StateStore,
    metadata: &QueuedCommandMetadata,
    verified: &VerifiedCommand,
    worker_id: &str,
    now: u64,
) -> Result<CommandJournalStatus> {
    let command_id = metadata.command_id.to_string();
    let mut status = CommandJournalStatus::New;
    store.update(|state| {
        if let Some(entry) = state.command_journal.get(&command_id) {
            validate_journal_metadata(entry, metadata)?;
            if entry.sender_device_id != hex::encode(verified.sender_device_id)
                || entry.key_epoch != verified.key_epoch
                || entry.sequence != verified.sequence
            {
                bail!("durable command envelope changed after it was journaled");
            }
            status = journal_entry_status(entry, worker_id, now)?;
            return Ok(());
        }
        let sender_device_id = hex::encode(verified.sender_device_id);
        if state.command_journal.iter().any(|(other_id, entry)| {
            other_id != &command_id
                && entry.sender_device_id == sender_device_id
                && entry.key_epoch == verified.key_epoch
                && entry.sequence == verified.sequence
        }) {
            bail!("durable command reused an authenticated sender/epoch/sequence tuple");
        }
        let lease_until_ms = now
            .checked_add(COMMAND_WORKER_LEASE_MS)
            .context("durable command worker lease overflow")?;
        state.command_journal.insert(
            command_id,
            CommandJournalEntry {
                sender_device_id,
                key_epoch: verified.key_epoch,
                sequence: verified.sequence,
                created_at_ms: metadata.created_at_ms,
                expires_at_ms: metadata.expires_at_ms,
                ciphertext_bytes: metadata.ciphertext_bytes,
                state: CommandJournalState::DispatchStarted {
                    worker_id: worker_id.to_owned(),
                    lease_until_ms,
                },
            },
        );
        status = CommandJournalStatus::Claimed;
        Ok(())
    })?;
    Ok(status)
}

fn ensure_command_acknowledgement(
    store: &StateStore,
    command_id: CommandId,
    result: CommandResult,
) -> Result<PendingAcknowledgement> {
    let journal_key = command_id.to_string();
    let state = store.load()?;
    let entry = state
        .command_journal
        .get(&journal_key)
        .context("durable command is missing from its journal")?;
    match &entry.state {
        CommandJournalState::AcknowledgementReady {
            envelope,
            idempotency_key,
            ..
        } => {
            return Ok(PendingAcknowledgement {
                envelope: URL_SAFE_NO_PAD
                    .decode(envelope)
                    .context("decode persisted command acknowledgement")?,
                idempotency_key: idempotency_key.clone(),
            });
        }
        CommandJournalState::Acked { .. } => bail!("durable command is already acknowledged"),
        CommandJournalState::DispatchStarted { .. } => {}
    }
    let recipient_device_id =
        decode_hex_array(&entry.sender_device_id, "command sender device id")?;
    let plaintext = serde_json::to_vec(&CommandAcknowledgement { command_id, result })
        .context("encode durable command acknowledgement")?;
    let envelope = seal_for_recipient(
        store,
        EnvelopeKind::Acknowledgement,
        RecipientKind::Device,
        recipient_device_id,
        &plaintext,
    )?;
    let pending = PendingAcknowledgement {
        envelope,
        idempotency_key: idempotency_key(),
    };
    let mut persisted = pending.clone();
    store.update(|state| {
        let entry = state
            .command_journal
            .get_mut(&journal_key)
            .context("durable command disappeared from its journal")?;
        match &entry.state {
            CommandJournalState::AcknowledgementReady {
                envelope,
                idempotency_key,
                ..
            } => {
                persisted = PendingAcknowledgement {
                    envelope: URL_SAFE_NO_PAD
                        .decode(envelope)
                        .context("decode persisted command acknowledgement")?,
                    idempotency_key: idempotency_key.clone(),
                };
            }
            CommandJournalState::DispatchStarted { .. } => {
                entry.state = CommandJournalState::AcknowledgementReady {
                    result,
                    envelope: URL_SAFE_NO_PAD.encode(&pending.envelope),
                    idempotency_key: pending.idempotency_key.clone(),
                };
            }
            CommandJournalState::Acked { .. } => {
                bail!("durable command was acknowledged concurrently")
            }
        }
        Ok(())
    })?;
    Ok(persisted)
}

fn mark_command_acked(store: &StateStore, command_id: CommandId, acked_at_ms: u64) -> Result<()> {
    let journal_key = command_id.to_string();
    store.update(|state| {
        let entry = state
            .command_journal
            .get_mut(&journal_key)
            .context("acknowledged durable command is missing from its journal")?;
        match entry.state {
            CommandJournalState::AcknowledgementReady { .. } => {
                entry.state = CommandJournalState::Acked { acked_at_ms };
            }
            CommandJournalState::Acked { .. } => {}
            CommandJournalState::DispatchStarted { .. } => {
                bail!("durable command acknowledgement was not persisted")
            }
        }
        Ok(())
    })?;
    Ok(())
}

fn prune_command_journal(store: &StateStore, now: u64) -> Result<()> {
    store.update(|state| {
        state.command_journal.retain(|_, entry| {
            entry
                .expires_at_ms
                .checked_add(COMMAND_JOURNAL_RETENTION_MS)
                .is_some_and(|retain_until| retain_until > now)
        });
        Ok(())
    })?;
    Ok(())
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
    validate_inbound_routing(envelope, identity)?;
    if envelope.metadata.key_epoch != identity.key_epoch {
        bail!("relay envelope uses an unavailable account key epoch");
    }
    Ok(())
}

fn validate_inbound_routing(envelope: &Envelope, identity: &Identity) -> Result<()> {
    let metadata = &envelope.metadata;
    if metadata.account_id != identity.account_id {
        bail!("relay envelope account does not match this host");
    }
    if metadata.recipient_kind != RecipientKind::Host || metadata.recipient_id != identity.host_id {
        bail!("relay envelope is not addressed to this host");
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
    store.update(|state| {
        let first_sequence = blob_sequence.unwrap_or(control_sequence);
        if blob_sequence.is_some_and(|sequence| sequence >= control_sequence) {
            bail!("relay blob sequence must precede its referencing envelope");
        }
        accept_inbound_sequence(state, sender_device_id, key_epoch, first_sequence)?;
        let namespace = format!("{}:{key_epoch}", hex::encode(sender_device_id));
        state.accepted_sequences.insert(namespace, control_sequence);
        Ok(())
    })?;
    Ok(())
}

fn accept_inbound_sequence(
    state: &mut LocalState,
    sender_device_id: [u8; 16],
    key_epoch: u32,
    sequence: u64,
) -> Result<()> {
    let namespace = format!("{}:{key_epoch}", hex::encode(sender_device_id));
    if state
        .accepted_sequences
        .get(&namespace)
        .is_some_and(|last| sequence <= *last)
    {
        bail!("replayed or out-of-order relay sequence");
    }
    state.accepted_sequences.insert(namespace, sequence);
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
    let data_key_epochs = state
        .data_key_epochs
        .iter()
        .map(|(epoch, encoded)| {
            decode_base64_array(encoded, "retained Account Data Key").map(|key| (*epoch, key))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
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
        data_key_epochs,
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
    use std::path::PathBuf;

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
        assert!(validate_command_request(&list).is_err());

        let injected = request(RouteId::ArchiveSession, "POST", &["../token", ""]);
        assert!(route_target(&injected).is_err());
        assert!(validate_command_request(&injected).is_err());

        let mut disallowed_header = request(RouteId::CreateSession, "POST", &[""]);
        disallowed_header
            .headers
            .push(("authorization".into(), "daemon-secret".into()));
        assert!(validate_command_request(&disallowed_header).is_err());
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
            data_key_epochs: BTreeMap::from([(3, [0x55; 32])]),
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

    fn durable_test_store(path: PathBuf) -> StateStore {
        let store = StateStore { path };
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
            .expect("save durable test state");
        store
    }

    fn durable_test_identity() -> Identity {
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

    fn queued_metadata(
        command_id: [u8; 16],
        sender_device_id: [u8; 16],
        created_at_ms: u64,
        ciphertext_bytes: u64,
    ) -> QueuedCommandMetadata {
        QueuedCommandMetadata {
            command_id: CommandId::new(command_id),
            sender_device_id,
            created_at_ms,
            expires_at_ms: created_at_ms + forge_anywhere_protocol::COMMAND_EXPIRY_MS,
            ciphertext_bytes,
        }
    }

    #[test]
    fn durable_command_journal_prevents_replay_after_reload() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("anywhere/state.json");
        let store = durable_test_store(path.clone());
        let metadata = queued_metadata([0xaa; 16], [0x66; 16], 100, 512);
        let verified = VerifiedCommand {
            sender_device_id: metadata.sender_device_id,
            key_epoch: 3,
            sequence: 9,
            plaintext: b"secret prompt and filename".to_vec(),
        };

        assert!(matches!(
            begin_command(&store, &metadata, &verified, "worker-a", 200).expect("begin command"),
            CommandJournalStatus::Claimed
        ));
        let reloaded = StateStore { path };
        assert!(matches!(
            command_journal_status(&reloaded, &metadata, "worker-b", 201).expect("reload journal"),
            CommandJournalStatus::Busy
        ));
        assert!(matches!(
            command_journal_status(&reloaded, &metadata, "worker-a", 201)
                .expect("same worker observes live lease"),
            CommandJournalStatus::Busy
        ));
        assert!(matches!(
            begin_command(&reloaded, &metadata, &verified, "worker-b", 201)
                .expect("observe replay"),
            CommandJournalStatus::Busy
        ));
        let state_json = std::fs::read_to_string(&reloaded.path).expect("read state");
        assert!(!state_json.contains("secret prompt"));
        assert!(reloaded
            .load()
            .expect("load replay state")
            .accepted_sequences
            .is_empty());
        assert!(matches!(
            command_journal_status(
                &reloaded,
                &metadata,
                "worker-a",
                200 + COMMAND_WORKER_LEASE_MS
            )
            .expect("same worker observes expired lease"),
            CommandJournalStatus::DispatchUncertain
        ));
    }

    #[test]
    fn crashed_dispatch_gets_categorical_ack_without_plaintext_persistence() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("anywhere/state.json");
        let store = durable_test_store(path.clone());
        let metadata = queued_metadata([0xaa; 16], [0x66; 16], 100, 512);
        begin_command(
            &store,
            &metadata,
            &VerifiedCommand {
                sender_device_id: metadata.sender_device_id,
                key_epoch: 3,
                sequence: 9,
                plaintext: b"daemon token and response body".to_vec(),
            },
            "crashed-worker",
            100,
        )
        .expect("persist pre-dispatch journal");

        let restarted = StateStore { path };
        let pending = ensure_command_acknowledgement(
            &restarted,
            metadata.command_id,
            CommandResult::Error {
                code: CommandErrorCode::ExecutionFailed,
                retryable: false,
            },
        )
        .expect("prepare crash acknowledgement");
        let envelope = Envelope::decode(&pending.envelope).expect("decode acknowledgement");
        assert_eq!(envelope.metadata.kind, EnvelopeKind::Acknowledgement);
        assert_eq!(envelope.metadata.recipient_id, metadata.sender_device_id);
        let plaintext = envelope
            .open(
                &[0x44; 32],
                &SigningKey::from_bytes(&[0x33; 32]).verifying_key(),
            )
            .expect("open acknowledgement");
        let acknowledgement: CommandAcknowledgement =
            serde_json::from_slice(&plaintext).expect("decode acknowledgement");
        assert_eq!(
            acknowledgement.result,
            CommandResult::Error {
                code: CommandErrorCode::ExecutionFailed,
                retryable: false,
            }
        );
        let state_json = std::fs::read_to_string(&restarted.path).expect("read state");
        assert!(!state_json.contains("daemon token"));
        assert!(!state_json.contains("response body"));
        assert!(state_json.contains("execution_failed"));
    }

    #[test]
    fn durable_command_rejects_wrong_routing_and_signer() {
        let identity = durable_test_identity();
        let sender_id = [0x66; 16];
        let sender = SigningKey::from_bytes(&[0x77; 32]);
        let request =
            serde_json::to_vec(&request(RouteId::Health, "GET", &[])).expect("encode request");
        let seal = |recipient_id, signing_key: &SigningKey| {
            Envelope::seal(
                EnvelopeMetadata {
                    kind: EnvelopeKind::Command,
                    flags: 0,
                    account_id: identity.account_id,
                    sender_device_id: sender_id,
                    recipient_kind: RecipientKind::Host,
                    recipient_id,
                    key_epoch: identity.key_epoch,
                    sequence: 12,
                    created_at_ms: 100,
                    nonce: [0x88; 24],
                },
                &request,
                &identity.data_key,
                signing_key,
            )
            .expect("seal command")
            .encode()
            .expect("encode command")
        };

        let wrong_route = seal([0x99; 16], &sender);
        let metadata = queued_metadata([0xaa; 16], sender_id, 100, wrong_route.len() as u64);
        let devices = HashMap::from([(sender_id, sender.verifying_key())]);
        assert!(verify_command_envelope(&metadata, &wrong_route, &identity, &devices).is_err());

        let correct_route = seal(identity.host_id, &sender);
        let metadata = queued_metadata([0xaa; 16], sender_id, 100, correct_route.len() as u64);
        let wrong_signer = SigningKey::from_bytes(&[0x79; 32]);
        let devices = HashMap::from([(sender_id, wrong_signer.verifying_key())]);
        assert!(verify_command_envelope(&metadata, &correct_route, &identity, &devices).is_err());
    }

    #[test]
    fn durable_ack_retries_reuse_exact_bytes_and_idempotency_key() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("anywhere/state.json");
        let store = durable_test_store(path.clone());
        let metadata = queued_metadata([0xaa; 16], [0x66; 16], 100, 512);
        begin_command(
            &store,
            &metadata,
            &VerifiedCommand {
                sender_device_id: metadata.sender_device_id,
                key_epoch: 3,
                sequence: 9,
                plaintext: Vec::new(),
            },
            "worker-a",
            100,
        )
        .expect("begin command");
        let first =
            ensure_command_acknowledgement(&store, metadata.command_id, CommandResult::Success)
                .expect("prepare acknowledgement");

        let restarted = StateStore { path };
        let retry = ensure_command_acknowledgement(
            &restarted,
            metadata.command_id,
            CommandResult::Error {
                code: CommandErrorCode::HostUnavailable,
                retryable: true,
            },
        )
        .expect("reuse acknowledgement");
        assert_eq!(retry.envelope, first.envelope);
        assert_eq!(retry.idempotency_key, first.idempotency_key);
        assert_eq!(restarted.load().expect("load state").next_sequence, 1);
    }

    #[test]
    fn concurrent_workers_claim_a_command_once() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("anywhere/state.json");
        durable_test_store(path.clone());
        let metadata = queued_metadata([0xa1; 16], [0x61; 16], 100, 512);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = ["worker-a", "worker-b"].map(|worker| {
            let path = path.clone();
            let metadata = metadata.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = StateStore { path };
                let verified = VerifiedCommand {
                    sender_device_id: metadata.sender_device_id,
                    key_epoch: 3,
                    sequence: 42,
                    plaintext: Vec::new(),
                };
                barrier.wait();
                begin_command(&store, &metadata, &verified, worker, 500).expect("claim command")
            })
        });
        let statuses = handles.map(|handle| handle.join().expect("join command worker"));
        assert_eq!(
            statuses
                .iter()
                .filter(|status| matches!(status, CommandJournalStatus::Claimed))
                .count(),
            1
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| matches!(status, CommandJournalStatus::Busy))
                .count(),
            1
        );
    }

    #[test]
    fn durable_commands_allow_sparse_order_but_reject_exact_tuple_reuse() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = durable_test_store(temp.path().join("anywhere/state.json"));
        let sender = [0x62; 16];
        let first = queued_metadata([0xa2; 16], sender, 100, 512);
        let out_of_order = queued_metadata([0xa3; 16], sender, 101, 512);
        let reused = queued_metadata([0xa4; 16], sender, 102, 512);
        let verified = |sequence| VerifiedCommand {
            sender_device_id: sender,
            key_epoch: 3,
            sequence,
            plaintext: Vec::new(),
        };

        assert!(matches!(
            begin_command(&store, &first, &verified(99), "worker", 200)
                .expect("claim first command"),
            CommandJournalStatus::Claimed
        ));
        assert!(matches!(
            begin_command(&store, &out_of_order, &verified(7), "worker", 201)
                .expect("claim sparse command"),
            CommandJournalStatus::Claimed
        ));
        assert!(begin_command(&store, &reused, &verified(99), "worker", 202).is_err());
    }

    #[test]
    fn durable_command_uses_retained_epoch_and_rejects_revoked_sender() {
        let mut identity = durable_test_identity();
        identity.key_epoch = 4;
        identity.data_key = [0x45; 32];
        identity.data_key_epochs.insert(4, [0x45; 32]);
        let sender_id = [0x63; 16];
        let sender = SigningKey::from_bytes(&[0x73; 32]);
        let request =
            serde_json::to_vec(&request(RouteId::Health, "GET", &[])).expect("encode request");
        let encoded = Envelope::seal(
            EnvelopeMetadata {
                kind: EnvelopeKind::Command,
                flags: 0,
                account_id: identity.account_id,
                sender_device_id: sender_id,
                recipient_kind: RecipientKind::Host,
                recipient_id: identity.host_id,
                key_epoch: 3,
                sequence: 12,
                created_at_ms: 100,
                nonce: [0x83; 24],
            },
            &request,
            &[0x44; 32],
            &sender,
        )
        .expect("seal retained-epoch command")
        .encode()
        .expect("encode retained-epoch command");
        let metadata = queued_metadata([0xa5; 16], sender_id, 100, encoded.len() as u64);

        let active = HashMap::from([(sender_id, sender.verifying_key())]);
        assert_eq!(
            verify_command_envelope(&metadata, &encoded, &identity, &active)
                .expect("open retained epoch")
                .key_epoch,
            3
        );
        assert!(verify_command_envelope(&metadata, &encoded, &identity, &HashMap::new()).is_err());
    }

    #[test]
    fn acknowledged_commands_are_terminal_and_pruned_after_retention() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = durable_test_store(temp.path().join("anywhere/state.json"));
        let metadata = queued_metadata([0xa6; 16], [0x64; 16], 100, 512);
        begin_command(
            &store,
            &metadata,
            &VerifiedCommand {
                sender_device_id: metadata.sender_device_id,
                key_epoch: 3,
                sequence: 1,
                plaintext: Vec::new(),
            },
            "worker",
            200,
        )
        .expect("claim command");
        ensure_command_acknowledgement(&store, metadata.command_id, CommandResult::Success)
            .expect("persist acknowledgement");
        mark_command_acked(&store, metadata.command_id, 300).expect("mark acknowledged");
        assert!(matches!(
            command_journal_status(&store, &metadata, "other", 400)
                .expect("read acknowledged command"),
            CommandJournalStatus::Acked
        ));

        prune_command_journal(
            &store,
            metadata.expires_at_ms + COMMAND_JOURNAL_RETENTION_MS,
        )
        .expect("prune expired journal");
        assert!(!store
            .load()
            .expect("load pruned journal")
            .command_journal
            .contains_key(&metadata.command_id.to_string()));
    }

    #[test]
    fn durable_websocket_commands_are_rejected() {
        let request = request(RouteId::WebSocket, "GET", &["session", "8"]);
        assert_eq!(
            validate_command_request(&request),
            Err(CommandErrorCode::PermissionDenied)
        );
    }
}
