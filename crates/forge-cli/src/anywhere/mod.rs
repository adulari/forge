//! Forge Anywhere account, device, and host commands.
//!
//! Managed credentials and private keys live only in an owner-readable data-directory file.
//! `config.toml` contains the public enable/name/sync switches and nothing secret.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use forge_anywhere_protocol::crypto::{
    derive_device_wrap_key, derive_recovery_wrap_key, exchange_public_key,
};
use forge_anywhere_protocol::keys::RecoverySecret;
use forge_anywhere_protocol::{Envelope, EnvelopeKind, EnvelopeMetadata, RecipientKind};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{AnywhereCmd, ShareExpiry};

mod connector;
mod handoff;
mod jobs;
mod push;
mod share;
mod sync;

pub(crate) fn spawn_connector(local_base_url: String) -> tokio::task::JoinHandle<()> {
    connector::spawn(local_base_url)
}

/// Send a content-free attention hint when a daemon session first becomes blocked on a person.
pub(crate) async fn notify_attention_required() {
    let Ok(config) = forge_config::load() else {
        return;
    };
    if !config.anywhere.enabled {
        return;
    }
    let Ok(store) = StateStore::platform() else {
        return;
    };
    let Ok(mut state) = store.load() else {
        return;
    };
    let Ok(token) = ensure_access_token(&store, &mut state).await else {
        return;
    };
    let Ok(http) = client() else {
        return;
    };
    push::request_best_effort(
        &http,
        config.anywhere.service_url(),
        &token,
        None,
        push::GenericPushEvent::AttentionRequired,
        &format!("attention-{}", idempotency_key()),
    )
    .await;
}

const STATE_VERSION: u8 = 1;
const KEY_EPOCH_INITIAL: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
struct LocalState {
    version: u8,
    account_id: Option<String>,
    github_login: Option<String>,
    device_id: Option<String>,
    signing_private_key: Option<String>,
    exchange_private_key: Option<String>,
    account_data_key: Option<String>,
    key_epoch: Option<u32>,
    #[serde(default)]
    data_key_epochs: BTreeMap<u32, String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    access_expires_at_ms: Option<u64>,
    host_id: Option<String>,
    #[serde(default)]
    next_sequence: u64,
    #[serde(default)]
    accepted_sequences: BTreeMap<String, u64>,
    #[serde(default)]
    command_journal: BTreeMap<String, CommandJournalEntry>,
    #[serde(default)]
    capsule_journal: BTreeMap<String, CapsuleJournalEntry>,
    #[serde(default)]
    capsule_replay: BTreeMap<String, String>,
    #[serde(default)]
    outgoing_handoffs: BTreeMap<String, OutgoingHandoffEntry>,
    /// Capsule IDs durably frozen before local export. No service request is permitted while an
    /// entry remains here, so crash recovery can safely unfreeze it.
    #[serde(default)]
    preparing_handoffs: BTreeMap<String, String>,
    #[serde(default)]
    refresh_lease_id: Option<String>,
    #[serde(default)]
    refresh_lease_until_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct CapsuleJournalEntry {
    acknowledgement_envelope: String,
    idempotency_key: String,
    #[serde(default)]
    imported_session_id: Option<String>,
    #[serde(default)]
    worktree_path: Option<String>,
    #[serde(default)]
    acked_at_ms: Option<u64>,
    #[serde(default)]
    terminal_at_ms: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct OutgoingHandoffEntry {
    capsule_id: String,
    destination_host_id: String,
    destination_name: String,
    envelope_path: String,
    request: forge_anywhere_protocol::CapsuleReserveRequest,
    reserve_idempotency_key: String,
    complete_idempotency_key: String,
    cancel_idempotency_key: String,
    #[serde(default)]
    accepted_destination_session_id: Option<String>,
    created_at_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct CommandJournalEntry {
    sender_device_id: String,
    key_epoch: u32,
    sequence: u64,
    created_at_ms: u64,
    expires_at_ms: u64,
    ciphertext_bytes: u64,
    #[serde(flatten)]
    state: CommandJournalState,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CommandJournalState {
    DispatchStarted {
        worker_id: String,
        lease_until_ms: u64,
    },
    AcknowledgementReady {
        result: forge_anywhere_protocol::CommandResult,
        envelope: String,
        idempotency_key: String,
    },
    Acked {
        acked_at_ms: u64,
    },
}

impl LocalState {
    fn is_logged_in(&self) -> bool {
        self.refresh_token.is_some()
    }

    fn clear_tokens(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.access_expires_at_ms = None;
    }
}

struct StateStore {
    path: PathBuf,
}

impl StateStore {
    fn platform() -> Result<Self> {
        let path = forge_config::data_dir()
            .context("no Forge platform data directory is available")?
            .join("anywhere")
            .join("state.json");
        Ok(Self { path })
    }

    fn load(&self) -> Result<LocalState> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LocalState {
                    version: STATE_VERSION,
                    ..LocalState::default()
                });
            }
            Err(error) => return Err(error).context("read Forge Anywhere state"),
        };
        let mut state: LocalState =
            serde_json::from_str(&text).context("parse Forge Anywhere state")?;
        if state.version != STATE_VERSION {
            bail!(
                "Forge Anywhere state version {} is unsupported by this Forge build",
                state.version
            );
        }
        if let (Some(epoch), Some(key)) = (state.key_epoch, state.account_data_key.clone()) {
            state.data_key_epochs.entry(epoch).or_insert(key);
        }
        Ok(state)
    }

