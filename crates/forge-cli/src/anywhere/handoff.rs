//! Safe source-side workspace handoff orchestration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use forge_anywhere_protocol::{
    CapsuleAcknowledgeRequest, CapsuleAcknowledgement, CapsuleClaim, CapsuleCompletion, CapsuleId,
    CapsuleOutcome, CapsuleReservation, CapsuleReserveRequest, CapsuleStatus, Envelope,
    EnvelopeKind, EnvelopeMetadata, PendingCapsule, PendingCapsuleList, RecipientKind,
    CAPSULE_FLAG_ACCEPTED, CAPSULE_VERSION, MAX_CAPSULE_ENVELOPE_BYTES,
};
use futures::StreamExt as _;
use reqwest::header::{HeaderName, HeaderValue};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use super::{
    client, decode_base64_array, decode_hex_array, ensure_access_token, now_ms,
    refresh_account_epoch, send_json, CapsuleJournalEntry, DeviceList, LocalState,
    OutgoingHandoffEntry, StateStore,
};

const STATUS_WAIT: Duration = Duration::from_secs(120);
const STATUS_POLL: Duration = Duration::from_secs(2);
const DESTINATION_POLL: Duration = Duration::from_secs(5);
const CAPSULE_JOURNAL_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Debug, Deserialize)]
struct HostList {
    hosts: Vec<HostRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct HostRow {
    id: String,
    name: String,
    #[serde(default)]
    revoked_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalSession {
    id: String,
    cwd: String,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    waiting: bool,
}

pub(super) async fn create(session: &str, destination: &str) -> Result<()> {
    let state_store = StateStore::platform()?;
    let mut state = state_store.load()?;
    let access_token = ensure_access_token(&state_store, &mut state).await?;
    let config = forge_config::load()?;
    let service_url = config.anywhere.service_url().to_owned();
    refresh_account_epoch(&state_store, &mut state, &service_url, &access_token).await?;
    recover_abandoned_pre_network(&state_store, &mut state, session)?;
    if let Some((source_session_id, operation)) = pending_outgoing(&state, session)? {
        return resume_outgoing_handoff(
            &state_store,
            &service_url,
            &access_token,
            &source_session_id,
            &operation,
            &identity(&state)?,
        )
        .await;
    }
    let local = require_idle_local_session(session).await?;
    let store = crate::open_store()?;
    let checkpoints = store
        .list_checkpoints(&local.id)
        .context("load handoff checkpoints")?;
    if checkpoints.is_empty() {
        bail!(
            "session {} has no completed checkpoint; finish one turn before handing it off",
            local.id
        );
    }
    let hosts: HostList = send_json(
        client()?
            .get(format!("{service_url}/v1/hosts"))
            .bearer_auth(&access_token),
    )
    .await
    .context("list Anywhere hosts")?;
    let destination = resolve_host(&hosts.hosts, destination)?;
    let source_host_id = state
        .host_id
        .clone()
        .context("this host is not registered; run `forge anywhere enable`")?;
    if destination.id == source_host_id {
        bail!("destination host is this host; choose another active host");
    }
    let identity = identity(&state)?;
    let capsule_id = CapsuleId::new(rand::random());
    // The durable split-brain guard is installed before stopping the driver and before reading a
    // single export byte. Direct/LAN sockets consult the same store row, so none can mutate the
    // captured session after this point. No service operation has happened yet, making rollback
    // safe if local preparation fails.
    state_store.update(|latest| {
        latest
            .preparing_handoffs
            .insert(local.id.clone(), capsule_id.to_string());
        Ok(())
    })?;
    if let Err(error) = store.begin_source_handoff(&local.id, &capsule_id.to_string()) {
        state_store.update(|latest| {
            latest.preparing_handoffs.remove(&local.id);
            Ok(())
        })?;
        return Err(error).context("freeze source session for handoff");
    }
    let prepared = async {
        archive_source_session(&local.id).await?;
        let stopped_checkpoints = store
            .list_checkpoints(&local.id)
            .context("recheck handoff checkpoint after stopping source driver")?;
        if stopped_checkpoints.is_empty() {
            bail!("source session lost its completed checkpoint while stopping the driver");
        }

        let exported_session = store
            .export_handoff_session(&local.id)
            .context("export portable session after source driver stopped")?;
        let session_json =
            serde_json::to_vec(&exported_session).context("encode portable session")?;
        let repository = repository_root(Path::new(&local.cwd))?;
        let temporary = tempfile::tempdir().context("create handoff staging directory")?;
        let archive_path = temporary.path().join("workspace.forge-capsule");
        let exported = forge_core::capsule::export_capsule(
            &repository,
            &archive_path,
            &local.id,
            &session_json,
            forge_core::capsule::CapsuleLimits::default(),
        )
        .map_err(|error| anyhow::anyhow!(format_capsule_error(error)))?;
        let plaintext = std::fs::read(&exported.path).context("read staged handoff capsule")?;
        let sequence = allocate_sequence(&state_store)?;
        let destination_id = decode_hex_array::<16>(&destination.id, "destination host id")?;
        let envelope = Envelope::seal(
            EnvelopeMetadata {
                kind: EnvelopeKind::Capsule,
                flags: 0,
                account_id: identity.account_id,
                sender_device_id: identity.device_id,
                recipient_kind: RecipientKind::Host,
                recipient_id: destination_id,
                key_epoch: identity.key_epoch,
                sequence,
                created_at_ms: now_ms(),
                nonce: rand::random(),
            },
            &plaintext,
            &identity.data_key,
            &identity.signing_key,
        )?
        .encode()?;
        let ciphertext_bytes = u64::try_from(envelope.len()).context("capsule length overflow")?;
        if ciphertext_bytes > MAX_CAPSULE_ENVELOPE_BYTES {
            bail!("encrypted capsule exceeds the 100 MiB handoff limit");
        }
        let ciphertext_sha256 = URL_SAFE_NO_PAD.encode(Sha256::digest(&envelope));
        let request = CapsuleReserveRequest {
            version: CAPSULE_VERSION,
            capsule_id,
            source_session_id: local.id.clone(),
            source_host_id,
            destination_host_id: destination.id.clone(),
            ciphertext_bytes,
            ciphertext_sha256,
        };
        let envelope_path = persist_outgoing_envelope(&state_store, capsule_id, &envelope)?;
        let operation = OutgoingHandoffEntry {
            capsule_id: capsule_id.to_string(),
            destination_host_id: destination.id.clone(),
            destination_name: destination.name.clone(),
            envelope_path: envelope_path.to_string_lossy().into_owned(),
            request,
            reserve_idempotency_key: capsule_id.to_string(),
            complete_idempotency_key: format!("{capsule_id}-complete"),
            cancel_idempotency_key: format!("{capsule_id}-cancel"),
            accepted_destination_session_id: None,
            created_at_ms: now_ms(),
        };
        if let Err(error) = state_store.update(|state| {
            state
                .outgoing_handoffs
                .insert(local.id.clone(), operation.clone());
            state.preparing_handoffs.remove(&local.id);
            Ok(())
        }) {
            let _ = std::fs::remove_file(&operation.envelope_path);
            return Err(error).context("persist outgoing handoff operation");
        }
        Ok::<OutgoingHandoffEntry, anyhow::Error>(operation)
    }
    .await;
    let operation = match prepared {
        Ok(operation) => operation,
        Err(error) => {
            rollback_pre_network_handoff(&state_store, &store, &local.id, &capsule_id)
                .context("local handoff preparation failed and rollback could not complete")?;
            return Err(error.context(
                "local handoff preparation failed before service reservation; source was unfrozen",
            ));
        }
    };
    resume_outgoing_handoff(
        &state_store,
        &service_url,
        &access_token,
        &local.id,
        &operation,
        &identity,
    )
    .await
}

fn recover_abandoned_pre_network(
    state_store: &StateStore,
    state: &mut LocalState,
    needle: &str,
) -> Result<()> {
    let matches = state
        .preparing_handoffs
        .iter()
        .filter(|(session_id, _)| *session_id == needle || session_id.starts_with(needle))
        .map(|(session_id, capsule_id)| (session_id.clone(), capsule_id.clone()))
        .collect::<Vec<_>>();
    let [(session_id, capsule_id)] = matches.as_slice() else {
        if matches.is_empty() {
            return Ok(());
        }
        bail!("session prefix {needle:?} matches multiple preparing handoffs");
    };
    if state.outgoing_handoffs.contains_key(session_id) {
        return Ok(());
    }
    let store = crate::open_store()?;
    if !store
        .cancel_source_handoff(session_id, capsule_id)
        .context("recover abandoned pre-network handoff freeze")?
    {
        bail!("abandoned pre-network handoff freeze no longer matches local state");
    }
    *state = state_store.update(|latest| {
        latest.preparing_handoffs.remove(session_id);
        Ok(())
    })?;
    Ok(())
}

fn rollback_pre_network_handoff(
    state_store: &StateStore,
    store: &forge_store::Store,
    source_session_id: &str,
    capsule_id: &CapsuleId,
) -> Result<()> {
    if !store
        .cancel_source_handoff(source_session_id, &capsule_id.to_string())
        .context("remove pre-network source freeze")?
    {
        bail!("pre-network source freeze was no longer pending");
    }
    state_store.update(|state| {
        if state
            .outgoing_handoffs
            .get(source_session_id)
            .is_some_and(|entry| entry.capsule_id == capsule_id.to_string())
        {
            state.outgoing_handoffs.remove(source_session_id);
        }
        state.preparing_handoffs.remove(source_session_id);
        Ok(())
    })?;
    Ok(())
}

fn pending_outgoing(
    state: &LocalState,
    needle: &str,
) -> Result<Option<(String, OutgoingHandoffEntry)>> {
    let matches = state
        .outgoing_handoffs
        .iter()
        .filter(|(session_id, _)| *session_id == needle || session_id.starts_with(needle))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [(session_id, operation)] => Ok(Some(((*session_id).clone(), (*operation).clone()))),
        _ => bail!("session prefix {needle:?} matches multiple pending handoffs"),
    }
}

async fn resume_outgoing_handoff(
    state_store: &StateStore,
    service_url: &str,
    access_token: &str,
    source_session_id: &str,
    operation: &OutgoingHandoffEntry,
    identity: &Identity,
) -> Result<()> {
    let capsule_id: CapsuleId = operation
        .capsule_id
        .parse()
        .map_err(|error| anyhow::anyhow!("pending handoff has an invalid capsule id: {error}"))?;
    if let Some(destination_session) = operation.accepted_destination_session_id.as_deref() {
        crate::open_store()?
            .mark_source_handoff_transferred(source_session_id, &operation.capsule_id)
            .context("finish transferred-away source state")?;
        state_store.update(|state| {
            state.outgoing_handoffs.remove(source_session_id);
            Ok(())
        })?;
        let _ = std::fs::remove_file(&operation.envelope_path);
        println!("Handoff complete. Destination session: {destination_session}");
        return Ok(());
    }
    let envelope = std::fs::read(&operation.envelope_path)
        .context("read persisted encrypted handoff capsule")?;
    let bytes = u64::try_from(envelope.len()).context("capsule length overflow")?;
    let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(&envelope));
    if bytes != operation.request.ciphertext_bytes
        || hash != operation.request.ciphertext_sha256
        || operation.request.capsule_id != capsule_id
        || operation.request.source_session_id != source_session_id
    {
        bail!(
            "persisted handoff capsule does not match its durable operation; source remains frozen"
        );
    }
    let store = crate::open_store()?;
    store
        .begin_source_handoff(source_session_id, &operation.capsule_id)
        .context("restore source handoff freeze")?;
    // A retry may follow a crash between the durable freeze and stopping the driver.
    archive_source_session(source_session_id).await?;
    let reservation: CapsuleReservation = send_json(
        large_client()?
            .post(format!("{service_url}/v1/capsules"))
            .bearer_auth(access_token)
            .header("Idempotency-Key", &operation.reserve_idempotency_key)
            .json(&operation.request),
    )
    .await
    .context("reserve encrypted handoff capsule")?;
    validate_reservation(&reservation, &capsule_id)?;
    if !reservation.already_complete {
        upload_exact(&reservation, envelope).await?;
        let completion = CapsuleCompletion {
            version: CAPSULE_VERSION,
            ciphertext_bytes: bytes,
            ciphertext_sha256: hash,
        };
        let _: CapsuleStatus = send_json(
            large_client()?
                .post(format!("{service_url}/v1/capsules/{}/complete", capsule_id))
                .bearer_auth(access_token)
                .header("Idempotency-Key", &operation.complete_idempotency_key)
                .json(&completion),
        )
        .await
        .context("complete encrypted handoff upload")?;
    }

