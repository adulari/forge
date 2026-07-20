//! Durable encrypted upload worker for the local Forge sync journal.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use forge_anywhere_protocol::sync::{SyncOperation, SyncRecord, SyncRecordKind};
use forge_anywhere_protocol::{Envelope, EnvelopeKind, EnvelopeMetadata, RecipientKind};
use forge_store::{RemoteSyncRecord, Store, SyncJournalEntry, SyncUploadEnvelope};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{
    client, decode_base64_array, decode_hex_array, ensure_access_token, now_ms,
    refresh_account_epoch, send_json, LocalState, StateStore,
};

const SYNC_INTERVAL: Duration = Duration::from_secs(10);
const MAX_SYNC_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
const BATCH_SIZE: usize = 10;

#[derive(Serialize)]
struct StartUploadRequest<'a> {
    record_kind: &'a str,
    stable_id: &'a str,
    revision: u64,
    logical_clock: u64,
    operation: &'a str,
    base_hash: Option<String>,
    content_hash: String,
    ciphertext_bytes: usize,
    ciphertext_sha256: String,
}

#[derive(Deserialize)]
struct StartUploadResponse {
    upload_id: String,
    upload_url: Option<String>,
    #[serde(default)]
    required_headers: BTreeMap<String, String>,
    #[serde(default)]
    already_complete: bool,
}

#[derive(Deserialize)]
struct CompleteUploadResponse {
    #[allow(dead_code)]
    cursor: i64,
}

#[derive(Deserialize)]
struct ChangeFeed {
    changes: Vec<Change>,
    next_cursor: i64,
}

#[derive(Deserialize)]
struct Change {
    cursor: i64,
    device_id: String,
    signing_public_key: String,
    record_kind: String,
    stable_id: String,
    revision: u64,
    logical_clock: u64,
    operation: String,
    base_hash: Option<String>,
    content_hash: String,
    ciphertext_bytes: u64,
    ciphertext_sha256: String,
    download_url: String,
}

struct SyncIdentity {
    account_id: [u8; 16],
    device_id: [u8; 16],
    data_key: [u8; 32],
    key_epoch: u32,
    signing_key: SigningKey,
}

pub(super) fn spawn() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut wait_before_next = Duration::ZERO;
        let mut retry_delay = SYNC_INTERVAL;
        let mut last_error = String::new();
        loop {
            tokio::time::sleep(wait_before_next).await;
            match upload_batch().await {
                Ok(()) => {
                    last_error.clear();
                    retry_delay = SYNC_INTERVAL;
                    wait_before_next = SYNC_INTERVAL;
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    if message != last_error {
                        eprintln!("⚠ Forge Anywhere sync paused: {message}");
                        last_error = message;
                    }
                    wait_before_next = retry_delay;
                    retry_delay = next_retry_delay(retry_delay);
                }
            }
        }
    })
}

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_SYNC_RETRY_DELAY)
}

async fn upload_batch() -> Result<()> {
    let config = forge_config::load()?;
    if !config.anywhere.enabled || !config.anywhere.sync {
        return Ok(());
    }
    let state_store = StateStore::platform()?;
    let mut state = state_store.load()?;
    let access_token = ensure_access_token(&state_store, &mut state).await?;
    let service_url = config.anywhere.service_url().to_owned();
    refresh_account_epoch(&state_store, &mut state, &service_url, &access_token).await?;
    let local_device_id = decode_hex_array::<16>(
        state
            .device_id
            .as_deref()
            .context("Anywhere device id is missing")?,
        "device id",
    )?;
    let store = crate::open_store()?;
    let entries = store.pending_sync_journal(BATCH_SIZE)?;
    let http = client()?;
    for entry in entries {
        upload_entry(
            &store,
            &state_store,
            &http,
            &service_url,
            &access_token,
            &entry,
        )
        .await?;
    }
    download_changes(&store, &state_store, &http, &service_url, &access_token).await?;
    let mut conflicts = 0;
    // A feed page may contain children before a parent from an earlier delayed upload. Retry a
    // bounded number of local-only passes so newly satisfied dependencies materialize now, while
    // genuinely missing parents remain durably staged for a future poll.
    for _ in 0..4 {
        let memory = store.apply_staged_memory_records(local_device_id, BATCH_SIZE)?;
        let history = store.apply_staged_history_records(local_device_id, BATCH_SIZE)?;
        let portable = store.apply_staged_portable_records(local_device_id, BATCH_SIZE)?;
        let files = store.apply_staged_file_records(local_device_id, BATCH_SIZE)?;
        conflicts += memory.conflicts + history.conflicts + portable.conflicts + files.conflicts;
        if memory.applied + history.applied + portable.applied + files.applied == 0 {
            break;
        }
    }
    if conflicts > 0 {
        eprintln!("⚠ Forge Anywhere staged {conflicts} conflict(s) without changing local data");
    }
    Ok(())
}