    fn save(&self, state: &LocalState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("Anywhere state path has no parent")?;
        std::fs::create_dir_all(parent).context("create Forge Anywhere state directory")?;
        set_owner_directory_permissions(parent)?;

        let suffix = rand::random::<u64>();
        let temp = parent.join(format!(".state-{}-{suffix:016x}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .context("create temporary Forge Anywhere state")?;
        let bytes = serde_json::to_vec_pretty(state).context("serialize Forge Anywhere state")?;
        file.write_all(&bytes)
            .context("write Forge Anywhere state")?;
        file.sync_all().context("sync Forge Anywhere state")?;
        drop(file);
        set_owner_file_permissions(&temp)?;
        if let Err(error) = std::fs::rename(&temp, &self.path) {
            let _ = std::fs::remove_file(&temp);
            return Err(error).context("install Forge Anywhere state");
        }
        set_owner_file_permissions(&self.path)?;
        sync_directory(parent).context("sync Forge Anywhere state directory")
    }

    fn update<F>(&self, update: F) -> Result<LocalState>
    where
        F: FnOnce(&mut LocalState) -> Result<()>,
    {
        self.with_exclusive_lock(|| {
            let mut state = self.load()?;
            update(&mut state)?;
            self.save(&state)?;
            Ok(state)
        })
    }

    fn reserve_sequences(&self, count: usize) -> Result<(LocalState, u64)> {
        let count = u64::try_from(count).context("Anywhere sequence reservation is too large")?;
        self.with_exclusive_lock(|| {
            let mut state = self.load()?;
            let first = state.next_sequence;
            state.next_sequence = state
                .next_sequence
                .checked_add(count)
                .context("Anywhere outbound sequence exhausted")?;
            self.save(&state)?;
            Ok((state, first))
        })
    }

    fn with_exclusive_lock<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        use fs2::FileExt as _;

        let parent = self
            .path
            .parent()
            .context("Anywhere state path has no parent")?;
        std::fs::create_dir_all(parent).context("create Forge Anywhere state directory")?;
        set_owner_directory_permissions(parent)?;
        let lock_path = parent.join("state.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options
            .open(&lock_path)
            .context("open Anywhere state lock")?;
        set_owner_file_permissions(&lock_path)?;
        lock.lock_exclusive().context("lock Anywhere state")?;
        let result = operation();
        fs2::FileExt::unlock(&lock).context("unlock Anywhere state")?;
        result
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .context("set owner-only Anywhere state permissions")
}

#[cfg(not(unix))]
fn set_owner_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .context("set owner-only Anywhere directory permissions")
}

#[cfg(not(unix))]
fn set_owner_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[derive(Serialize)]
struct DeviceFlowRequest<'a> {
    device_name: &'a str,
    signing_public_key: String,
    exchange_public_key: String,
}

#[derive(Deserialize)]
struct DeviceFlowStart {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct AuthSession {
    account_id: String,
    device_id: String,
    github_login: String,
    access_token: String,
    refresh_token: String,
    access_expires_at_ms: u64,
    #[serde(default)]
    new_account: bool,
    #[serde(default)]
    recovery_wrap_envelope: Option<String>,
    #[serde(default)]
    recovery_wrap_signing_public_key: Option<String>,
}

#[derive(Serialize)]
struct PollRequest<'a> {
    device_code: &'a str,
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    refresh_token: &'a str,
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: String,
    access_expires_at_ms: u64,
}

#[derive(Serialize)]
struct BootstrapEpochRequest {
    epoch: u32,
    device_wrap_envelope: String,
    recovery_wrap_envelope: String,
}

#[derive(Serialize)]
struct DeviceWrapRequest {
    epoch: u32,
    device_wrap_envelope: String,
}

#[derive(Serialize)]
struct HostRequest<'a> {
    name: &'a str,
    device_id: &'a str,
}

#[derive(Deserialize)]
struct HostResponse {
    id: String,
}

#[derive(Deserialize)]
struct MeResponse {
    entitlement: String,
    #[serde(default)]
    trial_ends_at: Option<String>,
    active_hosts: u32,
    devices: u32,
    storage_used_bytes: u64,
    storage_limit_bytes: u64,
}

#[derive(Deserialize)]
struct DeviceList {
    devices: Vec<DeviceRow>,
}

#[derive(Deserialize)]
struct DeviceRow {
    id: String,
    name: String,
    created_at: String,
    #[serde(default)]
    last_seen_at: Option<String>,
    #[serde(default)]
    signing_public_key: Option<String>,
    #[serde(default)]
    exchange_public_key: Option<String>,
}

#[derive(Deserialize)]
struct RecoveryWrapResponse {
    epoch: u32,
    recovery_wrap_envelope: String,
    signing_public_key: String,
}

#[derive(Serialize)]
struct RotationDeviceWrap {
    device_id: String,
    envelope: String,
}

#[derive(Serialize)]
struct RevokeDeviceRequest {
    epoch: u32,
    recovery_wrap_envelope: String,
    device_wraps: Vec<RotationDeviceWrap>,
}

#[derive(Deserialize)]
struct RevokeDeviceResponse {
    epoch: u32,
}

#[derive(Deserialize)]
struct CurrentDeviceWrapResponse {
    epoch: u32,
    device_wrap_envelope: String,
    signing_public_key: String,
    exchange_public_key: String,
}

