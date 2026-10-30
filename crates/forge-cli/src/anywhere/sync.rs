//! Durable encrypted upload worker for the local Forge sync journal.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use forge_anywhere_protocol::sync::{
    SyncOperation, SyncRecord, SyncRecordKind, MAX_SYNC_UPLOAD_ENVELOPE_BYTES,
};
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
/// How long one cycle may keep draining a single direction (uploads, downloads, staged applies)
/// before yielding. One batch per cycle could not keep up: a host with an active session journals
/// ~14 records a minute while a cycle (10 uploads + 100 sequential downloads) took about a minute,
/// so its 9,000-entry backlog only grew, and a device joining late needed ~35 h to read the feed.
const DRAIN_BUDGET: Duration = Duration::from_secs(20);
/// Parallel object GETs per feed page. Staging still happens in feed order.
const DOWNLOAD_CONCURRENCY: usize = 8;

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
            super::account_sync::wait_for_sync_request(wait_before_next).await;
            let outcome = upload_batch().await.map_err(|error| format!("{error:#}"));
            if let Some(line) = sync_report(&last_error, outcome.as_ref().err(), retry_delay) {
                eprintln!("{line}");
            }
            match outcome {
                Ok(()) => {
                    last_error.clear();
                    retry_delay = SYNC_INTERVAL;
                    wait_before_next = SYNC_INTERVAL;
                }
                Err(message) => {
                    last_error = message;
                    wait_before_next = retry_delay;
                    retry_delay = next_retry_delay(retry_delay);
                }
            }
        }
    })
}