    println!(
        "Encrypted capsule {} is ready for '{}'; the source session is frozen until cancellation or an accepted destination acknowledgement.",
        capsule_id, operation.destination_name
    );
    let destination_session = wait_for_acknowledgement(
        state_store,
        service_url,
        access_token,
        source_session_id,
        operation,
        &capsule_id,
        identity,
    )
    .await?;
    state_store.update(|state| {
        let entry = state
            .outgoing_handoffs
            .get_mut(source_session_id)
            .context("outgoing handoff disappeared before transfer")?;
        entry.accepted_destination_session_id = Some(destination_session.clone());
        Ok(())
    })?;
    store
        .mark_source_handoff_transferred(source_session_id, &operation.capsule_id)
        .context("persist transferred-away source session")?;
    state_store.update(|state| {
        state.outgoing_handoffs.remove(source_session_id);
        Ok(())
    })?;
    let _ = std::fs::remove_file(&operation.envelope_path);
    println!("Handoff complete. Destination session: {destination_session}");
    Ok(())
}

struct Identity {
    account_id: [u8; 16],
    device_id: [u8; 16],
    data_key: [u8; 32],
    key_epoch: u32,
    signing_key: SigningKey,
    host_id: [u8; 16],
    data_key_epochs: BTreeMap<u32, [u8; 32]>,
}