pub(crate) async fn anywhere_cmd(command: AnywhereCmd) -> Result<()> {
    match command {
        AnywhereCmd::Login => login().await,
        AnywhereCmd::Enable { name } => enable(name).await,
        AnywhereCmd::Status => status().await,
        AnywhereCmd::Handoff { session, to } => handoff(&session, &to).await,
        AnywhereCmd::Share { session, expires } => share(&session, expires).await,
        AnywhereCmd::Job {
            to,
            cwd,
            title,
            model,
            temper,
            worktree,
        } => {
            jobs::queue_create_session(
                &to,
                cwd.as_deref(),
                title.as_deref(),
                model.as_deref(),
                temper.as_deref(),
                worktree,
            )
            .await
        }
        AnywhereCmd::Jobs => jobs::resume_pending().await,
        AnywhereCmd::Devices { revoke } => devices(revoke.as_deref()).await,
        AnywhereCmd::Disable => disable().await,
        AnywhereCmd::Logout => logout().await,
    }
}

async fn login() -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    if state.is_logged_in() {
        println!(
            "Already logged in{}.",
            state
                .github_login
                .as_deref()
                .map(|login| format!(" as {login}"))
                .unwrap_or_default()
        );
        return Ok(());
    }

    let signing_private = rand::random::<[u8; 32]>();
    let signing_key = SigningKey::from_bytes(&signing_private);
    let exchange_private = rand::random::<[u8; 32]>();
    let exchange_public = exchange_public_key(&exchange_private);
    let device_name = default_host_name();
    let config = forge_config::load()?;
    let service_url = config.anywhere.service_url();
    let client = client()?;
    let start: DeviceFlowStart = send_json(
        client
            .post(format!("{service_url}/v1/auth/github/start"))
            .json(&DeviceFlowRequest {
                device_name: &device_name,
                signing_public_key: URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes()),
                exchange_public_key: URL_SAFE_NO_PAD.encode(exchange_public),
            }),
    )
    .await
    .context("start GitHub device login")?;

    println!(
        "Open {} and enter code {}",
        start.verification_uri, start.user_code
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(start.expires_in);
    let interval = Duration::from_secs(start.interval.clamp(1, 15));
    let auth: AuthSession = loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("GitHub device login expired; run `forge anywhere login` again");
        }
        tokio::time::sleep(interval).await;
        let response = client
            .post(format!("{service_url}/v1/auth/device/poll"))
            .json(&PollRequest {
                device_code: &start.device_code,
            })
            .send()
            .await
            .context("poll GitHub device login")?;
        if response.status() == StatusCode::ACCEPTED {
            continue;
        }
        break decode_response(response).await?;
    };

    let account_id = decode_hex_array::<16>(&auth.account_id, "account id")?;
    let device_id = decode_hex_array::<16>(&auth.device_id, "device id")?;
    let (data_key, key_epoch, next_sequence) = if auth.new_account {
        let data_key = bootstrap_new_account(
            &client,
            service_url,
            &auth.access_token,
            account_id,
            device_id,
            &signing_key,
            &exchange_private,
            &exchange_public,
        )
        .await?;
        // Bootstrap emitted the device wrap at sequence 0 and recovery wrap at sequence 1.
        (data_key, KEY_EPOCH_INITIAL, 2)
    } else {
        recover_existing_account(
            &client,
            service_url,
            &auth,
            account_id,
            device_id,
            &signing_key,
            &exchange_private,
            &exchange_public,
        )
        .await?
    };

    state = LocalState {
        version: STATE_VERSION,
        account_id: Some(auth.account_id),
        github_login: Some(auth.github_login),
        device_id: Some(auth.device_id),
        signing_private_key: Some(URL_SAFE_NO_PAD.encode(signing_private)),
        exchange_private_key: Some(URL_SAFE_NO_PAD.encode(exchange_private)),
        account_data_key: Some(URL_SAFE_NO_PAD.encode(data_key)),
        key_epoch: Some(key_epoch),
        data_key_epochs: BTreeMap::from([(key_epoch, URL_SAFE_NO_PAD.encode(data_key))]),
        access_token: Some(auth.access_token),
        refresh_token: Some(auth.refresh_token),
        access_expires_at_ms: Some(auth.access_expires_at_ms),
        host_id: None,
        next_sequence,
        accepted_sequences: BTreeMap::new(),
        command_journal: BTreeMap::new(),
        capsule_journal: BTreeMap::new(),
        capsule_replay: BTreeMap::new(),
        outgoing_handoffs: BTreeMap::new(),
        preparing_handoffs: BTreeMap::new(),
        refresh_lease_id: None,
        refresh_lease_until_ms: 0,
    };
    store.save(&state)?;
    println!("Logged in to Forge Anywhere. Run `forge anywhere enable` on this host.");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn bootstrap_new_account(
    client: &Client,
    service_url: &str,
    access_token: &str,
    account_id: [u8; 16],
    device_id: [u8; 16],
    signing_key: &SigningKey,
    exchange_private: &[u8; 32],
    exchange_public: &[u8; 32],
) -> Result<[u8; 32]> {
    let recovery = RecoverySecret::from_entropy(rand::random::<[u8; 32]>())?;
    let words = recovery.words()?;
    println!("\nRecovery phrase (shown once — store it offline):\n\n{words}\n");
    confirm_recovery_words(&words)?;

    let data_key = rand::random::<[u8; 32]>();
    let device_wrap_key = derive_device_wrap_key(
        exchange_private,
        exchange_public,
        &account_id,
        KEY_EPOCH_INITIAL,
    )?;
    let recovery_wrap_key =
        derive_recovery_wrap_key(recovery.as_bytes(), &account_id, KEY_EPOCH_INITIAL)?;
    let device_wrap = key_wrap_envelope(
        data_key,
        *device_wrap_key.as_bytes(),
        account_id,
        device_id,
        RecipientKind::Device,
        device_id,
        KEY_EPOCH_INITIAL,
        0,
        signing_key,
    )?;
    let recovery_wrap = key_wrap_envelope(
        data_key,
        *recovery_wrap_key.as_bytes(),
        account_id,
        device_id,
        RecipientKind::Account,
        account_id,
        KEY_EPOCH_INITIAL,
        1,
        signing_key,
    )?;
    let _: serde_json::Value = send_json(
        client
            .post(format!("{service_url}/v1/key-epochs"))
            .bearer_auth(access_token)
            .header("Idempotency-Key", idempotency_key())
            .json(&BootstrapEpochRequest {
                epoch: KEY_EPOCH_INITIAL,
                device_wrap_envelope: URL_SAFE_NO_PAD.encode(device_wrap),
                recovery_wrap_envelope: URL_SAFE_NO_PAD.encode(recovery_wrap),
            }),
    )
    .await
    .context("bootstrap encrypted account key")?;
    Ok(data_key)
}

