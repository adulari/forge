//! Durable local Anywhere state and crash-safe persistence.

use super::*;

pub(crate) const STATE_VERSION: u8 = 1;
pub(crate) const KEY_EPOCH_INITIAL: u32 = 1;
pub(crate) const PAIRING_VERSION: u8 = 1;
pub(crate) const PAIRING_LIFETIME: Duration = Duration::from_secs(10 * 60);
pub(crate) const PAIRING_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize, Default)]
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
    pub(crate) fn is_logged_in(&self) -> bool {
        self.refresh_token.is_some()
    }

    pub(crate) fn clear_tokens(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.access_expires_at_ms = None;
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
        Ok(Self { path })
    }

    pub(crate) fn load(&self) -> Result<LocalState> {
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

    pub(crate) fn save(&self, state: &LocalState) -> Result<()> {
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
        let result = operation();
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