/// The line to print for this cycle, or `None` to stay quiet.
///
/// Only TRANSITIONS are worth printing: a repeated identical error would spam the log every cycle.
/// But recovery must be announced too — without it the last Anywhere line in the log says the sync
/// is failing long after it is healthy again, which reads as a dead sync to anyone checking. That
/// is not hypothetical: a transient `database is locked` at startup left "sync paused" as the final
/// log line while the loop quietly recovered on its next attempt.
fn sync_report(
    previous_error: &str,
    error: Option<&String>,
    retry_delay: Duration,
) -> Option<String> {
    match error {
        // "paused" overstated it — this loop always retries. Name the delay so a reader can tell a
        // retrying sync from a stopped one.
        Some(message) if message.as_str() != previous_error => Some(format!(
            "⚠ Forge Anywhere sync failing: {message} — retrying in {}s",
            retry_delay.as_secs().max(1)
        )),
        Some(_) => None,
        None if !previous_error.is_empty() => Some("⚒ Forge Anywhere sync recovered".to_string()),
        None => None,
    }
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
    if let Err(error) = state_store.migrate_keys_to_secret_store() {
        tracing::warn!("could not move Anywhere keys into the secret store: {error:#}");
    }
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
    let http = client()?;
    let started = std::time::Instant::now();
    // An upload failure (a revision conflict, say) must not starve the download side: remote
    // changes keep arriving and the failure is reported once the rest of this pass is done.
    let upload_result: Result<()> = async {
        loop {
            let entries = store.pending_sync_journal(BATCH_SIZE)?;
            if entries.is_empty() {
                break;
            }
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
            if started.elapsed() >= DRAIN_BUDGET {
                break;
            }
        }
        Ok(())
    }
    .await;
    let started = std::time::Instant::now();
    while download_changes(&store, &state_store, &http, &service_url, &access_token).await? {
        if started.elapsed() >= DRAIN_BUDGET {
            break;
        }
    }
    let mut conflicts = 0;
    // A feed page may contain children before a parent from an earlier delayed upload. Keep
    // applying local-only passes while they make progress so newly satisfied dependencies
    // materialize now, while genuinely missing parents remain durably staged for a future poll.
    let started = std::time::Instant::now();
    while started.elapsed() < DRAIN_BUDGET {
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
    // Account sync reads what the passes above materialized and journals what changed locally;
    // the extra drain gets those records out now instead of a cycle later.
    let account_sync_config = config.anywhere.account_sync.clone();
    if !account_sync_config.any_enabled() {
        forge_config::account_sync::clear_sync_badge();
    } else {
        match super::account_sync::run_cycle(&store, &state, &account_sync_config).await {
            Ok(()) => {
                let started = std::time::Instant::now();
                while let Ok(entries) = store.pending_sync_journal(BATCH_SIZE) {
                    if entries.is_empty() || started.elapsed() >= DRAIN_BUDGET {
                        break;
                    }
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
                }
            }
            Err(error) => super::account_sync::record_cycle(Some(format!("{error:#}"))),
        }
    }
    // Acknowledged outbox revisions and already-materialized downloads otherwise retain duplicate
    // payloads forever. Keep this bounded so a long-lived daemon amortizes old cleanup without a
    // latency or write-lock spike; conflicts and each local record's newest revision are retained.
    store.prune_terminal_sync_rows(1_000)?;
    upload_result
}

/// Reads and stages one feed page. Returns whether the page held any changes, so the caller can
/// keep paging while the feed has more.
async fn download_changes(
    store: &Store,
    state_store: &StateStore,
    http: &reqwest::Client,
    service_url: &str,
    access_token: &str,
) -> Result<bool> {
    use futures::StreamExt as _;
    let cursor = store.sync_download_cursor()?;
    let feed: ChangeFeed = send_json(
        http.get(format!(
            "{service_url}/v1/sync/changes?cursor={cursor}&limit=100"
        ))
        .bearer_auth(access_token),
    )
    .await
    .context("load encrypted sync changes")?;
    let had_changes = !feed.changes.is_empty();
    let mut missing = 0usize;
    let mut fetched = futures::stream::iter(feed.changes)
        .map(|change| async move {
            let object = fetch_object(http, &change).await;
            (change, object)
        })
        .buffered(DOWNLOAD_CONCURRENCY);
    while let Some((change, object)) = fetched.next().await {
        match object? {
            Some(bytes) => stage_change(store, state_store, change, &bytes)?,
            None => missing += 1,
        }
    }
    if missing > 0 {
        eprintln!(
            "⚠ Forge Anywhere skipped {missing} sync change(s) whose encrypted object no longer \
             exists in storage"
        );
    }
    store.advance_sync_download_cursor(feed.next_cursor)?;
    Ok(had_changes)
}

/// A presigned GET that answers 404/410 names an object storage no longer holds, so no retry can
/// ever fetch it. Failing the pass on it pinned the feed cursor there for good: a device that
/// joined later (cursor 0) replayed into an early change whose object was gone and never got past
/// it, and the failure backoff starved its own uploads to ten entries per five minutes. Other
/// statuses (an expired signature, a 5xx) stay errors because a later poll can succeed.
fn object_permanently_missing(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE
}

/// The change's encrypted object, or `None` when storage no longer holds it.
async fn fetch_object(http: &reqwest::Client, change: &Change) -> Result<Option<Vec<u8>>> {
    if change.ciphertext_bytes > MAX_SYNC_UPLOAD_ENVELOPE_BYTES {
        bail!(
            "encrypted sync download exceeds {} MiB",
            MAX_SYNC_UPLOAD_ENVELOPE_BYTES / (1024 * 1024)
        );
    }
    let response = http
        .get(&change.download_url)
        .send()
        .await
        .context("download encrypted sync object")?;
    if object_permanently_missing(response.status()) {
        tracing::warn!(
            cursor = change.cursor,
            record_kind = %change.record_kind,
            revision = change.revision,
            "Anywhere sync object is missing from storage; skipping this change"
        );
        return Ok(None);
    }
    let bytes = response
        .error_for_status()
        .context("R2 rejected encrypted sync download")?
        .bytes()
        .await
        .context("read encrypted sync object")?;
    Ok(Some(bytes.to_vec()))
}

fn stage_change(
    store: &Store,
    state_store: &StateStore,
    change: Change,
    bytes: &[u8],
) -> Result<()> {
    if bytes.len() as u64 != change.ciphertext_bytes {
        bail!("encrypted sync download length does not match change metadata");
    }
    let expected_ciphertext_hash =
        decode_base64_array::<32>(&change.ciphertext_sha256, "sync ciphertext hash")?;
    if Sha256::digest(bytes).as_slice() != expected_ciphertext_hash {
        bail!("encrypted sync download hash does not match change metadata");
    }
    let envelope = Envelope::decode(bytes)?;
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
    // Rows journaled by an older build for a kind the service does not accept (continual-harness
    // learnings) can never upload; leaving them first in the strict-order outbox blocked everything
    // behind them forever.
    if record_kind(&entry.record_kind).is_err() {
        return skip_ineligible(store, entry);
    }
    let prepared = match store.sync_upload_envelope(entry.id)? {
        Some(prepared) => prepared,
        None => prepare_envelope(store, state_store, entry)?,
    };
    // The service refuses an oversized object with a bare HTTP 400, and this outbox drains strictly
    // in `id` order and aborts the whole cycle on the first error — so retrying one such record
    // forever blocks every later one. Checked here rather than only on fresh envelopes so a cached
    // envelope sealed before this cap existed is skipped too.
    if envelope_exceeds_cap(prepared.envelope.len()) {
        return skip_oversized(store, entry, prepared.envelope.len());
    }
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
    // Never persist an envelope the service will refuse: the staging row would keep a multi-megabyte
    // blob alive for a record that is about to be skipped anyway. The caller does the skipping, so
    // that one decision lives in one place.
    if envelope_exceeds_cap(envelope.len()) {
        return Ok(SyncUploadEnvelope {
            envelope,
            ciphertext_sha256,
        });
    }
    Ok(store.store_sync_upload_envelope(entry.id, &envelope, ciphertext_sha256)?)
}

fn envelope_exceeds_cap(envelope_len: usize) -> bool {
    envelope_len as u64 > MAX_SYNC_UPLOAD_ENVELOPE_BYTES
}

/// Give up on a record the service can never accept, so the strictly-ordered outbox keeps moving.
///
/// Marking it uploaded is the only way to advance: `uploaded_at IS NULL` *is* the outbox cursor.
/// The record genuinely never reaches the other devices, which is the deliberate trade — losing one
/// oversized record beats blocking every record behind it, which is what used to happen.
fn skip_oversized(store: &Store, entry: &SyncJournalEntry, envelope_len: usize) -> Result<()> {
    eprintln!(
        "⚠ Forge Anywhere skipped {} {} — its encrypted envelope is {:.1} MiB, over the {} MiB \
         limit. That record will not reach your other devices; everything after it still syncs.",
        entry.record_kind,
        entry.stable_id,
        envelope_len as f64 / (1024.0 * 1024.0),
        MAX_SYNC_UPLOAD_ENVELOPE_BYTES / (1024 * 1024)
    );
    store.mark_sync_journal_uploaded(&[entry.id], (now_ms() / 1_000) as i64)?;
    Ok(())
}

fn skip_ineligible(store: &Store, entry: &SyncJournalEntry) -> Result<()> {
    tracing::warn!(
        record_kind = %entry.record_kind,
        "skipping a sync journal row whose kind cannot be uploaded"
    );
    store.mark_sync_journal_uploaded(&[entry.id], (now_ms() / 1_000) as i64)?;
    Ok(())
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
    #[test]
    fn only_a_missing_object_is_skipped_rather_than_retried() {
        use reqwest::StatusCode;
        assert!(super::object_permanently_missing(StatusCode::NOT_FOUND));
        assert!(super::object_permanently_missing(StatusCode::GONE));
        assert!(!super::object_permanently_missing(StatusCode::FORBIDDEN));
        assert!(!super::object_permanently_missing(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(!super::object_permanently_missing(StatusCode::OK));
    }

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

    /// The defect this fixes: sync was the only Anywhere transport with no size cap. A 14.9 MB tool
    /// result sealed into a ~51 MiB envelope, the service answered HTTP 400, and because the outbox
    /// drains strictly in `id` order and aborts the cycle on the first error, that one record
    /// blocked 11,225 later ones for three days — the phone silently showed stale sessions.
    #[test]
    fn an_oversized_record_is_skipped_so_the_outbox_keeps_moving() {
        let store = Store::open_in_memory().expect("open store");
        for path in ["huge.md", "later.md"] {
            store
                .append_sync_file_journal(
                    path,
                    SyncJournalOperation::Upsert,
                    1,
                    4,
                    Some([0x55; 32]),
                    b"{}",
                )
                .expect("append journal");
        }
        let poison = store.pending_sync_journal(1).expect("pending").remove(0);
        assert_eq!(poison.stable_id, "huge.md");

        skip_oversized(&store, &poison, MAX_SYNC_UPLOAD_ENVELOPE_BYTES as usize + 1)
            .expect("skip the oversized record");

        let remaining: Vec<String> = store
            .pending_sync_journal(10)
            .expect("pending")
            .into_iter()
            .map(|entry| entry.stable_id)
            .collect();
        assert_eq!(
            remaining,
            vec!["later.md".to_string()],
            "the record behind the skipped one must become reachable"
        );
    }

    /// The wire path account sync depends on: a portable record journaled on one device is sealed
    /// by the uploader and verified, opened and staged by the downloader on another.
    #[test]
    fn a_portable_record_survives_seal_upload_and_stage_between_two_stores() {
        let a = Store::open_in_memory().expect("store a");
        let b = Store::open_in_memory().expect("store b");
        a.set_sync_journal_enabled(true).expect("journal on");
        let device = [0x22; 16];
        a.write_portable_sync_record(
            device,
            "user_setting",
            &"ab".repeat(32),
            Some(b"{\"schema\":\"x\"}"),
        )
        .expect("write portable");
        let entry = a.pending_sync_journal(1).expect("pending").remove(0);
        let temp = tempfile::tempdir().expect("temp dir");
        let state_store = StateStore {
            path: temp.path().join("anywhere/state.json"),
        };
        state_store
            .save(&LocalState {
                version: super::super::STATE_VERSION,
                account_id: Some(hex::encode([0x11; 16])),
                device_id: Some(hex::encode(device)),
                signing_private_key: Some(URL_SAFE_NO_PAD.encode([0x33; 32])),
                account_data_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
                key_epoch: Some(1),
                ..LocalState::default()
            })
            .expect("save state");
        let prepared = prepare_envelope(&a, &state_store, &entry).expect("seal");
        let change = Change {
            cursor: 1,
            device_id: hex::encode(device),
            signing_public_key: URL_SAFE_NO_PAD.encode(
                SigningKey::from_bytes(&[0x33; 32])
                    .verifying_key()
                    .to_bytes(),
            ),
            record_kind: entry.record_kind.clone(),
            stable_id: entry.stable_id.clone(),
            revision: entry.revision,
            logical_clock: entry.logical_clock,
            operation: entry.operation.clone(),
            base_hash: None,
            content_hash: URL_SAFE_NO_PAD.encode(entry.content_hash.clone()),
            ciphertext_bytes: prepared.envelope.len() as u64,
            ciphertext_sha256: URL_SAFE_NO_PAD.encode(prepared.ciphertext_sha256),
            download_url: String::new(),
        };
        stage_change(&b, &state_store, change, &prepared.envelope).expect("stage");
        b.apply_staged_portable_records([0x99; 16], 10)
            .expect("apply");
        let landed = b
            .portable_sync_record("user_setting", &entry.stable_id)
            .expect("read")
            .expect("record landed");
        assert_eq!(landed.payload, b"{\"schema\":\"x\"}");
    }

    #[test]
    fn harness_kinds_are_not_uploadable_so_legacy_rows_are_skipped() {
        assert!(record_kind("harness_entry").is_err());
        assert!(record_kind("harness_refinement").is_err());
        assert!(record_kind("user_setting").is_ok());
    }

    #[test]
    fn the_cap_rejects_only_above_the_limit() {
        assert!(!envelope_exceeds_cap(
            MAX_SYNC_UPLOAD_ENVELOPE_BYTES as usize
        ));
        assert!(envelope_exceeds_cap(
            MAX_SYNC_UPLOAD_ENVELOPE_BYTES as usize + 1
        ));
    }
}

#[cfg(test)]
mod sync_report_tests {
    use super::*;

    /// The defect this fixes: a transient failure printed one line, the loop recovered on its next
    /// attempt, and nothing said so — leaving "sync failing" as the last word in the log while the
    /// sync was healthy. An operator reading that log concludes the sync is dead. It cost a
    /// needless daemon restart to "fix" something that had already fixed itself.
    #[test]
    fn recovery_is_announced_so_the_log_does_not_end_on_a_failure() {
        let locked = "database is locked".to_string();

        let first = sync_report("", Some(&locked), SYNC_INTERVAL).expect("a new error is reported");
        assert!(first.contains("database is locked"));
        assert!(
            first.contains("retrying"),
            "the line must say it will retry, not that it stopped: {first}"
        );

        assert_eq!(
            sync_report(&locked, Some(&locked), SYNC_INTERVAL),
            None,
            "the same error every 10s would spam the log"
        );

        assert_eq!(
            sync_report(&locked, None, SYNC_INTERVAL),
            Some("⚒ Forge Anywhere sync recovered".to_string()),
            "recovery after an error MUST be announced"
        );
    }

    #[test]
    fn a_healthy_sync_stays_silent() {
        assert_eq!(sync_report("", None, SYNC_INTERVAL), None);
    }

    /// A different error while already failing is real news — it means the cause changed.
    #[test]
    fn a_changed_error_is_reported_with_the_current_backoff() {
        let line = sync_report(
            "database is locked",
            Some(&"connection refused".to_string()),
            Duration::from_secs(80),
        )
        .expect("a changed error is reported");
        assert!(line.contains("connection refused"));
        assert!(
            line.contains("80s"),
            "the actual backoff must be named: {line}"
        );
    }
}