#[allow(clippy::too_many_arguments)]
async fn recover_existing_account(
    client: &Client,
    service_url: &str,
    auth: &AuthSession,
    account_id: [u8; 16],
    device_id: [u8; 16],
    signing_key: &SigningKey,
    exchange_private: &[u8; 32],
    exchange_public: &[u8; 32],
) -> Result<([u8; 32], u32, u64)> {
    let encoded_wrap = auth
        .recovery_wrap_envelope
        .as_deref()
        .context("service did not return the encrypted recovery wrap")?;
    let wrap_signing_public_key = auth
        .recovery_wrap_signing_public_key
        .as_deref()
        .context("service did not return the recovery-wrap signing key")?;
    let words = rpassword::prompt_password("24-word recovery phrase: ")?;
    let recovery = RecoverySecret::from_words(words.trim())?;
    let envelope = Envelope::decode(&URL_SAFE_NO_PAD.decode(encoded_wrap)?)?;
    let recovery_key = derive_recovery_wrap_key(
        recovery.as_bytes(),
        &account_id,
        envelope.metadata.key_epoch,
    )?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&decode_base64_array(
        wrap_signing_public_key,
        "recovery-wrap signing key",
    )?)?;
    let plaintext = envelope.open(recovery_key.as_bytes(), &verifying_key)?;
    let data_key: [u8; 32] = plaintext
        .try_into()
        .map_err(|_| anyhow::anyhow!("recovered Account Data Key has the wrong length"))?;

    let device_wrap_key = derive_device_wrap_key(
        exchange_private,
        exchange_public,
        &account_id,
        envelope.metadata.key_epoch,
    )?;
    let device_wrap = key_wrap_envelope(
        data_key,
        *device_wrap_key.as_bytes(),
        account_id,
        device_id,
        RecipientKind::Device,
        device_id,
        envelope.metadata.key_epoch,
        0,
        signing_key,
    )?;
    let _: serde_json::Value = send_json(
        client
            .post(format!(
                "{service_url}/v1/key-epochs/{}/wraps",
                envelope.metadata.key_epoch
            ))
            .bearer_auth(&auth.access_token)
            .header("Idempotency-Key", idempotency_key())
            .json(&DeviceWrapRequest {
                epoch: envelope.metadata.key_epoch,
                device_wrap_envelope: URL_SAFE_NO_PAD.encode(device_wrap),
            }),
    )
    .await
    .context("enroll recovered device key")?;
    // Enrollment emitted this device's wrap at sequence 0.
    Ok((data_key, envelope.metadata.key_epoch, 1))
}

#[allow(clippy::too_many_arguments)]
fn key_wrap_envelope(
    data_key: [u8; 32],
    wrap_key: [u8; 32],
    account_id: [u8; 16],
    sender_device_id: [u8; 16],
    recipient_kind: RecipientKind,
    recipient_id: [u8; 16],
    key_epoch: u32,
    sequence: u64,
    signing_key: &SigningKey,
) -> Result<Vec<u8>> {
    let envelope = Envelope::seal(
        EnvelopeMetadata {
            kind: EnvelopeKind::KeyWrap,
            flags: 0,
            account_id,
            sender_device_id,
            recipient_kind,
            recipient_id,
            key_epoch,
            sequence,
            created_at_ms: now_ms(),
            nonce: rand::random::<[u8; 24]>(),
        },
        &data_key,
        &wrap_key,
        signing_key,
    )?;
    Ok(envelope.encode()?)
}