async fn download_changes(
    store: &Store,
    state_store: &StateStore,
    http: &reqwest::Client,
    service_url: &str,
    access_token: &str,
) -> Result<()> {
    let cursor = store.sync_download_cursor()?;
    let feed: ChangeFeed = send_json(
        http.get(format!(
            "{service_url}/v1/sync/changes?cursor={cursor}&limit=100"
        ))
        .bearer_auth(access_token),
    )
    .await
    .context("load encrypted sync changes")?;
    for change in feed.changes {
        download_change(store, state_store, http, change).await?;
    }
    store.advance_sync_download_cursor(feed.next_cursor)?;
    Ok(())
}

async fn download_change(
    store: &Store,
    state_store: &StateStore,
    http: &reqwest::Client,
    change: Change,
) -> Result<()> {
    if change.ciphertext_bytes > 32 * 1024 * 1024 {
        bail!("encrypted sync download exceeds 32 MiB");
    }
    let bytes = http
        .get(&change.download_url)
        .send()
        .await
        .context("download encrypted sync object")?
        .error_for_status()
        .context("R2 rejected encrypted sync download")?
        .bytes()
        .await
        .context("read encrypted sync object")?;
    if bytes.len() as u64 != change.ciphertext_bytes {
        bail!("encrypted sync download length does not match change metadata");
    }
    let expected_ciphertext_hash =
        decode_base64_array::<32>(&change.ciphertext_sha256, "sync ciphertext hash")?;
    if Sha256::digest(&bytes).as_slice() != expected_ciphertext_hash {
        bail!("encrypted sync download hash does not match change metadata");
    }
    let envelope = Envelope::decode(&bytes)?;
    let state = state_store.load()?;
    let account_id = decode_hex_array::<16>(
        state
            .account_id
            .as_deref()
            .context("Anywhere account id is missing")?,
        "account id",
    )?;
    let sender_device_id = decode_hex_array::<16>(&change.device_id, "sync sender device id")?;
    if envelope.metadata.kind != EnvelopeKind::SyncRecord
        || envelope.metadata.account_id != account_id
        || envelope.metadata.sender_device_id != sender_device_id
        || envelope.metadata.recipient_kind != RecipientKind::Account
        || envelope.metadata.recipient_id != account_id
    {
        bail!("encrypted sync envelope routing metadata does not match the change feed");
    }
    let key = state
        .data_key_epochs
        .get(&envelope.metadata.key_epoch)
        .or_else(|| {
            (state.key_epoch == Some(envelope.metadata.key_epoch))
                .then_some(state.account_data_key.as_ref())
                .flatten()
        })
        .context("encrypted sync object uses an unavailable Account Data Key epoch")?;
    let data_key = decode_base64_array::<32>(key, "sync Account Data Key")?;
    let signing_public_key =
        decode_base64_array::<32>(&change.signing_public_key, "sync sender signing key")?;
    let verifying_key = VerifyingKey::from_bytes(&signing_public_key)?;
    let plaintext = envelope.open(&data_key, &verifying_key)?;
    let record: SyncRecord = serde_json::from_slice(&plaintext).context("decode sync record")?;
    let expected_kind = record_kind(&change.record_kind)?;
    let expected_operation = match change.operation.as_str() {
        "upsert" => SyncOperation::Upsert,
        "tombstone" => SyncOperation::Tombstone,
        other => bail!("service returned unsupported sync operation {other}"),
    };
    let content_hash = decode_base64_array::<32>(&change.content_hash, "sync content hash")?;
    let base_hash = change
        .base_hash
        .as_deref()
        .map(|value| decode_base64_array::<32>(value, "sync base hash"))
        .transpose()?;
    if record.stable_id != change.stable_id
        || record.kind != expected_kind
        || record.revision != change.revision
        || record.logical_clock != change.logical_clock
        || record.device_id != sender_device_id
        || record.operation != expected_operation
        || record.base_hash != base_hash
        || record.content_hash != content_hash
    {
        bail!("decrypted sync record does not match authenticated change metadata");
    }
    if Sha256::digest(&record.payload).as_slice() != content_hash {
        bail!("decrypted sync payload does not match its authenticated content hash");
    }
    if record.operation == SyncOperation::Tombstone && !record.payload.is_empty() {
        bail!("decrypted sync tombstone contains an unexpected payload");
    }
    store.stage_remote_sync_record(&RemoteSyncRecord {
        cursor: change.cursor,
        sender_device_id,
        record_kind: change.record_kind,
        stable_id: record.stable_id,
        operation: change.operation,
        revision: record.revision,
        logical_clock: record.logical_clock,
        base_hash: record.base_hash,
        content_hash: record.content_hash,
        payload: record.payload,
    })?;
    Ok(())
}