fn identity(state: &LocalState) -> Result<Identity> {
    let key_epoch = state.key_epoch.context("Anywhere key epoch is missing")?;
    let data_key = state
        .data_key_epochs
        .get(&key_epoch)
        .or(state.account_data_key.as_ref())
        .context("Anywhere account data key is missing")?;
    let data_key_epochs = state
        .data_key_epochs
        .iter()
        .map(|(epoch, key)| Ok((*epoch, decode_base64_array(key, "account data key epoch")?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(Identity {
        account_id: decode_hex_array(
            state
                .account_id
                .as_deref()
                .context("account id is missing")?,
            "account id",
        )?,
        device_id: decode_hex_array(
            state.device_id.as_deref().context("device id is missing")?,
            "device id",
        )?,
        data_key: decode_base64_array(data_key, "account data key")?,
        key_epoch,
        signing_key: SigningKey::from_bytes(&decode_base64_array(
            state
                .signing_private_key
                .as_deref()
                .context("device signing key is missing")?,
            "device signing key",
        )?),
        host_id: decode_hex_array(
            state.host_id.as_deref().context("host id is missing")?,
            "host id",
        )?,
        data_key_epochs,
    })
}

fn allocate_sequence(store: &StateStore) -> Result<u64> {
    let mut allocated = 0;
    store.update(|state| {
        allocated = state.next_sequence.max(1);
        state.next_sequence = allocated
            .checked_add(1)
            .context("Anywhere sequence exhausted")?;
        Ok(())
    })?;
    Ok(allocated)
}

fn persist_outgoing_envelope(
    store: &StateStore,
    capsule_id: CapsuleId,
    envelope: &[u8],
) -> Result<PathBuf> {
    use std::io::Write as _;

    let root = store
        .path
        .parent()
        .context("Anywhere state path has no parent")?
        .join("outgoing");
    std::fs::create_dir_all(&root).context("create outgoing handoff directory")?;
    super::set_owner_directory_permissions(&root)?;
    let path = root.join(format!("{capsule_id}.fany"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .context("create durable outgoing handoff capsule")?;
    file.write_all(envelope)
        .context("persist exact outgoing handoff capsule")?;
    file.sync_all()
        .context("sync exact outgoing handoff capsule")?;
    super::set_owner_file_permissions(&path)?;
    super::sync_directory(&root).context("sync outgoing handoff directory")?;
    Ok(path)
}

async fn require_idle_local_session(needle: &str) -> Result<LocalSession> {
    let (base, token) = local_daemon()?;
    let response = client()?
        .get(format!("{base}/{token}/api/sessions"))
        .send()
        .await
        .with_context(|| format!("reach local forge serve at {base}"))?;
    if !response.status().is_success() {
        bail!(
            "local forge serve refused the handoff preflight ({})",
            response.status()
        );
    }
    let sessions: Vec<LocalSession> = response.json().await.context("decode local session list")?;
    let matches: Vec<LocalSession> = sessions
        .into_iter()
        .filter(|session| session.id == needle || session.id.starts_with(needle))
        .collect();
    let session = match matches.as_slice() {
        [session] => session,
        [] => bail!("no running local session matches {needle:?}"),
        _ => bail!("session prefix {needle:?} is ambiguous"),
    };
    if session.busy || session.waiting {
        bail!(
            "session {} is active or awaiting input; let the tool call finish or explicitly interrupt it before handoff",
            session.id
        );
    }
    Ok(session.clone())
}

fn local_daemon() -> Result<(String, String)> {
    let port = forge_config::load()?.remote.serve_port();
    Ok((
        format!("http://127.0.0.1:{port}"),
        crate::serve::daemon_token(false)?,
    ))
}

async fn archive_source_session(session_id: &str) -> Result<()> {
    let (base, token) = local_daemon()?;
    let response = client()?
        .post(format!("{base}/{token}/api/sessions/{session_id}/archive"))
        .send()
        .await
        .context("stop the source session for handoff")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // The durable store guard is already installed. A missing registry entry means no live
        // daemon driver remains to accept direct/LAN input.
        return Ok(());
    }
    if !response.status().is_success() {
        bail!(
            "the local source driver could not be frozen for handoff (HTTP {}); source remains blocked",
            response.status()
        );
    }
    Ok(())
}

fn resolve_host(hosts: &[HostRow], needle: &str) -> Result<HostRow> {
    let matches: Vec<&HostRow> = hosts
        .iter()
        .filter(|host| {
            host.revoked_at.is_none()
                && (host.id == needle || host.name.eq_ignore_ascii_case(needle))
        })
        .collect();
    match matches.as_slice() {
        [host] => Ok((*host).clone()),
        [] => bail!("no active Anywhere host matches {needle:?}"),
        _ => bail!("host name {needle:?} is ambiguous; use its host id"),
    }
}

fn repository_root(cwd: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("resolve repository root")?;
    if !output.status.success() {
        bail!("session workspace is not inside an available Git repository");
    }
    let root = String::from_utf8(output.stdout).context("repository path is not UTF-8")?;
    Ok(PathBuf::from(root.trim()))
}

fn format_capsule_error(error: forge_core::capsule::CapsuleError) -> String {
    match error {
        forge_core::capsule::CapsuleError::UnsafeFiles(paths) => {
            let mut message = String::from("handoff aborted; unsafe workspace paths:\n");
            for rejected in paths.0 {
                message.push_str(&format!("  - {}: {}\n", rejected.path, rejected.reason));
            }
            message
        }
        other => other.to_string(),
    }
}

fn validate_reservation(reservation: &CapsuleReservation, expected: &CapsuleId) -> Result<()> {
    if reservation.version != CAPSULE_VERSION || &reservation.capsule_id != expected {
        bail!("service returned a mismatched capsule reservation");
    }
    if !reservation.already_complete && reservation.upload_url.is_none() {
        bail!("service omitted the capsule upload URL");
    }
    Ok(())
}

async fn upload_exact(reservation: &CapsuleReservation, envelope: Vec<u8>) -> Result<()> {
    let url = reservation
        .upload_url
        .as_deref()
        .context("capsule upload URL is missing")?;
    let mut request = large_client()?.put(url).body(envelope);
    for (name, value) in &reservation.required_headers {
        request = request.header(
            HeaderName::from_bytes(name.as_bytes()).context("invalid upload header name")?,
            HeaderValue::from_str(value).context("invalid upload header value")?,
        );
    }
    let response = request.send().await.context("upload encrypted capsule")?;
    if !response.status().is_success() {
        bail!(
            "encrypted capsule upload failed with HTTP {}",
            response.status()
        );
    }
    Ok(())
}

async fn wait_for_acknowledgement(
    state_store: &StateStore,
    service_url: &str,
    token: &str,
    source_session_id: &str,
    operation: &OutgoingHandoffEntry,
    capsule_id: &CapsuleId,
    identity: &Identity,
) -> Result<String> {
    let started = tokio::time::Instant::now();
    while started.elapsed() < STATUS_WAIT {
        let status: CapsuleStatus = send_json(
            client()?
                .get(format!("{service_url}/v1/capsules/{capsule_id}"))
                .bearer_auth(token),
        )
        .await
        .context("read handoff status")?;
        if status.acknowledgement_envelope.is_some() {
            match open_acknowledgement(&status, capsule_id, identity)? {
                OpenedAcknowledgement::Accepted(session_id) => return Ok(session_id),
                OpenedAcknowledgement::Rejected(message) => {
                    finalize_local_cancellation(
                        state_store,
                        source_session_id,
                        operation,
                        capsule_id,
                    )?;
                    bail!("{message}. The source session is resumable.");
                }
            }
        }
        tokio::time::sleep(STATUS_POLL).await;
    }
    let response = client()?
        .delete(format!("{service_url}/v1/capsules/{capsule_id}"))
        .bearer_auth(token)
        .header("Idempotency-Key", &operation.cancel_idempotency_key)
        .send()
        .await;
    let Ok(response) = response else {
        bail!("handoff timed out and cancellation could not be confirmed; source remains frozen (capsule {capsule_id})");
    };
    if response.status().is_success() {
        let bytes = response
            .bytes()
            .await
            .context("read handoff cancellation result")?;
        if !bytes.is_empty() {
            let status: CapsuleStatus =
                serde_json::from_slice(&bytes).context("decode handoff cancellation result")?;
            if status.acknowledgement_envelope.is_some() {
                return match open_acknowledgement(&status, capsule_id, identity)? {
                    OpenedAcknowledgement::Accepted(session_id) => Ok(session_id),
                    OpenedAcknowledgement::Rejected(message) => {
                        finalize_local_cancellation(
                            state_store,
                            source_session_id,
                            operation,
                            capsule_id,
                        )?;
                        bail!("{message}. The source session is resumable.")
                    }
                };
            }
            if is_confirmed_cancellation(&status) {
                finalize_local_cancellation(state_store, source_session_id, operation, capsule_id)?;
                bail!("handoff timed out and was cancelled; the source session is resumable");
            }
        }
    }
    bail!("handoff timed out but the service did not prove whether cancellation or transfer won; source remains frozen (capsule {capsule_id})")
}

fn is_confirmed_cancellation(status: &CapsuleStatus) -> bool {
    status.acknowledgement_envelope.is_none()
        && matches!(status.state.as_str(), "cancelled" | "deleted")
}

enum OpenedAcknowledgement {
    Accepted(String),
    Rejected(String),
}

fn open_acknowledgement(
    status: &CapsuleStatus,
    capsule_id: &CapsuleId,
    identity: &Identity,
) -> Result<OpenedAcknowledgement> {
    let verifying_key = VerifyingKey::from_bytes(&decode_base64_array(
        status
            .acknowledgement_signing_public_key
            .as_deref()
            .context("handoff acknowledgement signing key is missing")?,
        "handoff acknowledgement signing key",
    )?)
    .context("handoff acknowledgement signing key is invalid")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(
            status
                .acknowledgement_envelope
                .as_deref()
                .context("handoff acknowledgement envelope is missing")?,
        )
        .context("decode encrypted handoff acknowledgement")?;
    let envelope = Envelope::decode(&bytes)?;
    if envelope.metadata.kind != EnvelopeKind::Capsule
        || envelope.metadata.account_id != identity.account_id
        || envelope.metadata.recipient_kind != RecipientKind::Device
        || envelope.metadata.recipient_id != identity.device_id
        || !matches!(envelope.metadata.flags, 0 | CAPSULE_FLAG_ACCEPTED)
    {
        bail!("handoff acknowledgement routing identity is invalid");
    }
    let acknowledgement_key = identity
        .data_key_epochs
        .get(&envelope.metadata.key_epoch)
        .context("handoff acknowledgement uses an unavailable key epoch")?;
    let plaintext = envelope.open(acknowledgement_key, &verifying_key)?;
    let acknowledgement: CapsuleAcknowledgement =
        serde_json::from_slice(&plaintext).context("decode handoff acknowledgement")?;
    if acknowledgement.capsule_id != *capsule_id {
        bail!("handoff acknowledgement capsule id is invalid");
    }
    let flag_accepted = envelope.metadata.flags & CAPSULE_FLAG_ACCEPTED != 0;
    if flag_accepted != matches!(acknowledgement.outcome, CapsuleOutcome::Accepted) {
        bail!("handoff acknowledgement outcome does not match its signed header");
    }
    match acknowledgement.outcome {
        CapsuleOutcome::Accepted => Ok(OpenedAcknowledgement::Accepted(
            acknowledgement
                .destination_session_id
                .context("accepted handoff acknowledgement omitted the destination session id")?,
        )),
        outcome => Ok(OpenedAcknowledgement::Rejected(format!(
            "destination rejected the handoff ({outcome:?}): {}",
            acknowledgement
                .detail
                .as_deref()
                .unwrap_or("no detail supplied")
        ))),
    }
}

fn finalize_local_cancellation(
    state_store: &StateStore,
    source_session_id: &str,
    operation: &OutgoingHandoffEntry,
    capsule_id: &CapsuleId,
) -> Result<()> {
    let store = crate::open_store()?;
    if !store
        .cancel_source_handoff(source_session_id, &capsule_id.to_string())
        .context("unfreeze cancelled source handoff")?
    {
        bail!("source handoff was no longer pending; source remains frozen");
    }
    state_store.update(|state| {
        state.outgoing_handoffs.remove(source_session_id);
        Ok(())
    })?;
    let _ = std::fs::remove_file(&operation.envelope_path);
    Ok(())
}

fn large_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10 * 60))
        .build()
        .context("build capsule HTTP client")
}

pub(super) async fn capsule_worker_loop() {
    let mut interval = tokio::time::interval(DESTINATION_POLL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_error = String::new();
    loop {
        interval.tick().await;
        match poll_destination_capsules().await {
            Ok(()) => last_error.clear(),
            Err(error) => {
                let message = format!("{error:#}");
                if message != last_error {
                    eprintln!(
                        "⚠ Forge Anywhere handoff receiver unavailable — local/direct Forge is unaffected: {message}"
                    );
                    last_error = message;
                }
            }
        }
    }
}

async fn poll_destination_capsules() -> Result<()> {
    let state_store = StateStore::platform()?;
    let mut state = state_store.load()?;
    let host_id = state.host_id.clone().context("host is not registered")?;
    let access_token = ensure_access_token(&state_store, &mut state).await?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    refresh_account_epoch(&state_store, &mut state, &service_url, &access_token).await?;
    let identity = identity(&state)?;
    let retryable = state_store
        .load()?
        .capsule_journal
        .into_iter()
        .filter(|(_, journal)| journal.acked_at_ms.is_none() && journal.terminal_at_ms.is_none())
        .collect::<Vec<_>>();
    for (capsule_id, journal) in retryable {
        let parsed = match capsule_id.parse() {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("⚠ invalid persisted handoff capsule id {capsule_id:?}: {error}");
                continue;
            }
        };
        if let Err(error) =
            submit_acknowledgement(&state_store, &service_url, &access_token, &parsed, &journal)
                .await
        {
            eprintln!("⚠ handoff acknowledgement retry failed for {capsule_id}: {error:#}");
        }
    }
    prune_capsule_journal(&state_store)?;
    let pending: PendingCapsuleList = send_json(
        client()?
            .get(format!("{service_url}/v1/capsules"))
            .query(&[
                ("destination_host_id", host_id.as_str()),
                ("state", "ready"),
            ])
            .bearer_auth(&access_token),
    )
    .await
    .context("list pending handoff capsules")?;
    if pending.version != CAPSULE_VERSION {
        bail!("service returned an unsupported capsule list version");
    }
    let devices: DeviceList = send_json(
        client()?
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(&access_token),
    )
    .await
    .context("load handoff sender keys")?;
    for capsule in pending.capsules {
        if let Some(journal) = state_store
            .load()?
            .capsule_journal
            .get(&capsule.capsule_id.to_string())
            .cloned()
        {
            if journal.acked_at_ms.is_none() && journal.terminal_at_ms.is_none() {
                if let Err(error) = submit_acknowledgement(
                    &state_store,
                    &service_url,
                    &access_token,
                    &capsule.capsule_id,
                    &journal,
                )
                .await
                {
                    eprintln!(
                        "⚠ handoff acknowledgement retry failed for {}: {error:#}",
                        capsule.capsule_id
                    );
                }
            }
            continue;
        }
        if let Err(error) = receive_capsule(
            &state_store,
            &service_url,
            &access_token,
            &identity,
            &devices,
            capsule.clone(),
        )
        .await
        {
            eprintln!(
                "⚠ handoff capsule {} could not be processed: {error:#}",
                capsule.capsule_id
            );
        }
    }
    Ok(())
}

async fn receive_capsule(
    state_store: &StateStore,
    service_url: &str,
    access_token: &str,
    identity: &Identity,
    devices: &DeviceList,
    pending: PendingCapsule,
) -> Result<()> {
    validate_pending(&pending)?;
    let source_device_id = decode_hex_array::<16>(&pending.source_device_id, "source device id")?;
    let verifying_key = devices
        .devices
        .iter()
        .find(|device| device.id == pending.source_device_id)
        .and_then(|device| device.signing_public_key.as_deref())
        .context("capsule sender is not an active signing device")?;
    let verifying_key = VerifyingKey::from_bytes(&decode_base64_array(
        verifying_key,
        "source signing public key",
    )?)
    .context("source signing public key is invalid")?;
    let claim: CapsuleClaim = send_json(
        large_client()?
            .post(format!(
                "{service_url}/v1/capsules/{}/claim",
                pending.capsule_id
            ))
            .bearer_auth(access_token)
            .header("Idempotency-Key", format!("{}-claim", pending.capsule_id)),
    )
    .await
    .context("claim encrypted handoff capsule")?;
    validate_claim(&pending, &claim)?;
    let bytes = download_capsule(&claim).await?;
    let envelope = Envelope::decode(&bytes).context("decode encrypted handoff capsule")?;
    if envelope.metadata.kind != EnvelopeKind::Capsule
        || envelope.metadata.flags != 0
        || envelope.metadata.account_id != identity.account_id
        || envelope.metadata.sender_device_id != source_device_id
        || envelope.metadata.recipient_kind != RecipientKind::Host
        || envelope.metadata.recipient_id != identity.host_id
        || envelope.metadata.key_epoch != pending.key_epoch
        || envelope.metadata.sequence != pending.sequence
    {
        bail!("capsule routing metadata does not match its claim");
    }
    let data_key = identity
        .data_key_epochs
        .get(&pending.key_epoch)
        .context("capsule uses an unavailable Account Data Key epoch")?;
    let plaintext = envelope
        .open(data_key, &verifying_key)
        .context("authenticate and decrypt handoff capsule")?;
    accept_capsule_replay(state_store, &pending)?;

    let import = import_destination_capsule(&pending, source_device_id, &plaintext);
    let (acknowledgement, imported) = match import {
        Ok(imported) => (
            CapsuleAcknowledgement {
                version: CAPSULE_VERSION,
                capsule_id: pending.capsule_id,
                outcome: CapsuleOutcome::Accepted,
                destination_session_id: Some(imported.session_id.clone()),
                detail: if imported.remapped {
                    Some("session id collided locally and was safely remapped".into())
                } else {
                    None
                },
            },
            Some(imported),
        ),
        Err(failure) => (
            CapsuleAcknowledgement {
                version: CAPSULE_VERSION,
                capsule_id: pending.capsule_id,
                outcome: failure.outcome,
                destination_session_id: None,
                detail: Some(failure.detail),
            },
            None,
        ),
    };
    let acknowledgement_envelope =
        seal_acknowledgement(state_store, identity, source_device_id, &acknowledgement)?;
    let journal = CapsuleJournalEntry {
        acknowledgement_envelope,
        idempotency_key: format!("{}-ack", pending.capsule_id),
        imported_session_id: imported.as_ref().map(|value| value.session_id.clone()),
        worktree_path: imported
            .as_ref()
            .map(|value| value.worktree_path.to_string_lossy().into_owned()),
        acked_at_ms: None,
        terminal_at_ms: None,
    };
    state_store.update(|state| {
        state
            .capsule_journal
            .insert(pending.capsule_id.to_string(), journal.clone());
        Ok(())
    })?;
    submit_acknowledgement(
        state_store,
        service_url,
        access_token,
        &pending.capsule_id,
        &journal,
    )
    .await?;
    if imported.is_some() {
        super::push::request_best_effort(
            &client()?,
            service_url,
            access_token,
            Some(&pending.source_device_id),
            super::push::GenericPushEvent::WorkspaceReady,
            &format!("capsule-{}-workspace-ready", pending.capsule_id),
        )
        .await;
    }
    Ok(())
}

fn prune_capsule_journal(store: &StateStore) -> Result<()> {
    let cutoff = now_ms().saturating_sub(CAPSULE_JOURNAL_RETENTION_MS);
    store.update(|state| {
        state.capsule_journal.retain(|_, journal| {
            journal
                .acked_at_ms
                .or(journal.terminal_at_ms)
                .is_none_or(|finished_at_ms| finished_at_ms >= cutoff)
        });
        let retained = state
            .capsule_journal
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        state
            .capsule_replay
            .retain(|_, capsule_id| retained.contains(capsule_id));
        Ok(())
    })?;
    Ok(())
}

struct ImportedDestination {
    session_id: String,
    worktree_path: PathBuf,
    remapped: bool,
}

struct ImportFailure {
    outcome: CapsuleOutcome,
    detail: String,
}

fn import_destination_capsule(
    pending: &PendingCapsule,
    source_device_id: [u8; 16],
    plaintext: &[u8],
) -> std::result::Result<ImportedDestination, ImportFailure> {
    let result = (|| -> Result<ImportedDestination> {
        let store = crate::open_store()?;
        if let Some(existing) = store
            .imported_session_by_capsule(&pending.capsule_id.to_string())
            .context("recover prior capsule import")?
        {
            return Ok(ImportedDestination {
                remapped: existing.session_id != existing.source_session_id,
                session_id: existing.session_id,
                worktree_path: PathBuf::from(existing.worktree_path),
            });
        }
        let temporary = tempfile::tempdir().context("create destination capsule staging")?;
        let capsule_path = temporary.path().join("received.forge-capsule");
        std::fs::write(&capsule_path, plaintext).context("stage decrypted capsule locally")?;
        let repository = std::env::current_dir().context("resolve destination repository")?;
        let worktree_root = forge_config::data_dir()
            .context("no Forge platform data directory is available")?
            .join("anywhere")
            .join("handoffs");
        std::fs::create_dir_all(&worktree_root).context("create handoff worktree directory")?;
        let worktree_path = worktree_root.join(pending.capsule_id.to_string());
        if worktree_path.exists() {
            // No provenance row exists (checked above), so this can only be an interrupted import.
            rollback_worktree(&repository, &worktree_path);
        }
        let imported = forge_core::capsule::import_capsule(
            &repository,
            &capsule_path,
            &worktree_path,
            forge_core::capsule::CapsuleLimits::default(),
        )?;
        let portable: forge_store::HandoffSessionExport =
            serde_json::from_slice(&imported.session_json).context("decode portable session")?;
        if portable.source_session_id != imported.manifest.session_id {
            rollback_worktree(&repository, &worktree_path);
            bail!("portable session id does not match capsule manifest");
        }
        let worktree = worktree_path.to_string_lossy().into_owned();
        let provenance = forge_store::HandoffImportProvenance {
            source_device_id,
            capsule_id: pending.capsule_id.to_string(),
            base_commit: imported.manifest.base_commit,
            imported_at: i64::try_from(now_ms() / 1000).unwrap_or(i64::MAX),
        };
        let session =
            match store.import_handoff_session_with_provenance(&portable, &worktree, &provenance) {
                Ok(session) => session,
                Err(error) => {
                    rollback_worktree(&repository, &worktree_path);
                    return Err(error.into());
                }
            };
        Ok(ImportedDestination {
            session_id: session.session_id,
            worktree_path,
            remapped: session.remapped,
        })
    })();
    result.map_err(classify_import_failure)
}

fn classify_import_failure(error: anyhow::Error) -> ImportFailure {
    let outcome = error
        .downcast_ref::<forge_core::capsule::CapsuleError>()
        .map(|error| match error {
            forge_core::capsule::CapsuleError::InvalidBase(_) => CapsuleOutcome::BaseUnavailable,
            forge_core::capsule::CapsuleError::RepositoryMismatch => {
                CapsuleOutcome::RepositoryMismatch
            }
            forge_core::capsule::CapsuleError::UnsafeFiles(_)
            | forge_core::capsule::CapsuleError::InvalidArchive(_)
            | forge_core::capsule::CapsuleError::CapsuleTooLarge { .. } => {
                CapsuleOutcome::UnsafeArchive
            }
            forge_core::capsule::CapsuleError::Git { operation, .. }
                if *operation == "apply capsule patch" =>
            {
                CapsuleOutcome::PatchConflict
            }
            _ => CapsuleOutcome::ImportFailed,
        })
        .unwrap_or(CapsuleOutcome::ImportFailed);
    ImportFailure {
        outcome,
        detail: format!("{error:#}"),
    }
}

fn rollback_worktree(repository: &Path, worktree: &Path) {
    let _ = Command::new("git")
        .current_dir(repository)
        .args([
            "worktree",
            "remove",
            "--force",
            worktree.to_string_lossy().as_ref(),
        ])
        .output();
}

fn validate_pending(pending: &PendingCapsule) -> Result<()> {
    if pending.version != CAPSULE_VERSION || pending.ciphertext_bytes > MAX_CAPSULE_ENVELOPE_BYTES {
        bail!("service returned invalid pending capsule metadata");
    }
    Ok(())
}

fn validate_claim(pending: &PendingCapsule, claim: &CapsuleClaim) -> Result<()> {
    if claim.version != CAPSULE_VERSION
        || claim.capsule_id != pending.capsule_id
        || claim.ciphertext_bytes != pending.ciphertext_bytes
        || claim.ciphertext_sha256 != pending.ciphertext_sha256
    {
        bail!("capsule claim does not match pending metadata");
    }
    Ok(())
}

async fn download_capsule(claim: &CapsuleClaim) -> Result<Vec<u8>> {
    let capacity = usize::try_from(claim.ciphertext_bytes)
        .context("capsule length does not fit this platform")?;
    let mut request = large_client()?.get(&claim.download_url);
    for (name, value) in &claim.required_headers {
        request = request.header(
            HeaderName::from_bytes(name.as_bytes()).context("invalid download header name")?,
            HeaderValue::from_str(value).context("invalid download header value")?,
        );
    }
    let response = request
        .send()
        .await
        .context("download encrypted handoff capsule")?
        .error_for_status()
        .context("object storage rejected capsule download")?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read encrypted handoff capsule")?;
        let next = bytes
            .len()
            .checked_add(chunk.len())
            .context("capsule length overflow")?;
        if next > capacity || next as u64 > MAX_CAPSULE_ENVELOPE_BYTES {
            bail!("capsule download exceeds its declared length");
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != capacity
        || URL_SAFE_NO_PAD.encode(Sha256::digest(&bytes)) != claim.ciphertext_sha256
    {
        bail!("capsule download integrity does not match its claim");
    }
    Ok(bytes)
}

fn accept_capsule_replay(store: &StateStore, pending: &PendingCapsule) -> Result<()> {
    let tuple = format!(
        "{}:{}:{}",
        pending.source_device_id, pending.key_epoch, pending.sequence
    );
    store.update(|state| match state.capsule_replay.get(&tuple) {
        Some(existing) if existing == &pending.capsule_id.to_string() => Ok(()),
        Some(_) => bail!("capsule replay tuple was reused for a different object"),
        None => {
            state
                .capsule_replay
                .insert(tuple, pending.capsule_id.to_string());
            Ok(())
        }
    })?;
    Ok(())
}

fn seal_acknowledgement(
    state_store: &StateStore,
    identity: &Identity,
    source_device_id: [u8; 16],
    acknowledgement: &CapsuleAcknowledgement,
) -> Result<String> {
    let sequence = allocate_sequence(state_store)?;
    let plaintext =
        serde_json::to_vec(acknowledgement).context("encode capsule acknowledgement")?;
    let envelope = Envelope::seal(
        EnvelopeMetadata {
            kind: EnvelopeKind::Capsule,
            flags: if matches!(acknowledgement.outcome, CapsuleOutcome::Accepted) {
                CAPSULE_FLAG_ACCEPTED
            } else {
                0
            },
            account_id: identity.account_id,
            sender_device_id: identity.device_id,
            recipient_kind: RecipientKind::Device,
            recipient_id: source_device_id,
            key_epoch: identity.key_epoch,
            sequence,
            created_at_ms: now_ms(),
            nonce: rand::random(),
        },
        &plaintext,
        &identity.data_key,
        &identity.signing_key,
    )?
    .encode()?;
    Ok(URL_SAFE_NO_PAD.encode(envelope))
}

async fn submit_acknowledgement(
    state_store: &StateStore,
    service_url: &str,
    access_token: &str,
    capsule_id: &CapsuleId,
    journal: &CapsuleJournalEntry,
) -> Result<()> {
    let request = CapsuleAcknowledgeRequest {
        version: CAPSULE_VERSION,
        acknowledgement_envelope: journal.acknowledgement_envelope.clone(),
    };
    let response = client()?
        .post(format!(
            "{service_url}/v1/capsules/{}/acknowledge",
            capsule_id
        ))
        .bearer_auth(access_token)
        .header("Idempotency-Key", &journal.idempotency_key)
        .json(&request)
        .send()
        .await
        .context("submit encrypted handoff acknowledgement")?;
    let status = response.status();
    if is_terminal_acknowledgement_status(status) {
        rollback_terminal_destination(state_store, capsule_id, journal)?;
        return Ok(());
    }
    if !status.is_success() {
        bail!(
            "service rejected handoff acknowledgement with HTTP {}",
            status
        );
    }
    if let Some(session_id) = journal.imported_session_id.as_deref() {
        crate::open_store()?
            .activate_destination_handoff(session_id, &capsule_id.to_string())
            .context("activate accepted destination handoff")?;
    }
    state_store.update(|state| {
        let entry = state
            .capsule_journal
            .get_mut(&capsule_id.to_string())
            .context("capsule acknowledgement journal disappeared")?;
        entry.acked_at_ms = Some(now_ms());
        Ok(())
    })?;
    Ok(())
}

fn is_terminal_acknowledgement_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 400 | 403 | 404 | 409 | 410 | 422)
}

