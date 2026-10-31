//! Durable local Anywhere state and crash-safe persistence.

use super::key_blob::{KeyBlob, KeyBlobMismatch, KEY_BLOB_VERSION};
use super::*;

pub(crate) const STATE_VERSION: u8 = 1;
pub(crate) const KEY_EPOCH_INITIAL: u32 = 1;
pub(crate) const PAIRING_VERSION: u8 = 1;
pub(crate) const PAIRING_LIFETIME: Duration = Duration::from_secs(10 * 60);
pub(crate) const PAIRING_POLL_INTERVAL: Duration = Duration::from_secs(2);
pub(crate) const LINK_STALE_AFTER: Duration = Duration::from_secs(90);

/// A crashed or killed process's atomic-write temp file that outlives it is left alone this long
/// before `StateStore::platform` sweeps it — long enough that an in-flight write from a live
/// process is never mistaken for an orphan.
const STALE_TEMP_FILE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Removes its target path on drop unless [`disarm`](Self::disarm) is called first. Guarantees an
/// atomic-write temp file is cleaned up on any error between its creation and the rename that
/// installs it — not only the rename failure, which used to be the only path handled explicitly.
struct RemoveOnDrop {
    path: PathBuf,
    armed: bool,
}

impl RemoveOnDrop {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Builds a temp-file path in the pattern the sweep in [`sweep_stale_temp_files`] recognizes:
/// `.<prefix>-<pid>-<16 hex digits>.tmp`.
fn temp_path(parent: &Path, prefix: &str) -> PathBuf {
    parent.join(format!(
        ".{prefix}-{}-{:016x}.tmp",
        std::process::id(),
        rand::random::<u64>()
    ))
}

/// Sweeps `.state-*.tmp`, `.link-state-*.tmp`, and `.command-state-*.tmp` files left behind by a
/// process that crashed or was killed between creating its atomic-write temp file and renaming it
/// into place — some of which are full copies of `state.json`, which holds live credentials. Never
/// touches anything else in `dir`: real state files don't start with `.` or end in `.tmp`.
fn sweep_stale_temp_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(pid) = stale_temp_file_pid(name) else {
            continue;
        };
        let is_stale = !process_is_alive(pid) || older_than(&entry, STALE_TEMP_FILE_AFTER);
        if is_stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Parses the pid embedded in a temp file name matching one of the three atomic-write prefixes,
/// returning `None` for anything else so the sweep leaves it untouched.
fn stale_temp_file_pid(name: &str) -> Option<u32> {
    for prefix in [".state-", ".link-state-", ".command-state-"] {
        if let Some(rest) = name
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(".tmp"))
        {
            return rest.split('-').next()?.parse().ok();
        }
    }
    None
}

fn older_than(entry: &std::fs::DirEntry, age: Duration) -> bool {
    let Ok(metadata) = entry.metadata() else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|elapsed| elapsed > age)
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // Signal zero performs permission/liveness validation without delivering a signal.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LinkState {
    pub(crate) daemon_pid: u32,
    pub(crate) connected: bool,
    pub(crate) last_exchange_ms: u64,
    pub(crate) updated_at_ms: u64,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkHealth {
    Healthy { age: Duration },
    Stale { age: Duration },
    Disconnected,
    Unknown,
}

impl LinkState {
    pub(crate) fn health_at(&self, now_ms: u64, daemon_pid: Option<u32>) -> LinkHealth {
        if daemon_pid != Some(self.daemon_pid) || !self.connected {
            return LinkHealth::Disconnected;
        }
        if self.last_exchange_ms == 0 {
            return LinkHealth::Unknown;
        }
        let age = Duration::from_millis(now_ms.saturating_sub(self.last_exchange_ms));
        if age > LINK_STALE_AFTER {
            LinkHealth::Stale { age }
        } else {
            LinkHealth::Healthy { age }
        }
    }
}

pub(crate) struct LinkStateStore {
    path: PathBuf,
}

impl LinkStateStore {
    pub(crate) fn platform() -> Result<Self> {
        let path = forge_config::data_dir()
            .context("no Forge platform data directory is available")?
            .join("anywhere")
            .join("link-state.json");
        Ok(Self { path })
    }

