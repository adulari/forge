//! Controller-side producer for durable encrypted remote jobs.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use forge_anywhere_protocol::bridge::{BridgeRequest, RouteId};
use forge_anywhere_protocol::{
    CommandAcknowledgement, CommandEnqueueResponse, CommandResult, Envelope, EnvelopeKind,
    EnvelopeMetadata, RecipientKind, MAX_COMMAND_ENVELOPE_BYTES,
};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use super::{
    client, decode_base64_array, decode_hex_array, ensure_access_token, idempotency_key, now_ms,
    refresh_account_epoch, send_json, set_owner_directory_permissions, set_owner_file_permissions,
    sync_directory, LocalState, StateStore,
};

const JOURNAL_VERSION: u8 = 1;
const OCTET_STREAM: &str = "application/octet-stream";

#[derive(Debug, Clone, Serialize)]
struct CreateSessionBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<&'a str>,
    worktree: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temper: Option<&'a str>,
}

#[derive(Debug, Clone, Deserialize)]
struct HostList {
    hosts: Vec<HostRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct HostRow {
    id: String,
    device_id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceList {
    devices: Vec<DeviceRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceRow {
    id: String,
    signing_public_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct JobJournal {
    version: u8,
    jobs: BTreeMap<String, OutgoingJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutgoingJob {
    host_id: String,
    host_device_id: String,
    created_at_ms: u64,
    envelope: String,
    idempotency_key: String,
    #[serde(default)]
    command_id: Option<String>,
    #[serde(default)]
    expires_at_ms: Option<u64>,
    #[serde(default)]
    result: Option<CommandResult>,
}

struct JobStore {
    path: PathBuf,
}

/// Queue an encrypted create-session job and opportunistically submit/poll every pending job.
///
/// The exact envelope is persisted before the first request. A transport failure therefore leaves
/// a retryable local queue entry instead of resealing with a new nonce or sequence.
pub(super) async fn queue_create_session(
    host: &str,
    cwd: Option<&str>,
    title: Option<&str>,
    model: Option<&str>,
    temper: Option<&str>,
    worktree: bool,
) -> Result<()> {
    let state_store = StateStore::platform()?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    let http = client()?;
    let target = if is_full_host_id(host) {
        resolve_host(&http, &service_url, "", host).await?
    } else {
        let mut state = state_store.load()?;
        let token = ensure_access_token(&state_store, &mut state).await?;
        refresh_account_epoch(&state_store, &mut state, &service_url, &token).await?;
        resolve_host(&http, &service_url, &token, host).await?
    };

    let (reserved, sequence) = state_store.reserve_sequences(1)?;
    let request_id = rand::random::<[u8; 16]>();
    let body = serde_json::to_vec(&CreateSessionBody {
        cwd,
        worktree,
        title,
        model,
        temper,
    })
    .context("encode remote create-session job")?;
    let request = BridgeRequest {
        request_id,
        route: RouteId::CreateSession,
        method: "POST".to_owned(),
        parameters: Vec::new(),
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
        body_blob: None,
    };
    let plaintext = serde_json::to_vec(&request).context("encode durable remote job")?;
    let created_at_ms = now_ms();
    let identity = ProducerIdentity::from_state(&reserved)?;
    let host_id = decode_hex_array::<16>(&target.id, "destination host id")?;
    let envelope = Envelope::seal(
        EnvelopeMetadata {
            kind: EnvelopeKind::Command,
            flags: 0,
            account_id: identity.account_id,
            sender_device_id: identity.device_id,
            recipient_kind: RecipientKind::Host,
            recipient_id: host_id,
            key_epoch: identity.key_epoch,
            sequence,
            created_at_ms,
            nonce: rand::random(),
        },
        &plaintext,
        &identity.data_key,
        &identity.signing_key,
    )?
    .encode()?;
    if envelope.len() as u64 > MAX_COMMAND_ENVELOPE_BYTES {
        bail!("encrypted remote job exceeds the 256 KiB command limit");
    }

    let local_id = hex::encode(request_id);
    let jobs = JobStore::platform()?;
    jobs.update(|journal| {
        journal.jobs.insert(
            local_id.clone(),
            OutgoingJob {
                host_id: target.id,
                host_device_id: target.device_id,
                created_at_ms,
                envelope: URL_SAFE_NO_PAD.encode(envelope),
                idempotency_key: idempotency_key(),
                command_id: None,
                expires_at_ms: None,
                result: None,
            },
        );
        Ok(())
    })?;

    let delivery = async {
        let mut state = state_store.load()?;
        let token = ensure_access_token(&state_store, &mut state).await?;
        refresh_account_epoch(&state_store, &mut state, &service_url, &token).await?;
        resume_pending_inner(&state_store, &jobs, &http, &service_url, &token).await
    }
    .await;
    match delivery {
        Ok(()) => print_job(&jobs.load()?, &local_id),
        Err(error) => {
            println!("Queued encrypted remote job {local_id}; delivery will resume on the next `forge anywhere jobs` run.");
            eprintln!("⚠ Current delivery attempt failed: {error:#}");
        }
    }
    Ok(())
}

/// Retry exact pending ciphertext, poll categorical acknowledgements, and print local status.
pub(super) async fn resume_pending() -> Result<()> {
    let state_store = StateStore::platform()?;
    let mut state = state_store.load()?;
    let token = ensure_access_token(&state_store, &mut state).await?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    refresh_account_epoch(&state_store, &mut state, &service_url, &token).await?;
    let jobs = JobStore::platform()?;
    resume_pending_inner(&state_store, &jobs, &client()?, &service_url, &token).await?;
    let journal = jobs.load()?;
    if journal.jobs.is_empty() {
        println!("No encrypted remote jobs are queued.");
    } else {
        for id in journal.jobs.keys() {
            print_job(&journal, id);
        }
    }
    Ok(())
}

async fn resume_pending_inner(
    state_store: &StateStore,
    jobs: &JobStore,
    http: &reqwest::Client,
    service_url: &str,
    token: &str,
) -> Result<()> {
    let mut journal = jobs.load()?;
    let mut changed = false;
    let state = state_store.load()?;
    let identity = ProducerIdentity::from_state(&state)?;
    let devices = device_signing_keys(http, service_url, token).await?;

    for job in journal.jobs.values_mut().filter(|job| job.result.is_none()) {
        if job.command_id.is_none() {
            let envelope = URL_SAFE_NO_PAD
                .decode(&job.envelope)
                .context("decode queued remote job envelope")?;
            let response: CommandEnqueueResponse = send_json(
                http.post(format!("{service_url}/v1/hosts/{}/commands", job.host_id))
                    .bearer_auth(token)
                    .header(CONTENT_TYPE, OCTET_STREAM)
                    .header("Idempotency-Key", &job.idempotency_key)
                    .body(envelope),
            )
            .await
            .context("submit encrypted remote job")?;
            response.validate(now_ms())?;
            job.command_id = Some(response.command_id.to_string());
            job.expires_at_ms = Some(response.expires_at_ms);
            changed = true;
        }

        if job.host_device_id.is_empty() {
            let hosts: HostList = send_json(
                http.get(format!("{service_url}/v1/hosts"))
                    .bearer_auth(token),
            )
            .await
            .context("resolve queued remote job host device")?;
            job.host_device_id = hosts
                .hosts
                .into_iter()
                .find(|host| host.id == job.host_id)
                .map(|host| host.device_id)
                .context("queued remote job host is no longer enrolled")?;
            changed = true;
        }

        if job.expires_at_ms.is_some_and(|expiry| expiry <= now_ms()) {
            continue;
        }
        let command_id = job
            .command_id
            .as_deref()
            .context("queued job command id missing")?;
        let response = http
            .get(format!(
                "{service_url}/v1/hosts/{}/commands/{command_id}/ack",
                job.host_id
            ))
            .bearer_auth(token)
            .header(ACCEPT, OCTET_STREAM)
            .send()
            .await
            .context("poll encrypted remote job acknowledgement")?;
        if response.status() == StatusCode::NOT_FOUND {
            continue;
        }
        let response = response
            .error_for_status()
            .context("service rejected remote job acknowledgement poll")?;
        let encoded = response
            .bytes()
            .await
            .context("read remote job acknowledgement")?;
        let envelope = Envelope::decode(&encoded)?;
        let host_device_id = decode_hex_array::<16>(&job.host_device_id, "host device id")?;
        if envelope.metadata.kind != EnvelopeKind::Acknowledgement
            || envelope.metadata.account_id != identity.account_id
            || envelope.metadata.sender_device_id != host_device_id
            || envelope.metadata.recipient_kind != RecipientKind::Device
            || envelope.metadata.recipient_id != identity.device_id
        {
            bail!("remote job acknowledgement has mismatched routing metadata");
        }
        let signing_key = devices
            .get(&job.host_device_id)
            .context("remote job host device is unknown or revoked")?;
        let data_key = identity
            .data_key_epochs
            .get(&envelope.metadata.key_epoch)
            .context("remote job acknowledgement uses an unavailable key epoch")?;
        let plaintext = envelope.open(data_key, signing_key)?;
        let acknowledgement: CommandAcknowledgement =
            serde_json::from_slice(&plaintext).context("decode remote job acknowledgement")?;
        if acknowledgement.command_id.to_string() != command_id {
            bail!("remote job acknowledgement refers to a different command");
        }
        job.result = Some(acknowledgement.result);
        changed = true;
    }
    if changed {
        jobs.update(|latest| {
            for (id, candidate) in &journal.jobs {
                match latest.jobs.get(id) {
                    Some(current) if job_progress(current) > job_progress(candidate) => {}
                    _ => {
                        latest.jobs.insert(id.clone(), candidate.clone());
                    }
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn job_progress(job: &OutgoingJob) -> u8 {
    if job.result.is_some() {
        2
    } else if job.command_id.is_some() {
        1
    } else {
        0
    }
}

async fn resolve_host(
    http: &reqwest::Client,
    service_url: &str,
    token: &str,
    selector: &str,
) -> Result<HostRow> {
    if is_full_host_id(selector) {
        // A full host id is sufficient routing metadata for an offline queue entry. Its owning
        // device id is resolved after the service accepts delivery, before acknowledgement trust.
        return Ok(HostRow {
            id: selector.to_owned(),
            device_id: String::new(),
            name: selector.to_owned(),
        });
    }
    let list: HostList = send_json(
        http.get(format!("{service_url}/v1/hosts"))
            .bearer_auth(token),
    )
    .await?;
    let mut matches = list.hosts.into_iter().filter(|host| {
        host.id == selector
            || host.name.eq_ignore_ascii_case(selector)
            || host.id.starts_with(selector)
    });
    let found = matches.next().context("destination host was not found")?;
    if matches.next().is_some() {
        bail!("destination host selector is ambiguous");
    }
    Ok(found)
}

fn is_full_host_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn device_signing_keys(
    http: &reqwest::Client,
    service_url: &str,
    token: &str,
) -> Result<BTreeMap<String, VerifyingKey>> {
    let list: DeviceList = send_json(
        http.get(format!("{service_url}/v1/devices"))
            .bearer_auth(token),
    )
    .await?;
    list.devices
        .into_iter()
        .filter_map(|device| device.signing_public_key.map(|key| (device.id, key)))
        .map(|(id, encoded)| {
            let bytes = decode_base64_array::<32>(&encoded, "device signing public key")?;
            Ok((id, VerifyingKey::from_bytes(&bytes)?))
        })
        .collect()
}

fn print_job(journal: &JobJournal, local_id: &str) {
    let Some(job) = journal.jobs.get(local_id) else {
        return;
    };
    let status = match job.result {
        Some(CommandResult::Success) => "completed",
        Some(CommandResult::Error { .. }) => "failed",
        None if job.command_id.is_some() => "waiting for host",
        None => "queued locally",
    };
    println!("{local_id}  {}  {status}", job.host_id);
}

struct ProducerIdentity {
    account_id: [u8; 16],
    device_id: [u8; 16],
    data_key: [u8; 32],
    data_key_epochs: BTreeMap<u32, [u8; 32]>,
    key_epoch: u32,
    signing_key: SigningKey,
}

impl ProducerIdentity {
    fn from_state(state: &LocalState) -> Result<Self> {
        let key_epoch = state.key_epoch.context("Anywhere key epoch is missing")?;
        let data_key = decode_base64_array::<32>(
            state
                .account_data_key
                .as_deref()
                .context("Anywhere Account Data Key is missing")?,
            "Account Data Key",
        )?;
        let mut data_key_epochs = state
            .data_key_epochs
            .iter()
            .map(|(epoch, key)| {
                Ok((
                    *epoch,
                    decode_base64_array(key, "retained Account Data Key")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        data_key_epochs.entry(key_epoch).or_insert(data_key);
        Ok(Self {
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
            data_key,
            data_key_epochs,
            key_epoch,
            signing_key: SigningKey::from_bytes(&decode_base64_array(
                state
                    .signing_private_key
                    .as_deref()
                    .context("Anywhere signing key is missing")?,
                "device signing private key",
            )?),
        })
    }
}

impl JobStore {
    fn platform() -> Result<Self> {
        let path = forge_config::data_dir()
            .context("no Forge platform data directory is available")?
            .join("anywhere")
            .join("outgoing-jobs.json");
        Ok(Self { path })
    }

    fn load(&self) -> Result<JobJournal> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                let journal: JobJournal =
                    serde_json::from_str(&text).context("parse Anywhere remote job journal")?;
                if journal.version != JOURNAL_VERSION {
                    bail!("unsupported Anywhere remote job journal version");
                }
                Ok(journal)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(JobJournal {
                version: JOURNAL_VERSION,
                jobs: BTreeMap::new(),
            }),
            Err(error) => Err(error).context("read Anywhere remote job journal"),
        }
    }

    fn update(&self, update: impl FnOnce(&mut JobJournal) -> Result<()>) -> Result<()> {
        use fs2::FileExt as _;
        let parent = self
            .path
            .parent()
            .context("remote job journal path has no parent")?;
        std::fs::create_dir_all(parent)?;
        set_owner_directory_permissions(parent)?;
        let lock_path = parent.join("outgoing-jobs.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        set_owner_file_permissions(&lock_path)?;
        lock.lock_exclusive()?;
        let result = (|| {
            let mut journal = self.load()?;
            update(&mut journal)?;
            self.save(&journal)
        })();
        fs2::FileExt::unlock(&lock)?;
        result
    }

    fn save(&self, journal: &JobJournal) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("remote job journal path has no parent")?;
        std::fs::create_dir_all(parent)?;
        set_owner_directory_permissions(parent)?;
        let temp = parent.join(format!(
            ".outgoing-jobs-{}-{:016x}.tmp",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(journal)?)?;
        file.sync_all()?;
        drop(file);
        set_owner_file_permissions(&temp)?;
        if let Err(error) = std::fs::rename(&temp, &self.path) {
            let _ = std::fs::remove_file(&temp);
            return Err(error.into());
        }
        set_owner_file_permissions(&self.path)?;
        sync_directory(parent)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_ciphertext_and_idempotency_survive_journal_restart() {
        let directory = tempfile::tempdir().expect("temp directory");
        let store = JobStore {
            path: directory.path().join("jobs.json"),
        };
        let expected = OutgoingJob {
            host_id: "11".repeat(16),
            host_device_id: "22".repeat(16),
            created_at_ms: 10,
            envelope: "exact-ciphertext".into(),
            idempotency_key: "stable-key".into(),
            command_id: None,
            expires_at_ms: None,
            result: None,
        };
        store
            .update(|journal| {
                journal.jobs.insert("local".into(), expected.clone());
                Ok(())
            })
            .expect("persist");
        let restarted = store.load().expect("reload");
        let actual = restarted.jobs.get("local").expect("job");
        assert_eq!(actual.envelope, expected.envelope);
        assert_eq!(actual.idempotency_key, expected.idempotency_key);
    }

    #[test]
    fn create_session_payload_contains_no_service_visible_metadata() {
        let body = serde_json::to_value(CreateSessionBody {
            cwd: Some("private/repo"),
            worktree: true,
            title: Some("secret"),
            model: None,
            temper: None,
        })
        .expect("encode");
        let request = BridgeRequest {
            request_id: [1; 16],
            route: RouteId::CreateSession,
            method: "POST".into(),
            parameters: Vec::new(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&body).expect("body"),
            body_blob: None,
        };
        let plaintext = serde_json::to_vec(&request).expect("request");
        let decoded: BridgeRequest = serde_json::from_slice(&plaintext).expect("decode request");
        assert!(String::from_utf8(decoded.body)
            .expect("utf8")
            .contains("private/repo"));
        // Queue rows expose only routing/time/size outside the envelope; the journal deliberately
        // stores the request only as exact encoded ciphertext.
        let row = OutgoingJob {
            host_id: "11".repeat(16),
            host_device_id: "22".repeat(16),
            created_at_ms: 1,
            envelope: "opaque".into(),
            idempotency_key: "idempotent".into(),
            command_id: None,
            expires_at_ms: None,
            result: None,
        };
        let serialized = serde_json::to_string(&row).expect("row");
        assert!(!serialized.contains("private/repo"));
        assert!(!serialized.contains("secret"));
    }

    #[tokio::test]
    async fn full_host_id_can_be_prepared_without_service_reachability() {
        let host = resolve_host(
            &reqwest::Client::new(),
            "http://127.0.0.1:1",
            "unused",
            &"ab".repeat(16),
        )
        .await
        .expect("resolve offline host id");
        assert_eq!(host.id, "ab".repeat(16));
        assert!(host.device_id.is_empty());
    }
}