fn rollback_terminal_destination(
    state_store: &StateStore,
    capsule_id: &CapsuleId,
    journal: &CapsuleJournalEntry,
) -> Result<()> {
    if let Some(session_id) = journal.imported_session_id.as_deref() {
        crate::open_store()?
            .rollback_handoff_session(session_id)
            .context("rollback terminally rejected destination handoff")?;
    }
    if let Some(worktree) = journal.worktree_path.as_deref() {
        if let Ok(repository) = std::env::current_dir() {
            rollback_worktree(&repository, Path::new(worktree));
        }
    }
    state_store.update(|state| {
        let entry = state
            .capsule_journal
            .get_mut(&capsule_id.to_string())
            .context("capsule acknowledgement journal disappeared")?;
        entry.terminal_at_ms = Some(now_ms());
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: &str, sequence: u64) -> PendingCapsule {
        PendingCapsule {
            version: CAPSULE_VERSION,
            capsule_id: id.parse().expect("canonical capsule id"),
            source_host_id: "10".repeat(16),
            source_device_id: "20".repeat(16),
            key_epoch: 3,
            sequence,
            ciphertext_bytes: 128,
            ciphertext_sha256: URL_SAFE_NO_PAD.encode([0x30; 32]),
            expires_at_ms: now_ms() + 60_000,
        }
    }

    #[test]
    fn destination_resolution_rejects_revoked_and_ambiguous_hosts() {
        let hosts = vec![
            HostRow {
                id: "01".repeat(16),
                name: "laptop".into(),
                revoked_at: None,
            },
            HostRow {
                id: "02".repeat(16),
                name: "LAPTOP".into(),
                revoked_at: None,
            },
            HostRow {
                id: "03".repeat(16),
                name: "old".into(),
                revoked_at: Some("now".into()),
            },
        ];
        assert!(resolve_host(&hosts, "laptop").is_err());
        assert!(resolve_host(&hosts, "old").is_err());
        assert_eq!(
            resolve_host(&hosts, &"01".repeat(16)).unwrap().id,
            "01".repeat(16)
        );
    }

    #[test]
    fn capsule_errors_show_every_rejected_path() {
        let error =
            forge_core::capsule::CapsuleError::UnsafeFiles(forge_core::capsule::UnsafePaths(vec![
                forge_core::capsule::UnsafePath {
                    path: ".env".into(),
                    reason: "secret-like path".into(),
                },
                forge_core::capsule::UnsafePath {
                    path: "link".into(),
                    reason: "symbolic link".into(),
                },
            ]));
        let shown = format_capsule_error(error);
        assert!(shown.contains(".env"));
        assert!(shown.contains("link"));
    }

    #[test]
    fn capsule_replay_accepts_exact_retry_but_rejects_tuple_reuse() {
        let temporary = tempfile::tempdir().unwrap();
        let store = StateStore {
            path: temporary.path().join("state.json"),
        };
        store
            .save(&LocalState {
                version: super::super::STATE_VERSION,
                ..LocalState::default()
            })
            .unwrap();
        let first = pending(&"aa".repeat(16), 7);
        accept_capsule_replay(&store, &first).unwrap();
        accept_capsule_replay(&store, &first).unwrap();
        let reused = pending(&"bb".repeat(16), 7);
        assert!(accept_capsule_replay(&store, &reused).is_err());
    }

    #[test]
    fn acknowledgement_signed_flag_matches_encrypted_outcome() {
        let temporary = tempfile::tempdir().unwrap();
        let store = StateStore {
            path: temporary.path().join("state.json"),
        };
        store
            .save(&LocalState {
                version: super::super::STATE_VERSION,
                next_sequence: 1,
                ..LocalState::default()
            })
            .unwrap();
        let signing_key = SigningKey::from_bytes(&[0x41; 32]);
        let identity = Identity {
            account_id: [0x11; 16],
            device_id: [0x22; 16],
            data_key: [0x33; 32],
            key_epoch: 4,
            signing_key: signing_key.clone(),
            host_id: [0x44; 16],
            data_key_epochs: BTreeMap::from([(4, [0x33; 32])]),
        };
        for (outcome, expected_flag) in [
            (CapsuleOutcome::Accepted, CAPSULE_FLAG_ACCEPTED),
            (CapsuleOutcome::PatchConflict, 0),
        ] {
            let acknowledgement = CapsuleAcknowledgement {
                version: CAPSULE_VERSION,
                capsule_id: CapsuleId::new([0xab; 16]),
                outcome: outcome.clone(),
                destination_session_id: None,
                detail: None,
            };
            let encoded =
                seal_acknowledgement(&store, &identity, [0x55; 16], &acknowledgement).unwrap();
            let envelope = Envelope::decode(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
            assert_eq!(envelope.metadata.flags, expected_flag);
            let plaintext = envelope
                .open(&identity.data_key, &signing_key.verifying_key())
                .unwrap();
            let opened: CapsuleAcknowledgement = serde_json::from_slice(&plaintext).unwrap();
            assert_eq!(opened.outcome, outcome);
        }

        let accepted = CapsuleAcknowledgement {
            version: CAPSULE_VERSION,
            capsule_id: CapsuleId::new([0xab; 16]),
            outcome: CapsuleOutcome::Accepted,
            destination_session_id: Some("destination-session".into()),
            detail: None,
        };
        let status = CapsuleStatus {
            version: CAPSULE_VERSION,
            capsule_id: accepted.capsule_id,
            state: "acknowledged".into(),
            acknowledgement_envelope: Some(
                seal_acknowledgement(&store, &identity, identity.device_id, &accepted).unwrap(),
            ),
            acknowledgement_signing_public_key: Some(
                URL_SAFE_NO_PAD.encode(signing_key.verifying_key().as_bytes()),
            ),
        };
        assert!(matches!(
            open_acknowledgement(&status, &accepted.capsule_id, &identity).unwrap(),
            OpenedAcknowledgement::Accepted(session) if session == "destination-session"
        ));
    }

    #[test]
    fn outgoing_handoff_crash_resume_preserves_exact_ciphertext_and_keys() {
        let temporary = tempfile::tempdir().unwrap();
        let store = StateStore {
            path: temporary.path().join("anywhere/state.json"),
        };
        store
            .save(&LocalState {
                version: super::super::STATE_VERSION,
                ..LocalState::default()
            })
            .unwrap();
        let capsule_id = CapsuleId::new([0x91; 16]);
        let envelope = b"exact opaque encrypted capsule";
        let path = persist_outgoing_envelope(&store, capsule_id, envelope).unwrap();
        let entry = OutgoingHandoffEntry {
            capsule_id: capsule_id.to_string(),
            destination_host_id: "22".repeat(16),
            destination_name: "laptop".into(),
            envelope_path: path.to_string_lossy().into_owned(),
            request: CapsuleReserveRequest {
                version: CAPSULE_VERSION,
                capsule_id,
                source_session_id: "session-source".into(),
                source_host_id: "11".repeat(16),
                destination_host_id: "22".repeat(16),
                ciphertext_bytes: envelope.len() as u64,
                ciphertext_sha256: URL_SAFE_NO_PAD.encode(Sha256::digest(envelope)),
            },
            reserve_idempotency_key: capsule_id.to_string(),
            complete_idempotency_key: format!("{capsule_id}-complete"),
            cancel_idempotency_key: format!("{capsule_id}-cancel"),
            accepted_destination_session_id: None,
            created_at_ms: 7,
        };
        store
            .update(|state| {
                state
                    .outgoing_handoffs
                    .insert("session-source".into(), entry.clone());
                Ok(())
            })
            .unwrap();

        let reloaded = store.load().unwrap();
        let (_, recovered) = pending_outgoing(&reloaded, "session-s").unwrap().unwrap();
        assert_eq!(recovered.capsule_id, entry.capsule_id);
        assert_eq!(
            recovered.reserve_idempotency_key,
            entry.reserve_idempotency_key
        );
        assert_eq!(std::fs::read(recovered.envelope_path).unwrap(), envelope);
    }

    #[test]
    fn pre_network_failure_removes_durable_freeze_and_operation() {
        let temporary = tempfile::tempdir().unwrap();
        let state_store = StateStore {
            path: temporary.path().join("anywhere/state.json"),
        };
        state_store
            .save(&LocalState {
                version: super::super::STATE_VERSION,
                ..LocalState::default()
            })
            .unwrap();
        let store = forge_store::Store::open_in_memory().unwrap();
        let session = store.create_session("/repo", "default").unwrap();
        let capsule_id = CapsuleId::new([0x71; 16]);

        // This is the exact ordering boundary used by create(): durable freeze first, then any
        // driver stop/export work. A racing direct/LAN resume is rejected immediately.
        state_store
            .update(|state| {
                state
                    .preparing_handoffs
                    .insert(session.clone(), capsule_id.to_string());
                Ok(())
            })
            .unwrap();
        store
            .begin_source_handoff(&session, &capsule_id.to_string())
            .unwrap();
        assert!(store.session_handoff_blocked(&session).unwrap());
        assert!(store.unarchive_session(&session).is_err());

        rollback_pre_network_handoff(&state_store, &store, &session, &capsule_id).unwrap();
        assert!(!store.session_handoff_blocked(&session).unwrap());
        assert!(!store.session_archived(&session).unwrap());
        let state = state_store.load().unwrap();
        assert!(state.outgoing_handoffs.is_empty());
        assert!(state.preparing_handoffs.is_empty());
    }

    #[test]
    fn terminal_acknowledgement_errors_are_distinct_from_retryable_failures() {
        for code in [400, 403, 404, 409, 410, 422] {
            assert!(is_terminal_acknowledgement_status(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
        for code in [401, 408, 429, 500, 503] {
            assert!(!is_terminal_acknowledgement_status(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
        let cancelled = CapsuleStatus {
            version: CAPSULE_VERSION,
            capsule_id: CapsuleId::new([4; 16]),
            state: "cancelled".into(),
            acknowledgement_envelope: None,
            acknowledgement_signing_public_key: None,
        };
        assert!(is_confirmed_cancellation(&cancelled));
        assert!(!is_confirmed_cancellation(&CapsuleStatus {
            state: "acknowledged".into(),
            acknowledgement_envelope: Some("late-ack".into()),
            ..cancelled
        }));
    }
}