    pub(crate) fn load(&self) -> Result<Option<LinkState>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("parse Forge Anywhere link state")
                .map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("read Forge Anywhere link state"),
        }
    }

    pub(crate) fn save(&self, state: &LinkState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("Anywhere link state path has no parent")?;
        std::fs::create_dir_all(parent).context("create Forge Anywhere state directory")?;
        set_owner_directory_permissions(parent)?;
        let temp = temp_path(parent, "link-state");
        let guard = RemoveOnDrop::new(temp.clone());
        let bytes = serde_json::to_vec(state).context("serialize Forge Anywhere link state")?;
        std::fs::write(&temp, bytes).context("write Forge Anywhere link state")?;
        set_owner_file_permissions(&temp)?;
        std::fs::rename(&temp, &self.path).context("install Forge Anywhere link state")?;
        guard.disarm();
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CommandState {
    pub(crate) updated_at_ms: u64,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

pub(crate) struct CommandStateStore {
    path: PathBuf,
}

impl CommandStateStore {
    pub(crate) fn platform() -> Result<Self> {
        let path = forge_config::data_dir()
            .context("no Forge platform data directory is available")?
            .join("anywhere")
            .join("command-state.json");
        Ok(Self { path })
    }

    pub(crate) fn load(&self) -> Result<Option<CommandState>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("parse Forge Anywhere command state")
                .map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("read Forge Anywhere command state"),
        }
    }

    pub(crate) fn save(&self, state: &CommandState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("Anywhere command state path has no parent")?;
        std::fs::create_dir_all(parent).context("create Forge Anywhere state directory")?;
        set_owner_directory_permissions(parent)?;
        let temp = temp_path(parent, "command-state");
        let guard = RemoveOnDrop::new(temp.clone());
        let bytes = serde_json::to_vec(state).context("serialize Forge Anywhere command state")?;
        std::fs::write(&temp, bytes).context("write Forge Anywhere command state")?;
        set_owner_file_permissions(&temp)?;
        std::fs::rename(&temp, &self.path).context("install Forge Anywhere command state")?;
        guard.disarm();
        Ok(())
    }
}

