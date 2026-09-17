//! Durable local Anywhere state and crash-safe persistence.

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
        if let Some(parent) = path.parent() {
            sweep_stale_temp_files(parent);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