async fn upload_entry(
    store: &Store,
    state_store: &StateStore,
    http: &reqwest::Client,
    service_url: &str,
    access_token: &str,
    entry: &SyncJournalEntry,
) -> Result<()> {
    let prepared = match store.sync_upload_envelope(entry.id)? {
        Some(prepared) => prepared,
        None => prepare_envelope(store, state_store, entry)?,
    };
    let content_hash: [u8; 32] = entry
        .content_hash
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("sync journal content hash has the wrong length"))?;
    let start: StartUploadResponse = send_json(
        http.post(format!("{service_url}/v1/sync/uploads"))
            .bearer_auth(access_token)
            .header("Idempotency-Key", sync_idempotency_key("start", entry))
            .json(&StartUploadRequest {
                record_kind: &entry.record_kind,
                stable_id: &entry.stable_id,
                revision: entry.revision,
                logical_clock: entry.logical_clock,
                operation: &entry.operation,
                base_hash: entry.base_hash.map(|hash| URL_SAFE_NO_PAD.encode(hash)),
                content_hash: URL_SAFE_NO_PAD.encode(content_hash),
                ciphertext_bytes: prepared.envelope.len(),
                ciphertext_sha256: URL_SAFE_NO_PAD.encode(prepared.ciphertext_sha256),
            }),
    )
    .await
    .context("reserve encrypted sync upload")?;
    if !start.already_complete {
        let upload_url = start
            .upload_url
            .as_deref()
            .context("sync upload response omitted upload_url")?;
        let mut request = http.put(upload_url).body(prepared.envelope.clone());
        for (name, value) in &start.required_headers {
            request = request.header(
                HeaderName::from_bytes(name.as_bytes()).context("invalid upload header name")?,
                HeaderValue::from_str(value).context("invalid upload header value")?,
            );
        }
        request
            .send()
            .await
            .context("upload encrypted sync object")?
            .error_for_status()
            .context("R2 rejected encrypted sync upload")?;
        let _: CompleteUploadResponse = send_json(
            http.post(format!(
                "{service_url}/v1/sync/uploads/{}/complete",
                start.upload_id
            ))
            .bearer_auth(access_token)
            .header("Idempotency-Key", sync_idempotency_key("complete", entry)),
        )
        .await
        .context("complete encrypted sync upload")?;
    }
    store.mark_sync_journal_uploaded(&[entry.id], (now_ms() / 1_000) as i64)?;
    Ok(())
}

fn prepare_envelope(
    store: &Store,
    state_store: &StateStore,
    entry: &SyncJournalEntry,
) -> Result<SyncUploadEnvelope> {
    let kind = record_kind(&entry.record_kind)?;
    let operation = match entry.operation.as_str() {
        "upsert" => SyncOperation::Upsert,
        "tombstone" => SyncOperation::Tombstone,
        other => bail!("unsupported sync journal operation {other}"),
    };
    let content_hash: [u8; 32] = entry
        .content_hash
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("sync journal content hash has the wrong length"))?;
    let (state, sequence) = state_store.reserve_sequences(1)?;
    let identity = sync_identity(&state)?;
    let record = SyncRecord {
        stable_id: entry.stable_id.clone(),
        kind,
        revision: entry.revision,
        logical_clock: entry.logical_clock,
        device_id: identity.device_id,
        operation,
        base_hash: entry.base_hash,
        content_hash,
        payload: entry.payload.clone(),
    };
    let plaintext = serde_json::to_vec(&record).context("serialize sync record")?;
    let envelope = Envelope::seal(
        EnvelopeMetadata {
            kind: EnvelopeKind::SyncRecord,
            flags: 0,
            account_id: identity.account_id,
            sender_device_id: identity.device_id,
            recipient_kind: RecipientKind::Account,
            recipient_id: identity.account_id,
            key_epoch: identity.key_epoch,
            sequence,
            created_at_ms: now_ms(),
            nonce: rand::random::<[u8; 24]>(),
        },
        &plaintext,
        &identity.data_key,
        &identity.signing_key,
    )?
    .encode()?;
    let ciphertext_sha256: [u8; 32] = Sha256::digest(&envelope).into();
    Ok(store.store_sync_upload_envelope(entry.id, &envelope, ciphertext_sha256)?)
}