async fn enable(name: Option<String>) -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let token = ensure_access_token(&store, &mut state).await?;
    let device_id = state
        .device_id
        .as_deref()
        .context("device is not enrolled")?;
    let name = name.unwrap_or_else(default_host_name);
    if name.trim().is_empty() || name.len() > 80 {
        bail!("host name must contain 1–80 characters");
    }
    let config = forge_config::load()?;
    let service_url = config.anywhere.service_url();
    let host: HostResponse = send_json(
        client()?
            .post(format!("{service_url}/v1/hosts"))
            .bearer_auth(token)
            .header("Idempotency-Key", idempotency_key())
            .json(&HostRequest {
                name: name.trim(),
                device_id,
            }),
    )
    .await
    .context("register Anywhere host")?;
    state.host_id = Some(host.id);
    store.save(&state)?;
    forge_config::write_anywhere_settings(true, Some(name.trim()), config.anywhere.sync)?;
    crate::open_store()?
        .set_sync_journal_enabled(config.anywhere.sync)
        .context("enable Anywhere sync journal")?;
    let activation = ensure_managed_connector().await?;
    println!(
        "Forge Anywhere is enabled for host '{}'. {activation}",
        name.trim(),
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectorActivation {
    Attach,
    StartDaemon,
}

fn connector_activation(has_discovery_state: bool, probe_succeeded: bool) -> ConnectorActivation {
    if has_discovery_state && probe_succeeded {
        ConnectorActivation::Attach
    } else {
        ConnectorActivation::StartDaemon
    }
}

async fn ensure_managed_connector() -> Result<&'static str> {
    if let Some(serve) = crate::serve::read_state()? {
        if serve.exposure == "lan" && serve.process_is_alive() {
            // LAN listeners use a self-signed TLS certificate, so the local CLI intentionally
            // does not bypass certificate validation to hit the trigger endpoint. The daemon's
            // supervisor observes the just-written config and starts within one poll interval.
            tokio::time::sleep(Duration::from_millis(600)).await;
            return Ok("Attached the managed connector to the running `forge serve` daemon.");
        }
        let endpoint = format!(
            "{}/api/anywhere/enable",
            serve.base_url.trim_end_matches('/')
        );
        match client()?.post(endpoint).send().await {
            Ok(response) if response.status().is_success() => {
                debug_assert_eq!(
                    connector_activation(true, true),
                    ConnectorActivation::Attach
                );
                return Ok("Attached the managed connector to the running `forge serve` daemon.");
            }
            Ok(response) => {
                bail!(
                    "the running forge serve daemon could not attach Anywhere (HTTP {}); restart that daemon once, then retry `forge anywhere enable`",
                    response.status()
                );
            }
            Err(error) if !error.is_connect() && !error.is_timeout() => {
                return Err(error).context("attach Anywhere to running forge serve daemon");
            }
            Err(_) => {
                // A crash may leave serve-state.json behind. Starting below refreshes discovery.
            }
        }
    }
    debug_assert_eq!(
        connector_activation(false, false),
        ConnectorActivation::StartDaemon
    );
    let executable = std::env::current_exe().context("locate the forge executable")?;
    let mut command = std::process::Command::new(&executable);
    command
        .args(["serve", "--local"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().with_context(|| {
        format!(
            "start managed local daemon with `{}` serve --local",
            executable.display()
        )
    })?;
    Ok("Started a local `forge serve` daemon with the managed connector.")
}

async fn status() -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let config = forge_config::load()?;
    println!(
        "Local: {} · connector {} · sync {}",
        state
            .github_login
            .as_deref()
            .map(|login| format!("logged in as {login}"))
            .unwrap_or_else(|| "logged out".into()),
        if config.anywhere.enabled {
            "enabled"
        } else {
            "disabled"
        },
        if config.anywhere.sync { "on" } else { "off" }
    );
    if !state.is_logged_in() {
        return Ok(());
    }
    let token = ensure_access_token(&store, &mut state).await?;
    let me: MeResponse = send_json(
        client()?
            .get(format!("{}/v1/me", config.anywhere.service_url()))
            .bearer_auth(token),
    )
    .await
    .context("load Anywhere account status")?;
    println!(
        "Service: {} · {} host(s) · {} device(s) · {} / {} encrypted storage",
        me.entitlement,
        me.active_hosts,
        me.devices,
        human_bytes(me.storage_used_bytes),
        human_bytes(me.storage_limit_bytes)
    );
    if let Some(trial_end) = me.trial_ends_at {
        println!("Trial ends: {trial_end}");
    }
    Ok(())
}

async fn share(session: &str, expires: ShareExpiry) -> Result<()> {
    share::create(session, expires).await
}

async fn handoff(session: &str, to: &str) -> Result<()> {
    handoff::create(session, to).await
}

async fn devices(revoke: Option<&str>) -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let token = ensure_access_token(&store, &mut state).await?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    refresh_account_epoch(&store, &mut state, &service_url, &token).await?;
    if let Some(device_id) = revoke {
        revoke_device(&store, &mut state, &service_url, &token, device_id).await?;
        return Ok(());
    }

    let rows: DeviceList = send_json(
        client()?
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(token),
    )
    .await
    .context("list Anywhere devices")?;
    for device in rows.devices {
        println!(
            "{}  {}  enrolled {}{}",
            device.id,
            device.name,
            device.created_at,
            device
                .last_seen_at
                .map(|seen| format!("  last seen {seen}"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

async fn revoke_device(
    store: &StateStore,
    state: &mut LocalState,
    service_url: &str,
    access_token: &str,
    target_device_id: &str,
) -> Result<()> {
    let rows: DeviceList = send_json(
        client()?
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(access_token),
    )
    .await
    .context("load active devices before revocation")?;
    if !rows
        .devices
        .iter()
        .any(|device| device.id == target_device_id)
    {
        bail!("device {target_device_id} is not an active enrolled device");
    }
    let account_id = decode_hex_array::<16>(
        state
            .account_id
            .as_deref()
            .context("Anywhere account id is missing")?,
        "account id",
    )?;
    let sender_device_id = decode_hex_array::<16>(
        state
            .device_id
            .as_deref()
            .context("Anywhere device id is missing")?,
        "device id",
    )?;
    let signing_private = decode_base64_array::<32>(
        state
            .signing_private_key
            .as_deref()
            .context("Anywhere signing key is missing")?,
        "signing private key",
    )?;
    let signing_key = SigningKey::from_bytes(&signing_private);
    let exchange_private = decode_base64_array::<32>(
        state
            .exchange_private_key
            .as_deref()
            .context("Anywhere exchange key is missing")?,
        "exchange private key",
    )?;
    let current_data_key = decode_base64_array::<32>(
        state
            .account_data_key
            .as_deref()
            .context("Anywhere account data key is missing")?,
        "account data key",
    )?;
    let current_epoch = state.key_epoch.context("Anywhere key epoch is missing")?;
    let new_epoch = current_epoch
        .checked_add(1)
        .context("Anywhere key epoch is exhausted")?;

    let current_recovery: RecoveryWrapResponse = send_json(
        client()?
            .get(format!(
                "{service_url}/v1/key-epochs/{current_epoch}/wraps/recovery"
            ))
            .bearer_auth(access_token),
    )
    .await
    .context("load the current encrypted recovery wrap")?;
    if current_recovery.epoch != current_epoch {
        bail!("service returned the wrong recovery key epoch");
    }
    let recovery_words = rpassword::prompt_password("24-word recovery phrase: ")?;
    let recovery = RecoverySecret::from_words(recovery_words.trim())?;
    let old_envelope = Envelope::decode(
        &URL_SAFE_NO_PAD
            .decode(&current_recovery.recovery_wrap_envelope)
            .context("decode current recovery wrap")?,
    )?;
    let old_recovery_key =
        derive_recovery_wrap_key(recovery.as_bytes(), &account_id, current_epoch)?;
    let old_signing_key = ed25519_dalek::VerifyingKey::from_bytes(&decode_base64_array(
        &current_recovery.signing_public_key,
        "recovery-wrap signing key",
    )?)?;
    let recovered = old_envelope.open(old_recovery_key.as_bytes(), &old_signing_key)?;
    if recovered.as_slice() != current_data_key {
        bail!("recovery phrase does not match the current account key; no device was revoked");
    }

    let new_data_key = rand::random::<[u8; 32]>();
    let wrap_count = rows.devices.len().saturating_sub(1).saturating_add(1);
    let (reserved_state, mut next_sequence) = store.reserve_sequences(wrap_count)?;
    *state = reserved_state;
    let mut device_wraps = Vec::with_capacity(rows.devices.len().saturating_sub(1));
    for device in &rows.devices {
        if device.id == target_device_id {
            continue;
        }
        let recipient_id = decode_hex_array::<16>(&device.id, "recipient device id")?;
        let recipient_exchange_key = decode_base64_array::<32>(
            device
                .exchange_public_key
                .as_deref()
                .context("service omitted a device exchange key")?,
            "device exchange public key",
        )?;
        let wrap_key = derive_device_wrap_key(
            &exchange_private,
            &recipient_exchange_key,
            &account_id,
            new_epoch,
        )?;
        let envelope = key_wrap_envelope(
            new_data_key,
            *wrap_key.as_bytes(),
            account_id,
            sender_device_id,
            RecipientKind::Device,
            recipient_id,
            new_epoch,
            next_sequence,
            &signing_key,
        )?;
        next_sequence = next_sequence
            .checked_add(1)
            .context("Anywhere sequence is exhausted")?;
        device_wraps.push(RotationDeviceWrap {
            device_id: device.id.clone(),
            envelope: URL_SAFE_NO_PAD.encode(envelope),
        });
    }
    let recovery_wrap_key = derive_recovery_wrap_key(recovery.as_bytes(), &account_id, new_epoch)?;
    let recovery_wrap = key_wrap_envelope(
        new_data_key,
        *recovery_wrap_key.as_bytes(),
        account_id,
        sender_device_id,
        RecipientKind::Account,
        account_id,
        new_epoch,
        next_sequence,
        &signing_key,
    )?;
    next_sequence = next_sequence
        .checked_add(1)
        .context("Anywhere sequence is exhausted")?;

    debug_assert_eq!(next_sequence, state.next_sequence);
    let response: RevokeDeviceResponse = send_json(
        client()?
            .post(format!(
                "{service_url}/v1/devices/{target_device_id}/revoke"
            ))
            .bearer_auth(access_token)
            .header("Idempotency-Key", idempotency_key())
            .json(&RevokeDeviceRequest {
                epoch: new_epoch,
                recovery_wrap_envelope: URL_SAFE_NO_PAD.encode(recovery_wrap),
                device_wraps,
            }),
    )
    .await
    .context("atomically revoke device and rotate the account key")?;
    if response.epoch != new_epoch {
        bail!("service acknowledged the wrong replacement key epoch");
    }
    let target_is_local = state.device_id.as_deref() == Some(target_device_id);
    let encoded_data_key = URL_SAFE_NO_PAD.encode(new_data_key);
    *state = store.update(|latest| {
        if target_is_local {
            latest.clear_tokens();
        } else if latest.key_epoch.unwrap_or(0) < new_epoch {
            latest.account_data_key = Some(encoded_data_key.clone());
            latest.key_epoch = Some(new_epoch);
            latest.data_key_epochs.insert(new_epoch, encoded_data_key);
        }
        Ok(())
    })?;
    println!("Revoked device {target_device_id} and rotated encrypted data to epoch {new_epoch}.");
    Ok(())
}

async fn disable() -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let config = forge_config::load()?;
    if let Some(host_id) = state.host_id.clone() {
        if state.is_logged_in() {
            let token = ensure_access_token(&store, &mut state).await?;
            let response = client()?
                .delete(format!(
                    "{}/v1/hosts/{host_id}",
                    config.anywhere.service_url()
                ))
                .bearer_auth(token)
                .header("Idempotency-Key", idempotency_key())
                .send()
                .await
                .context("revoke Anywhere host")?;
            require_empty_success(response).await?;
        }
        state.host_id = None;
        store.save(&state)?;
    }
    forge_config::write_anywhere_settings(
        false,
        config.anywhere.host_name.as_deref(),
        config.anywhere.sync,
    )?;
    crate::open_store()?
        .set_sync_journal_enabled(false)
        .context("disable Anywhere sync journal")?;
    println!("Forge Anywhere is disabled. Local Forge and direct remote access are unchanged.");
    Ok(())
}

async fn logout() -> Result<()> {
    let store = StateStore::platform()?;
    let state = store.load()?;
    let refresh_token = state.refresh_token.clone();
    let service_url = forge_config::load()
        .ok()
        .map(|config| config.anywhere.service_url().to_owned());
    let remote_warning =
        if let (Some(refresh_token), Some(service_url)) = (refresh_token.as_deref(), service_url) {
            match client() {
                Ok(client) => match client
                    .post(format!("{service_url}/v1/auth/logout"))
                    .json(&RefreshRequest { refresh_token })
                    .send()
                    .await
                {
                    Ok(response) => require_empty_success(response).await.err(),
                    Err(error) => Some(error.into()),
                },
                Err(error) => Some(error),
            }
        } else {
            None
        };
    // Local revocation is the security boundary. It is one owner-only atomic state replacement
    // and must not depend on service reachability; encrypted history and device keys stay intact.
    let _ = store.update(|latest| {
        latest.clear_tokens();
        Ok(())
    })?;
    if let Some(error) = remote_warning {
        eprintln!(
            "⚠ remote logout could not be confirmed; local credentials were cleared: {error}"
        );
    }
    println!("Logged out. Local data and device keys were left intact.");
    Ok(())
}

async fn ensure_access_token(store: &StateStore, state: &mut LocalState) -> Result<String> {
    let lease_id = hex::encode(rand::random::<[u8; 16]>());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    loop {
        let now = now_ms();
        let mut claimed_refresh = None;
        *state = store.update(|latest| {
            if latest.access_expires_at_ms.unwrap_or(0) > now.saturating_add(30_000) {
                return Ok(());
            }
            if latest.refresh_lease_until_ms > now
                && latest.refresh_lease_id.as_deref() != Some(lease_id.as_str())
            {
                return Ok(());
            }
            claimed_refresh = Some(
                latest
                    .refresh_token
                    .clone()
                    .context("not logged in; run `forge anywhere login`")?,
            );
            latest.refresh_lease_id = Some(lease_id.clone());
            latest.refresh_lease_until_ms = now.saturating_add(30_000);
            Ok(())
        })?;
        if state.access_expires_at_ms.unwrap_or(0) > now.saturating_add(30_000) {
            return state
                .access_token
                .clone()
                .context("Anywhere access token is missing");
        }
        let Some(refresh_token) = claimed_refresh else {
            if tokio::time::Instant::now() >= deadline {
                bail!("timed out waiting for another Forge process to refresh Anywhere login");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };
        let service_url = forge_config::load()?.anywhere.service_url().to_owned();
        let result: Result<RefreshResponse> = send_json(
            client()?
                .post(format!("{service_url}/v1/auth/refresh"))
                .json(&RefreshRequest {
                    refresh_token: &refresh_token,
                }),
        )
        .await
        .context("refresh Anywhere login");
        match result {
            Ok(refreshed) => {
                let access_token = refreshed.access_token.clone();
                *state = store.update(|latest| {
                    if latest.refresh_lease_id.as_deref() == Some(lease_id.as_str()) {
                        latest.access_token = Some(refreshed.access_token);
                        latest.refresh_token = Some(refreshed.refresh_token);
                        latest.access_expires_at_ms = Some(refreshed.access_expires_at_ms);
                        latest.refresh_lease_id = None;
                        latest.refresh_lease_until_ms = 0;
                    }
                    Ok(())
                })?;
                return Ok(access_token);
            }
            Err(error) => {
                let _ = store.update(|latest| {
                    if latest.refresh_lease_id.as_deref() == Some(lease_id.as_str()) {
                        latest.refresh_lease_id = None;
                        latest.refresh_lease_until_ms = 0;
                    }
                    Ok(())
                });
                return Err(error);
            }
        }
    }
}

async fn refresh_account_epoch(
    store: &StateStore,
    state: &mut LocalState,
    service_url: &str,
    access_token: &str,
) -> Result<()> {
    let current: CurrentDeviceWrapResponse = send_json(
        client()?
            .get(format!("{service_url}/v1/key-epochs/current/wraps/device"))
            .bearer_auth(access_token),
    )
    .await
    .context("load current Anywhere key epoch")?;
    let local_epoch = state.key_epoch.context("Anywhere key epoch is missing")?;
    if current.epoch == local_epoch {
        return Ok(());
    }
    if current.epoch < local_epoch {
        bail!(
            "service key epoch {} is older than local epoch {local_epoch}",
            current.epoch
        );
    }
    let account_id = decode_hex_array::<16>(
        state
            .account_id
            .as_deref()
            .context("Anywhere account id is missing")?,
        "account id",
    )?;
    let device_id = decode_hex_array::<16>(
        state
            .device_id
            .as_deref()
            .context("Anywhere device id is missing")?,
        "device id",
    )?;
    let exchange_private = decode_base64_array::<32>(
        state
            .exchange_private_key
            .as_deref()
            .context("Anywhere exchange private key is missing")?,
        "exchange private key",
    )?;
    let sender_exchange_public =
        decode_base64_array::<32>(&current.exchange_public_key, "sender exchange public key")?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&decode_base64_array::<32>(
        &current.signing_public_key,
        "sender signing public key",
    )?)?;
    let envelope = Envelope::decode(
        &URL_SAFE_NO_PAD
            .decode(&current.device_wrap_envelope)
            .context("decode current device key wrap")?,
    )?;
    if envelope.metadata.kind != EnvelopeKind::KeyWrap
        || envelope.metadata.account_id != account_id
        || envelope.metadata.recipient_kind != RecipientKind::Device
        || envelope.metadata.recipient_id != device_id
        || envelope.metadata.key_epoch != current.epoch
    {
        bail!("service returned a device key wrap with mismatched routing metadata");
    }
    let wrap_key = derive_device_wrap_key(
        &exchange_private,
        &sender_exchange_public,
        &account_id,
        current.epoch,
    )?;
    let plaintext = envelope.open(wrap_key.as_bytes(), &verifying_key)?;
    let data_key: [u8; 32] = plaintext
        .try_into()
        .map_err(|_| anyhow::anyhow!("current Account Data Key has the wrong length"))?;
    let encoded_data_key = URL_SAFE_NO_PAD.encode(data_key);
    *state = store.update(|latest| {
        if latest.key_epoch.unwrap_or(0) < current.epoch {
            latest.account_data_key = Some(encoded_data_key.clone());
            latest.key_epoch = Some(current.epoch);
            latest
                .data_key_epochs
                .insert(current.epoch, encoded_data_key);
            // This device has not emitted an envelope in the new epoch yet. The replay namespace
            // includes key_epoch, so sequence zero is safe.
            latest.next_sequence = 0;
        }
        Ok(())
    })?;
    Ok(())
}

fn client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("build Forge Anywhere HTTP client")
}

async fn send_json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T> {
    decode_response(request.send().await.context("send Anywhere request")?).await
}

async fn decode_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("read Anywhere service response")?;
    if !status.is_success() {
        bail!(service_error(status, &body));
    }
    serde_json::from_slice(&body).with_context(|| {
        format!(
            "decode Anywhere service response (HTTP {})",
            status.as_u16()
        )
    })
}

async fn require_empty_success(response: reqwest::Response) -> Result<()> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("read Anywhere service response")?;
    if status.is_success() {
        Ok(())
    } else {
        bail!(service_error(status, &body))
    }
}

fn service_error(status: StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ErrorBody {
        code: Option<String>,
        message: Option<String>,
    }
    serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|error| {
            format!(
                "Anywhere service returned HTTP {}{}{}",
                status.as_u16(),
                error
                    .code
                    .map(|code| format!(" [{code}]"))
                    .unwrap_or_default(),
                error
                    .message
                    .map(|message| format!(": {message}"))
                    .unwrap_or_default()
            )
        })
        .unwrap_or_else(|| format!("Anywhere service returned HTTP {}", status.as_u16()))
}