thread_local! {
    static STATE_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct StateLockMark;

impl StateLockMark {
    fn set() -> Self {
        STATE_LOCK_HELD.with(|held| held.set(true));
        Self
    }
}

impl Drop for StateLockMark {
    fn drop(&mut self) {
        STATE_LOCK_HELD.with(|held| held.set(false));
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub(crate) struct LocalState {
    pub(crate) version: u8,
    pub(crate) account_id: Option<String>,
    pub(crate) github_login: Option<String>,
    pub(crate) device_id: Option<String>,
    pub(crate) signing_private_key: Option<String>,
    pub(crate) exchange_private_key: Option<String>,
    pub(crate) account_data_key: Option<String>,
    pub(crate) key_epoch: Option<u32>,
    #[serde(default)]
    pub(crate) data_key_epochs: BTreeMap<u32, String>,
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) access_expires_at_ms: Option<u64>,
    pub(crate) host_id: Option<String>,
    #[serde(default)]
    pub(crate) next_sequence: u64,
    #[serde(default)]
    pub(crate) accepted_sequences: BTreeMap<String, u64>,
    #[serde(default)]
    pub(crate) command_journal: BTreeMap<String, CommandJournalEntry>,
    #[serde(default)]
    pub(crate) capsule_journal: BTreeMap<String, CapsuleJournalEntry>,
    #[serde(default)]
    pub(crate) capsule_replay: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) outgoing_handoffs: BTreeMap<String, OutgoingHandoffEntry>,
    /// Capsule IDs durably frozen before local export. No service request is permitted while an
    /// entry remains here, so crash recovery can safely unfreeze it.
    #[serde(default)]
    pub(crate) preparing_handoffs: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) refresh_lease_id: Option<String>,
    #[serde(default)]
    pub(crate) refresh_lease_until_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CapsuleJournalEntry {
    pub(crate) acknowledgement_envelope: String,
    pub(crate) idempotency_key: String,
    #[serde(default)]
    pub(crate) imported_session_id: Option<String>,
    #[serde(default)]
    pub(crate) worktree_path: Option<String>,
    #[serde(default)]
    pub(crate) acked_at_ms: Option<u64>,
    #[serde(default)]
    pub(crate) terminal_at_ms: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct OutgoingHandoffEntry {
    pub(crate) capsule_id: String,
    pub(crate) destination_host_id: String,
    pub(crate) destination_name: String,
    pub(crate) envelope_path: String,
    pub(crate) request: forge_anywhere_protocol::CapsuleReserveRequest,
    pub(crate) reserve_idempotency_key: String,
    pub(crate) complete_idempotency_key: String,
    pub(crate) cancel_idempotency_key: String,
    #[serde(default)]
    pub(crate) accepted_destination_session_id: Option<String>,
    pub(crate) created_at_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CommandJournalEntry {
    pub(crate) sender_device_id: String,
    pub(crate) key_epoch: u32,
    pub(crate) sequence: u64,
    pub(crate) created_at_ms: u64,
    pub(crate) expires_at_ms: u64,
    pub(crate) ciphertext_bytes: u64,
    #[serde(flatten)]
    pub(crate) state: CommandJournalState,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum CommandJournalState {
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
    fn has_key_material(&self) -> bool {
        !KeyBlob::from_state(self).is_empty()
    }

    pub(crate) fn is_logged_in(&self) -> bool {
        self.refresh_token.is_some()
    }

    pub(crate) fn clear_tokens(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.access_expires_at_ms = None;
    }
}

/// Whether this machine is signed in to Forge Anywhere (a refresh token is stored).
pub(crate) fn signed_in() -> bool {
    StateStore::platform()
        .and_then(|store| store.load())
        .is_ok_and(|state| state.is_logged_in())
}

/// State-file operations lock and fsync; on a multi-thread runtime hand the worker back to tokio
/// while they run instead of stalling every task parked on it.
pub(super) fn blocking_fs<T>(operation: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(operation)
        }
        _ => operation(),
    }
}

pub(crate) struct StateStore {
    pub(crate) path: PathBuf,
}

impl StateStore {
    pub(crate) fn platform() -> Result<Self> {
        let path = forge_config::data_dir()
            .context("no Forge platform data directory is available")?
            .join("anywhere")
            .join("state.json");
        if let Some(parent) = path.parent() {
            sweep_stale_temp_files(parent);
        }
        Ok(Self { path })
    }

    pub(crate) fn load(&self) -> Result<LocalState> {
        match self.load_inner(false) {
            // A save stores the keys and then installs the file, so a lock-free read can land
            // between the two. Under the lock no save is in flight, so a blob still ahead of the
            // file is a save that was cut off: it is rolled forward and the file repaired.
            Err(error)
                if error.is::<KeyBlobMismatch>() && !STATE_LOCK_HELD.with(std::cell::Cell::get) =>
            {
                self.with_exclusive_lock(|| {
                    let (state, rolled_forward) = self.load_inner(false)?;
                    if rolled_forward {
                        self.save(&state)?;
                    }
                    Ok(state)
                })
            }
            result => result.map(|(state, _)| state),
        }
    }

    /// Like [`load`](Self::load) but a key blob that does not belong to the state file yields the
    /// state without any keys instead of an error. For sign-in and sign-out, which replace or
    /// ignore the old keys and must stay possible when they cannot be read.
    pub(crate) fn load_lenient(&self) -> Result<LocalState> {
        match self.load() {
            Err(error) if error.is::<KeyBlobMismatch>() => {
                self.load_inner(true).map(|(state, _)| state)
            }
            result => result,
        }
    }

    /// [`update`](Self::update) over [`load_lenient`](Self::load_lenient).
    pub(crate) fn update_lenient<F>(&self, update: F) -> Result<LocalState>
    where
        F: FnOnce(&mut LocalState) -> Result<()>,
    {
        self.with_exclusive_lock(|| {
            let mut state = match self.load() {
                Err(error) if error.is::<KeyBlobMismatch>() => self.load_inner(true)?.0,
                result => result?,
            };
            update(&mut state)?;
            self.save(&state)?;
            Ok(state)
        })
    }

    /// Reads the state file and joins it with its keys. `skip_keys` leaves them out. The flag in
    /// the result says the keys were one epoch ahead of the file and the state was rolled forward.
    fn load_inner(&self, skip_keys: bool) -> Result<(LocalState, bool)> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((
                    LocalState {
                        version: STATE_VERSION,
                        ..LocalState::default()
                    },
                    false,
                ));
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
        let mut rolled_forward = false;
        if !skip_keys && !state.has_key_material() {
            let may_roll_forward = STATE_LOCK_HELD.with(std::cell::Cell::get);
            if let Some(blob) = self.load_key_blob(&state, may_roll_forward)? {
                if blob.version >= KEY_BLOB_VERSION && blob.key_epoch != state.key_epoch {
                    state.key_epoch = blob.key_epoch;
                    rolled_forward = true;
                }
                blob.apply(&mut state);
            }
        }
        if let (Some(epoch), Some(key)) = (state.key_epoch, state.account_data_key.clone()) {
            state.data_key_epochs.entry(epoch).or_insert(key);
        }
        Ok((state, rolled_forward))
    }

    fn key_slot(&self) -> String {
        let digest = Sha256::digest(self.path.to_string_lossy().as_bytes());
        format!("anywhere-keys:{}", hex::encode(&digest[..8]))
    }

    /// Every parseable copy of the key blob, read past the process-local cache: the file's
    /// `key_epoch` and the keys come from different stores, and another process may have moved
    /// both since this one last looked.
    fn key_blob_copies(&self) -> Vec<KeyBlob> {
        forge_config::secret_store::get_each(&self.key_slot())
            .iter()
            .filter_map(|raw| serde_json::from_str(raw).ok())
            .collect()
    }

    /// The key blob saved for the device and epoch `state` names. A store this process cannot
    /// reach may hold an older or newer copy, so the copy has to match; none matching is an error,
    /// never a guess. Blobs from before the binding existed are taken when nothing better does.
    fn load_key_blob(&self, state: &LocalState, may_roll_forward: bool) -> Result<Option<KeyBlob>> {
        let mut copies = self.key_blob_copies();
        if copies.is_empty() {
            return Ok(None);
        }
        let chosen = copies
            .iter()
            .position(|blob| blob.belongs_to(state))
            .or_else(|| copies.iter().position(|blob| blob.is_legacy_for(state)))
            .or_else(|| {
                may_roll_forward
                    .then(|| copies.iter().position(|blob| blob.is_ahead_of(state)))
                    .flatten()
            });
        match chosen {
            Some(index) => Ok(Some(copies.swap_remove(index))),
            None => {
                let held = copies
                    .iter()
                    .map(|blob| {
                        format!(
                            "device {} epoch {}",
                            blob.device_id.as_deref().unwrap_or("?"),
                            blob.key_epoch.map_or("?".to_string(), |e| e.to_string())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                Err(anyhow::Error::new(KeyBlobMismatch(format!(
                    "the Anywhere keys in the secret store ({held}) do not belong to this state (device {} epoch {}); refusing to use keys from another \
                     device or epoch. To recover, run `forge anywhere logout` and then \
                     `forge anywhere login`; synced data stays in your account",
                    state.device_id.as_deref().unwrap_or("?"),
                    state.key_epoch.map_or("?".to_string(), |e| e.to_string())
                ))))
            }
        }
    }

    /// Moves the Account Data Key epochs and the device private keys out of the plaintext state
    /// file into the secret store (OS keyring first, encrypted file second). Idempotent, and a
    /// no-op for a state file that no longer holds them. Falls back to leaving them in the file.
    pub(crate) fn migrate_keys_to_secret_store(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        self.with_exclusive_lock(|| {
            let state = self.load()?;
            if state.has_key_material()
                && (self.file_holds_keys()? || !self.key_blob_is_current(&state))
            {
                self.save(&state)?;
            }
            Ok(())
        })
    }

    /// Whether the secret store already holds a bound copy of this state's keys. Copies from
    /// before the binding carry no device or epoch, so they are rewritten once.
    fn key_blob_is_current(&self, state: &LocalState) -> bool {
        self.key_blob_copies()
            .iter()
            .any(|blob| blob.belongs_to(state))
    }

    fn file_holds_keys(&self) -> Result<bool> {
        let text = std::fs::read_to_string(&self.path).context("read Forge Anywhere state")?;
        let on_disk: LocalState =
            serde_json::from_str(&text).context("parse Forge Anywhere state")?;
        Ok(on_disk.has_key_material())
    }

    pub(crate) fn save(&self, state: &LocalState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("Anywhere state path has no parent")?;
        std::fs::create_dir_all(parent).context("create Forge Anywhere state directory")?;
        set_owner_directory_permissions(parent)?;

        let temp = temp_path(parent, "state");
        let guard = RemoveOnDrop::new(temp.clone());
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
        let stripped = self.strip_secret_keys(state);
        let state = stripped.as_ref().unwrap_or(state);
        let bytes = serde_json::to_vec_pretty(state).context("serialize Forge Anywhere state")?;
        file.write_all(&bytes)
            .context("write Forge Anywhere state")?;
        file.sync_all().context("sync Forge Anywhere state")?;
        drop(file);
        set_owner_file_permissions(&temp)?;
        std::fs::rename(&temp, &self.path).context("install Forge Anywhere state")?;
        guard.disarm();
        set_owner_file_permissions(&self.path)?;
        sync_directory(parent).context("sync Forge Anywhere state directory")
    }

    /// Stores the key material in the secret store and returns a copy of `state` without it, or
    /// `None` when the keys must stay in the file (nothing to move, or the store is unavailable).
    fn strip_secret_keys(&self, state: &LocalState) -> Option<LocalState> {
        let blob = KeyBlob::from_state(state);
        if blob.is_empty() {
            return None;
        }
        let slot = self.key_slot();
        let raw = serde_json::to_string(&blob).ok()?;
        let stored = forge_config::secret_store::set_all(&slot, &raw).is_ok()
            && forge_config::secret_store::get_each(&slot).contains(&raw);
        if !stored {
            tracing::warn!(
                "secret store unavailable; Anywhere device keys stay in the owner-only state file"
            );
            return None;
        }
        Some(LocalState {
            signing_private_key: None,
            exchange_private_key: None,
            account_data_key: None,
            data_key_epochs: BTreeMap::new(),
            ..state.clone()
        })
    }

    pub(crate) fn update<F>(&self, update: F) -> Result<LocalState>
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

    pub(crate) fn reserve_sequences(&self, count: usize) -> Result<(LocalState, u64)> {
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

    pub(crate) fn with_exclusive_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
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
        let mark = StateLockMark::set();
        let result = operation();
        drop(mark);
        fs2::FileExt::unlock(&lock).context("unlock Anywhere state")?;
        result
    }
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_owner_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .context("set owner-only Anywhere state permissions")
}

#[cfg(not(unix))]
pub(crate) fn set_owner_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_owner_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .context("set owner-only Anywhere directory permissions")
}

#[cfg(not(unix))]
pub(crate) fn set_owner_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed_state() -> LocalState {
        LocalState {
            version: STATE_VERSION,
            account_id: Some("acct".into()),
            signing_private_key: Some("signing-secret".into()),
            exchange_private_key: Some("exchange-secret".into()),
            account_data_key: Some("data-key-two".into()),
            key_epoch: Some(2),
            data_key_epochs: BTreeMap::from([
                (1, "data-key-one".into()),
                (2, "data-key-two".into()),
            ]),
            ..LocalState::default()
        }
    }

    #[test]
    fn keys_leave_the_state_file_but_load_back_whole() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore {
            path: dir.path().join("anywhere/state.json"),
        };
        store.save(&keyed_state()).unwrap();
        let on_disk = std::fs::read_to_string(&store.path).unwrap();
        for secret in [
            "signing-secret",
            "exchange-secret",
            "data-key-one",
            "data-key-two",
        ] {
            assert!(!on_disk.contains(secret), "{secret} leaked into state.json");
        }
        assert!(on_disk.contains("acct"));
        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.signing_private_key.as_deref(),
            Some("signing-secret")
        );
        assert_eq!(loaded.data_key_epochs.len(), 2);
        assert_eq!(loaded.key_epoch, Some(2));
    }

    fn store_in(dir: &tempfile::TempDir) -> StateStore {
        StateStore {
            path: dir.path().join("anywhere/state.json"),
        }
    }

    fn device_state(device: &str, epoch: u32, keys: &[(u32, &str)]) -> LocalState {
        LocalState {
            version: STATE_VERSION,
            account_id: Some("acct".into()),
            device_id: Some(device.into()),
            signing_private_key: Some(format!("signing-{device}")),
            exchange_private_key: Some(format!("exchange-{device}")),
            account_data_key: keys
                .iter()
                .find(|(e, _)| *e == epoch)
                .map(|(_, k)| k.to_string()),
            key_epoch: Some(epoch),
            data_key_epochs: keys.iter().map(|(e, k)| (*e, k.to_string())).collect(),
            ..LocalState::default()
        }
    }

    #[test]
    fn a_stale_cached_blob_is_never_attached_to_a_newer_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&device_state("dev", 1, &[(1, "K1")])).unwrap();
        let epoch_one_blob = forge_config::secret_store::get_each(&store.key_slot()).remove(0);

        // Another process revokes a device: epoch 2 and the new key land in state and keyring.
        store
            .update(|state| {
                state.key_epoch = Some(2);
                state.account_data_key = Some("K2".into());
                state.data_key_epochs.insert(2, "K2".into());
                Ok(())
            })
            .unwrap();
        // This process's read memo still answers with the epoch-1 blob it cached earlier.
        forge_config::secret_store::staging::pin_stale_read(
            &store.key_slot(),
            Some(&epoch_one_blob),
        );

        let loaded = store.load().unwrap();
        assert_eq!(loaded.key_epoch, Some(2));
        assert_eq!(loaded.account_data_key.as_deref(), Some("K2"));
        assert_eq!(
            loaded.data_key_epochs.get(&2).map(String::as_str),
            Some("K2"),
            "epoch 2 must map to the epoch-2 key, not the revoked one"
        );
        store.save(&loaded).unwrap();
        let after = store.load().unwrap();
        assert_eq!(after.account_data_key.as_deref(), Some("K2"));
        assert_eq!(
            after.data_key_epochs.get(&1).map(String::as_str),
            Some("K1")
        );
    }

    #[test]
    fn keys_saved_for_another_epoch_are_refused_instead_of_grafted() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&device_state("dev", 1, &[(1, "K1")])).unwrap();
        let epoch_one_blob = forge_config::secret_store::get_each(&store.key_slot()).remove(0);
        store
            .save(&device_state("dev", 2, &[(1, "K1"), (2, "K2")]))
            .unwrap();
        for write in [
            forge_config::secret_store::staging::write_keyring_only,
            forge_config::secret_store::staging::write_file_only,
        ] {
            write(&store.key_slot(), &epoch_one_blob);
        }

        let error = store.load().err().expect("a mismatched blob must not load");
        assert!(
            format!("{error:#}").contains("do not belong"),
            "a mismatch must be loud: {error:#}"
        );
    }

    #[test]
    fn keys_are_readable_by_a_process_that_cannot_reach_the_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&device_state("dev", 1, &[(1, "K1")])).unwrap();

        forge_config::secret_store::staging::set_keyring_reachable(&store.key_slot(), false);
        let loaded = store.load().unwrap();
        forge_config::secret_store::staging::set_keyring_reachable(&store.key_slot(), true);

        assert_eq!(loaded.signing_private_key.as_deref(), Some("signing-dev"));
        assert_eq!(loaded.account_data_key.as_deref(), Some("K1"));
    }

    #[test]
    fn a_rekeyed_device_wins_over_the_stale_copy_left_in_the_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&device_state("old", 1, &[(1, "OLD")])).unwrap();

        // Logout then login from a shell with no keyring: the new keys reach only the file.
        let slot = store.key_slot();
        forge_config::secret_store::staging::set_keyring_reachable(&slot, false);
        store.save(&device_state("new", 1, &[(1, "NEW")])).unwrap();
        forge_config::secret_store::staging::set_keyring_reachable(&slot, true);

        let loaded = store.load().unwrap();
        assert_eq!(loaded.device_id.as_deref(), Some("new"));
        assert_eq!(loaded.signing_private_key.as_deref(), Some("signing-new"));
        assert_eq!(loaded.account_data_key.as_deref(), Some("NEW"));
    }

    #[test]
    fn a_blob_from_before_the_binding_still_loads_and_is_rewritten_bound() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&device_state("dev", 1, &[(1, "K1")])).unwrap();
        let slot = store.key_slot();
        let legacy = serde_json::json!({
            "signing_private_key": "signing-dev",
            "exchange_private_key": "exchange-dev",
            "account_data_key": "K1",
            "data_key_epochs": { "1": "K1" },
        })
        .to_string();
        forge_config::secret_store::set_all(&slot, &legacy).unwrap();

        assert_eq!(
            store.load().unwrap().account_data_key.as_deref(),
            Some("K1")
        );
        assert!(!store.key_blob_is_current(&store.load().unwrap()));
        store.migrate_keys_to_secret_store().unwrap();
        assert!(store.key_blob_is_current(&store.load().unwrap()));
    }

    fn blob_json(
        device: Option<&str>,
        epoch: Option<u32>,
        keys: &[(u32, &str)],
        current: &str,
    ) -> String {
        let mut blob = serde_json::json!({
            "signing_private_key": "signing-dev",
            "exchange_private_key": "exchange-dev",
            "account_data_key": current,
            "data_key_epochs": keys.iter().map(|(e, k)| (e.to_string(), *k)).collect::<BTreeMap<_, _>>(),
        });
        if let Some(epoch) = epoch {
            blob["version"] = 1.into();
            blob["key_epoch"] = epoch.into();
            blob["device_id"] = device.into();
        }
        blob.to_string()
    }

    #[test]
    fn a_stale_legacy_copy_never_wins_over_the_legacy_copy_holding_the_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store
            .save(&device_state("dev", 2, &[(1, "K1"), (2, "K2")]))
            .unwrap();
        let slot = store.key_slot();
        // Desktop keyring kept the epoch-1 blob; an SSH shell wrote epoch 2 to the file.
        forge_config::secret_store::staging::write_keyring_only(
            &slot,
            &blob_json(None, None, &[(1, "K1")], "K1"),
        );
        forge_config::secret_store::staging::write_file_only(
            &slot,
            &blob_json(None, None, &[(1, "K1"), (2, "K2")], "K2"),
        );

        let loaded = store.load().unwrap();
        assert_eq!(loaded.account_data_key.as_deref(), Some("K2"));
        store.migrate_keys_to_secret_store().unwrap();
        for copy in store.key_blob_copies() {
            assert_eq!(
                copy.account_data_key.as_deref(),
                Some("K2"),
                "K2 was destroyed"
            );
            assert_eq!(copy.data_key_epochs.get(&2).map(String::as_str), Some("K2"));
        }
    }

    #[test]
    fn a_legacy_copy_without_the_epoch_is_refused_not_grafted() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store
            .save(&device_state("dev", 2, &[(1, "K1"), (2, "K2")]))
            .unwrap();
        let slot = store.key_slot();
        let stale = blob_json(None, None, &[(1, "K1")], "K1");
        forge_config::secret_store::staging::write_keyring_only(&slot, &stale);
        forge_config::secret_store::staging::write_file_only(&slot, &stale);

        assert!(store.load().is_err(), "K1 must not be filed under epoch 2");
    }

    #[test]
    fn a_save_cut_off_after_the_keys_were_stored_rolls_forward_on_the_next_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&device_state("dev", 1, &[(1, "K1")])).unwrap();
        // Crash point: the epoch-2 keys reached the secret store, state.json never moved.
        assert!(store
            .strip_secret_keys(&device_state("dev", 2, &[(1, "K1"), (2, "K2")]))
            .is_some());
        let on_disk: LocalState =
            serde_json::from_str(&std::fs::read_to_string(&store.path).unwrap()).unwrap();
        assert_eq!(on_disk.key_epoch, Some(1));

        let loaded = store.load().unwrap();
        assert_eq!(loaded.key_epoch, Some(2));
        assert_eq!(loaded.account_data_key.as_deref(), Some("K2"));
        assert_eq!(
            loaded.data_key_epochs.get(&1).map(String::as_str),
            Some("K1")
        );
        let repaired: LocalState =
            serde_json::from_str(&std::fs::read_to_string(&store.path).unwrap()).unwrap();
        assert_eq!(
            repaired.key_epoch,
            Some(2),
            "the file is repaired on the way"
        );
        assert_eq!(
            store.load().unwrap().account_data_key.as_deref(),
            Some("K2")
        );
    }

    #[test]
    fn a_cut_off_re_enrolment_leaves_sign_out_and_sign_in_possible() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        let mut old = device_state("old", 1, &[(1, "OLD")]);
        old.refresh_token = Some("refresh".into());
        store.save(&old).unwrap();
        // Crash point: the new device's keys were stored, state.json still names the old device.
        assert!(store
            .strip_secret_keys(&device_state("new", 1, &[(1, "NEW")]))
            .is_some());

        let error = store
            .load()
            .err()
            .expect("another device's keys must not load");
        let text = format!("{error:#}");
        assert!(text.contains("forge anywhere logout"), "{text}");

        let lenient = store.load_lenient().unwrap();
        assert_eq!(lenient.refresh_token.as_deref(), Some("refresh"));
        assert!(lenient.signing_private_key.is_none());
        let out = store
            .update_lenient(|state| {
                state.clear_tokens();
                Ok(())
            })
            .unwrap();
        assert!(!out.is_logged_in());

        // Sign-in replaces the state wholesale and the new keys load again.
        store.save(&device_state("new", 1, &[(1, "NEW")])).unwrap();
        assert_eq!(
            store.load().unwrap().signing_private_key.as_deref(),
            Some("signing-new")
        );
    }

    #[test]
    fn the_old_plaintext_layout_still_loads_and_migrates_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore {
            path: dir.path().join("anywhere/state.json"),
        };
        std::fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        std::fs::write(&store.path, serde_json::to_vec(&keyed_state()).unwrap()).unwrap();
        assert_eq!(
            store.load().unwrap().exchange_private_key.as_deref(),
            Some("exchange-secret"),
            "the legacy layout must stay readable before migration"
        );
        store.migrate_keys_to_secret_store().unwrap();
        assert!(!std::fs::read_to_string(&store.path)
            .unwrap()
            .contains("exchange-secret"));
        store.migrate_keys_to_secret_store().unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.exchange_private_key.as_deref(),
            Some("exchange-secret")
        );
        assert_eq!(
            loaded.data_key_epochs.get(&1).map(String::as_str),
            Some("data-key-one")
        );
    }

    #[test]
    fn remove_on_drop_cleans_up_unless_disarmed() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join(".state-1-0000000000000000.tmp");

        std::fs::write(&path, b"partial write").expect("write temp file");
        drop(RemoveOnDrop::new(path.clone()));
        assert!(
            !path.exists(),
            "an armed guard removes its temp file on drop"
        );

        std::fs::write(&path, b"partial write").expect("recreate temp file");
        RemoveOnDrop::new(path.clone()).disarm();
        assert!(path.exists(), "a disarmed guard leaves its temp file alone");
    }

    #[test]
    fn save_removes_its_temp_file_when_the_final_rename_fails() {
        // Renaming a regular file onto an existing, non-empty path that is itself a directory is
        // a deterministic, portable way to force the rename step of `save` to fail without
        // relying on filesystem permission tricks.
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let blocked = temp_dir.path().join("blocked");
        std::fs::create_dir(&blocked).expect("create blocking directory");
        let store = StateStore { path: blocked };

        let result = store.save(&LocalState {
            version: STATE_VERSION,
            ..LocalState::default()
        });
        assert!(result.is_err(), "renaming onto a directory must fail");

        let leftovers: Vec<_> = std::fs::read_dir(temp_dir.path())
            .expect("read temp dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".state-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "a failed rename must not leak its temp file: {leftovers:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn sweep_removes_temp_files_whose_pid_is_dead_but_leaves_live_ones_and_real_state_alone() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let dir = temp_dir.path();

        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn short-lived helper process");
        let dead_pid = child.id();
        child.wait().expect("wait for helper process to exit");

        let dead = dir.join(format!(".state-{dead_pid}-aaaaaaaaaaaaaaaa.tmp"));
        std::fs::write(&dead, b"leaked credentials").expect("write dead-pid temp file");

        let live_pid = std::process::id();
        let live = dir.join(format!(".command-state-{live_pid}-bbbbbbbbbbbbbbbb.tmp"));
        std::fs::write(&live, b"in flight").expect("write live-pid temp file");

        let real_state = dir.join("state.json");
        std::fs::write(&real_state, b"{}").expect("write real state file");
        let lock = dir.join("state.lock");
        std::fs::write(&lock, b"").expect("write lock file");

        sweep_stale_temp_files(dir);

        assert!(
            !dead.exists(),
            "a temp file for a dead process must be swept"
        );
        assert!(
            live.exists(),
            "a temp file for a live, recent process must be left alone"
        );
        assert!(
            real_state.exists(),
            "sweep must never touch the real state file"
        );
        assert!(lock.exists(), "sweep must never touch the lock file");
    }

    #[test]
    fn stale_temp_file_pid_ignores_files_outside_the_three_known_patterns() {
        assert_eq!(stale_temp_file_pid("state.json"), None);
        assert_eq!(stale_temp_file_pid("state.lock"), None);
        assert_eq!(stale_temp_file_pid("outgoing-jobs.json"), None);
        assert_eq!(
            stale_temp_file_pid(".state-123-aaaaaaaaaaaaaaaa.tmp"),
            Some(123)
        );
        assert_eq!(
            stale_temp_file_pid(".link-state-456-bbbbbbbbbbbbbbbb.tmp"),
            Some(456)
        );
        assert_eq!(
            stale_temp_file_pid(".command-state-789-cccccccccccccccc.tmp"),
            Some(789)
        );
    }
}