fn sync_identity(state: &LocalState) -> Result<SyncIdentity> {
    Ok(SyncIdentity {
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
        data_key: decode_base64_array(
            state
                .account_data_key
                .as_deref()
                .context("Anywhere account data key is missing")?,
            "account data key",
        )?,
        key_epoch: state.key_epoch.context("Anywhere key epoch is missing")?,
        signing_key: SigningKey::from_bytes(&decode_base64_array(
            state
                .signing_private_key
                .as_deref()
                .context("Anywhere signing key is missing")?,
            "signing key",
        )?),
    })
}

fn record_kind(kind: &str) -> Result<SyncRecordKind> {
    Ok(match kind {
        "session" => SyncRecordKind::Session,
        "message" => SyncRecordKind::Message,
        "checkpoint" => SyncRecordKind::Checkpoint,
        "tool_call" => SyncRecordKind::ToolCall,
        "routing_decision" => SyncRecordKind::RoutingDecision,
        "usage" => SyncRecordKind::Usage,
        "compaction" => SyncRecordKind::Compaction,
        "memory" => SyncRecordKind::Memory,
        "user_setting" => SyncRecordKind::UserSetting,
        "command" => SyncRecordKind::Command,
        "skill" => SyncRecordKind::Skill,
        "agent" => SyncRecordKind::Agent,
        "workflow" => SyncRecordKind::Workflow,
        "file" => SyncRecordKind::File,
        other => bail!("record kind {other} is not eligible for Anywhere sync"),
    })
}

fn sync_idempotency_key(scope: &str, entry: &SyncJournalEntry) -> String {
    let input = format!(
        "{scope}:{}:{}:{}",
        entry.record_kind, entry.stable_id, entry.revision
    );
    hex::encode(Sha256::digest(input.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_store::SyncJournalOperation;

    #[test]
    fn dependency_failures_back_off_without_abandoning_the_durable_journal() {
        assert_eq!(next_retry_delay(SYNC_INTERVAL), Duration::from_secs(20));
        assert_eq!(
            next_retry_delay(Duration::from_secs(20)),
            Duration::from_secs(40)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(160)),
            Duration::from_secs(300)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(300)),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn pending_snapshot_is_encrypted_once_and_cached_durably() {
        let store = Store::open_in_memory().expect("open store");
        store
            .append_sync_file_journal(
                "commands/review.md",
                SyncJournalOperation::Upsert,
                1,
                4,
                Some([0x55; 32]),
                br#"{"id":"message-1","content":"plaintext secret"}"#,
            )
            .expect("append journal");
        let entry = store
            .pending_sync_journal(1)
            .expect("pending journal")
            .remove(0);
        let temp = tempfile::tempdir().expect("temp dir");
        let state_store = StateStore {
            path: temp.path().join("anywhere/state.json"),
        };
        let signing_seed = [0x33; 32];
        let data_key = [0x44; 32];
        state_store
            .save(&LocalState {
                version: super::super::STATE_VERSION,
                account_id: Some(hex::encode([0x11; 16])),
                device_id: Some(hex::encode([0x22; 16])),
                signing_private_key: Some(URL_SAFE_NO_PAD.encode(signing_seed)),
                account_data_key: Some(URL_SAFE_NO_PAD.encode(data_key)),
                key_epoch: Some(2),
                ..LocalState::default()
            })
            .expect("save state");

        let prepared = prepare_envelope(&store, &state_store, &entry).expect("prepare envelope");
        assert_eq!(state_store.load().expect("load state").next_sequence, 1);
        assert!(!prepared
            .envelope
            .windows(b"plaintext secret".len())
            .any(|window| window == b"plaintext secret"));
        let envelope = Envelope::decode(&prepared.envelope).expect("decode envelope");
        assert_eq!(envelope.metadata.kind, EnvelopeKind::SyncRecord);
        assert_eq!(envelope.metadata.recipient_kind, RecipientKind::Account);
        let plaintext = envelope
            .open(
                &data_key,
                &SigningKey::from_bytes(&signing_seed).verifying_key(),
            )
            .expect("open envelope");
        let record: SyncRecord = serde_json::from_slice(&plaintext).expect("decode sync record");
        assert_eq!(record.stable_id, "commands/review.md");
        assert_eq!(record.base_hash, Some([0x55; 32]));
        assert_eq!(record.payload, entry.payload);
        assert_eq!(
            store
                .sync_upload_envelope(entry.id)
                .expect("load cached envelope"),
            Some(prepared)
        );
    }
}