fn confirm_recovery_words(words: &str) -> Result<()> {
    let words: Vec<&str> = words.split_whitespace().collect();
    if words.len() != 24 {
        bail!("generated recovery phrase did not contain 24 words");
    }
    for index in [2_usize, 11, 20] {
        let answer = rpassword::prompt_password(format!("Recovery word {}: ", index + 1))?;
        if answer.trim() != words[index] {
            bail!("recovery confirmation failed; no key material was uploaded");
        }
    }
    Ok(())
}

fn default_host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Forge host".into())
}

fn decode_hex_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    hex::decode(value)
        .with_context(|| format!("decode {label}"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must contain {N} bytes"))
}

fn decode_base64_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    URL_SAFE_NO_PAD
        .decode(value)
        .with_context(|| format!("decode {label}"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must contain {N} bytes"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn idempotency_key() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

fn human_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_is_owner_only_and_logout_preserves_keys() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = StateStore {
            path: temp.path().join("anywhere/state.json"),
        };
        let mut state = LocalState {
            version: STATE_VERSION,
            signing_private_key: Some("private".into()),
            account_data_key: Some("data".into()),
            access_token: Some("access".into()),
            refresh_token: Some("refresh".into()),
            ..LocalState::default()
        };
        store.save(&state).expect("save state");
        state.clear_tokens();
        store.save(&state).expect("save logged-out state");
        let loaded = store.load().expect("load state");
        assert_eq!(loaded.signing_private_key.as_deref(), Some("private"));
        assert_eq!(loaded.account_data_key.as_deref(), Some("data"));
        assert!(!loaded.is_logged_in());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&store.path)
                .expect("state metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn enable_attaches_to_a_discovered_daemon_and_starts_one_when_absent() {
        assert_eq!(
            connector_activation(true, true),
            ConnectorActivation::Attach
        );
        assert_eq!(
            connector_activation(false, false),
            ConnectorActivation::StartDaemon
        );
        assert_eq!(
            connector_activation(true, false),
            ConnectorActivation::StartDaemon
        );
    }

    #[test]
    fn service_errors_do_not_echo_untrusted_response_bodies() {
        let error = service_error(
            StatusCode::BAD_REQUEST,
            br#"{"code":"invalid_request","message":"safe message","secret":"do-not-log"}"#,
        );
        assert!(error.contains("invalid_request"));
        assert!(error.contains("safe message"));
        assert!(!error.contains("do-not-log"));
    }

    #[test]
    fn sequence_reservations_are_serialized_across_state_store_instances() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("anywhere/state.json");
        StateStore { path: path.clone() }
            .save(&LocalState {
                version: STATE_VERSION,
                ..LocalState::default()
            })
            .expect("save state");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let barrier = barrier.clone();
            let path = path.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                StateStore { path }
                    .reserve_sequences(1)
                    .expect("reserve sequence")
                    .1
            }));
        }
        let mut sequences = threads
            .into_iter()
            .map(|thread| thread.join().expect("reservation thread"))
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        assert_eq!(sequences, (0_u64..8).collect::<Vec<_>>());
        assert_eq!(
            StateStore { path }
                .load()
                .expect("load state")
                .next_sequence,
            8
        );
    }
}
