//! Persistent secret storage with an OS-keyring-first, encrypted-file-fallback strategy.
//!
//! The OS keyring is preferred (macOS Keychain, Windows Credential Manager, Linux Secret
//! Service). But it isn't always reachable: a headless box, or a Linux session where no
//! `org.freedesktop.secrets` provider is activatable, makes every keyring call fail — and the
//! kernel-keyutils backend that *does* always work is wiped on logout/reboot (the "keyring keeps
//! resetting, I had to re-enter my API keys" bug). So when the keyring is unavailable we fall
//! back to an encrypted file under the config dir: AEAD (ChaCha20-Poly1305) with a random key in
//! a sibling `0600` keyfile. That persists across reboots regardless of any daemon.
//!
//! Every public entry point here routes through [`get`]/[`set`]/[`delete`], which try the keyring
//! and transparently fall back to the file, so callers (provider keys, search keys, MCP tokens,
//! OAuth tokens) get durable storage without caring which backend answered.
//!
//! # Test isolation
//!
//! Two independent layers stop tests from ever touching real secrets:
//!
//! 1. **`test-secrets` feature** — swaps [`get`]/[`set`]/[`delete`] for a process-local
//!    `Mutex<BTreeMap>` so the real keyring/file path is not reached. Enabled from dependents'
//!    `[dev-dependencies]` (resolver=2: does not leak into production builds).
//! 2. **Runtime tripwire** — on the real path, panics if `current_exe` looks like a cargo
//!    test/bench binary (`…/target/…/deps/…`), unless `FORGE_ALLOW_REAL_SECRETS=1`.
//!
//! The feature has one side effect worth knowing about: `cargo test` (and `--all-targets`) builds
//! the `forge` bin for the integration tests' `CARGO_BIN_EXE_forge` with the dev-dependency
//! features unified in, and uplifts that bin to `target/debug/forge`. Until the next
//! `cargo build --bin forge`, that path is a binary whose secret store is this in-memory map:
//! every `forge auth --list` says "no keys configured" and `forge models`/`forge mesh` see only
//! keyless providers. It is not a debug-vs-release difference — a plain `cargo build` in either
//! profile reads the same keyring + file as the release binary. [`backend`] tells which store a
//! process is reading so the commands can say so.

#[cfg(any(not(feature = "test-secrets"), test))]
use std::path::PathBuf;
use std::path::{Component, Path};

#[cfg(not(feature = "test-secrets"))]
use std::collections::BTreeMap;
#[cfg(not(feature = "test-secrets"))]
use std::io::Write as _;

#[cfg(not(feature = "test-secrets"))]
use base64::Engine as _;
#[cfg(not(feature = "test-secrets"))]
use chacha20poly1305::aead::{Aead, KeyInit};
#[cfg(not(feature = "test-secrets"))]
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::ConfigError;

#[cfg(not(feature = "test-secrets"))]
const KEYRING_SERVICE: &str = "forge";

/// Escape hatch for the runtime tripwire: set to `1` only when a test deliberately needs the
/// real keyring/file store (almost never).
#[cfg(not(feature = "test-secrets"))]
const ALLOW_REAL_SECRETS_ENV: &str = "FORGE_ALLOW_REAL_SECRETS";

// =============================================================================================
// Public API — feature-gated between in-memory (tests) and real backends (production)
// =============================================================================================

/// Which store answers [`get`]/[`set`]/[`delete`] in this process. Reported by
/// `forge auth --list`, `forge models`, `forge mesh` and `forge doctor` so an empty store is never
/// mistaken for a missing key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretBackend {
    /// The `test-secrets` process-local map: nothing on disk or in the OS keyring is visible.
    InMemoryTest,
    /// The OS keyring (service `forge`) answered the probe; `fallback` is the encrypted file that
    /// is consulted after it.
    OsKeyring {
        fallback: Option<std::path::PathBuf>,
    },
    /// The OS keyring is unreachable; the encrypted file is the only store.
    EncryptedFile(Option<std::path::PathBuf>),
}

impl SecretBackend {
    /// One human-readable line naming the store and, for the test map, why it is empty.
    pub fn describe(&self) -> String {
        let file = |p: &Option<std::path::PathBuf>| match p {
            Some(p) => p.display().to_string(),
            None => "<no config dir>".to_string(),
        };
        match self {
            SecretBackend::InMemoryTest => "in-memory test store — this binary was compiled with \
                 the `test-secrets` feature (a `forge` built by `cargo test` or `--all-targets`), \
                 so NO stored key is visible; rebuild with `cargo build --bin forge`"
                .to_string(),
            SecretBackend::OsKeyring { fallback } => {
                format!(
                    "OS keyring (service `forge`), then encrypted file {}",
                    file(fallback)
                )
            }
            SecretBackend::EncryptedFile(path) => {
                format!("encrypted file {} (OS keyring unreachable)", file(path))
            }
        }
    }

    /// Whether reads can never see a stored key, whatever the user configured.
    pub fn is_blind(&self) -> bool {
        matches!(self, SecretBackend::InMemoryTest)
    }
}

/// The store this process reads. Runs the (cached, bounded) keyring probe on the real path.
#[cfg(feature = "test-secrets")]
pub fn backend() -> SecretBackend {
    SecretBackend::InMemoryTest
}

/// The store this process reads. Runs the (cached, bounded) keyring probe on the real path.
#[cfg(not(feature = "test-secrets"))]
pub fn backend() -> SecretBackend {
    if keyring_available() {
        SecretBackend::OsKeyring {
            fallback: secrets_path(),
        }
    } else {
        SecretBackend::EncryptedFile(secrets_path())
    }
}

/// Store `value` under `key`.
#[cfg(feature = "test-secrets")]
pub fn set(key: &str, value: &str) -> Result<(), ConfigError> {
    memory::set(key, value)
}

/// Read the secret for `key`.
#[cfg(feature = "test-secrets")]
pub fn get(key: &str) -> Option<String> {
    memory::get(key)
}

/// Remove `key`. `Ok(true)` if something was removed, `Ok(false)` if nothing was stored.
#[cfg(feature = "test-secrets")]
pub fn delete(key: &str) -> Result<bool, ConfigError> {
    memory::delete(key)
}

/// Store `value` under `key`: OS keyring first, encrypted file on keyring failure/unavailability.
#[cfg(not(feature = "test-secrets"))]
pub fn set(key: &str, value: &str) -> Result<(), ConfigError> {
    refuse_if_test_binary();
    read_cache().invalidate(key);
    let result = set_inner(key, value);
    if result.is_ok() {
        touch_change_stamp();
    }
    result
}

#[cfg(not(feature = "test-secrets"))]
fn set_inner(key: &str, value: &str) -> Result<(), ConfigError> {
    if keyring_available() {
        let key = key.to_string();
        let value = value.to_string();
        if matches!(
            keyring_call("write", move || {
                keyring::Entry::new(KEYRING_SERVICE, &key)
                    .and_then(|entry| entry.set_password(&value))
            }),
            Some(Ok(()))
        ) {
            return Ok(());
        }
    }
    file_set(key, value)
}

/// Read the secret for `key`: env-independent. Keyring first, then the encrypted file.
///
/// Memoized for [`READ_CACHE_TTL`]: one startup asks for the same ~30 provider entries five times
/// over (discovery, key injection, routing, quota refreshes), and on a live Secret Service every
/// miss is a D-Bus round trip — 138 reads cost ~0.9 s before a single model was even considered.
#[cfg(not(feature = "test-secrets"))]
pub fn get(key: &str) -> Option<String> {
    refuse_if_test_binary();
    let now = std::time::Instant::now();
    if let Some(hit) = read_cache().lookup(key, now) {
        return hit;
    }
    let value = get_uncached(key);
    read_cache().store(key, value.clone(), now);
    value
}

#[cfg(not(feature = "test-secrets"))]
fn get_uncached(key: &str) -> Option<String> {
    if keyring_available() {
        let key = key.to_string();
        if let Some(Ok(v)) = keyring_call("read", move || {
            keyring::Entry::new(KEYRING_SERVICE, &key).and_then(|entry| entry.get_password())
        }) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    file_get(key)
}

#[cfg(not(feature = "test-secrets"))]
fn read_cache() -> std::sync::MutexGuard<'static, ReadCache> {
    static CACHE: std::sync::Mutex<ReadCache> = std::sync::Mutex::new(ReadCache::new());
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// File whose mtime moves whenever any Forge process stores or deletes a secret. The keyring has no
/// change notification, and re-reading ~150 entries to notice a new key would be a D-Bus storm, so
/// writers leave this one-stat signal for long-running sessions to poll.
fn change_stamp_path() -> Option<std::path::PathBuf> {
    crate::config_dir().map(|d| d.join("secrets.stamp"))
}

#[cfg(not(feature = "test-secrets"))]
fn touch_change_stamp() {
    if let Some(path) = change_stamp_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let _ = std::fs::write(path, nanos.to_string());
    }
}

/// Modification time of the secret-change stamp: `None` until some process has written a secret.
/// One `stat`; safe to poll.
pub fn change_stamp() -> Option<std::time::SystemTime> {
    std::fs::metadata(change_stamp_path()?)
        .and_then(|m| m.modified())
        .ok()
}

/// How long a secret read (hit or miss) is trusted before the backends are asked again. Long
/// enough to cover one command's startup burst; short enough that a key added by `forge auth` in
/// another process reaches a long-running `forge serve` on its next turn, not its next restart.
#[cfg_attr(feature = "test-secrets", allow(dead_code))]
const READ_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

/// Process-local memo of recent secret reads. Pure over an injected clock so the TTL and
/// invalidation rules are unit-testable without a keyring.
#[derive(Debug, Default)]
#[cfg_attr(feature = "test-secrets", allow(dead_code))]
pub(crate) struct ReadCache {
    entries: Vec<(String, Option<String>, std::time::Instant)>,
}

#[cfg_attr(feature = "test-secrets", allow(dead_code))]
impl ReadCache {
    pub(crate) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// `Some(value)` when `key` was read within the TTL (the value itself may be `None`: a
    /// confirmed miss is cached too — that is where the startup savings are).
    pub(crate) fn lookup(&self, key: &str, now: std::time::Instant) -> Option<Option<String>> {
        self.entries
            .iter()
            .find(|(k, _, at)| k == key && now.duration_since(*at) < READ_CACHE_TTL)
            .map(|(_, v, _)| v.clone())
    }

    pub(crate) fn store(&mut self, key: &str, value: Option<String>, now: std::time::Instant) {
        self.invalidate(key);
        self.entries.push((key.to_string(), value, now));
    }

    /// A write or delete through this process must be visible to its very next read.
    pub(crate) fn invalidate(&mut self, key: &str) {
        self.entries.retain(|(k, _, _)| k != key);
    }
}

/// Remove `key` from wherever it lives. `Ok(true)` if something was removed (from either store),
/// `Ok(false)` if nothing was stored — so removal stays idempotent.
#[cfg(not(feature = "test-secrets"))]
pub fn delete(key: &str) -> Result<bool, ConfigError> {
    refuse_if_test_binary();
    read_cache().invalidate(key);
    let result = delete_inner(key);
    if result.is_ok() {
        touch_change_stamp();
    }
    result
}

#[cfg(not(feature = "test-secrets"))]
fn delete_inner(key: &str) -> Result<bool, ConfigError> {
    let mut removed = false;
    if keyring_available() {
        let key = key.to_string();
        match keyring_call("delete", move || {
            keyring::Entry::new(KEYRING_SERVICE, &key).and_then(|entry| entry.delete_credential())
        }) {
            Some(Ok(())) => removed = true,
            Some(Err(keyring::Error::NoEntry)) => {}
            Some(Err(_)) | None => {} // unreachable/timed out — fall through to the file store
        }
    }
    removed |= file_delete(key)?;
    Ok(removed)
}

// =============================================================================================
// In-memory backend (`test-secrets` feature) — never constructs a keyring::Entry
// =============================================================================================

#[cfg(feature = "test-secrets")]
mod memory {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use crate::ConfigError;

    fn map() -> &'static Mutex<BTreeMap<String, String>> {
        static MAP: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
        &MAP
    }

    pub(super) fn set(key: &str, value: &str) -> Result<(), ConfigError> {
        map()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    pub(super) fn get(key: &str) -> Option<String> {
        map()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    pub(super) fn delete(key: &str) -> Result<bool, ConfigError> {
        Ok(map()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key)
            .is_some())
    }
}

// =============================================================================================
// Runtime tripwire (real path only) — panic if a cargo test binary reaches here
// =============================================================================================

/// Whether `exe` looks like a cargo unit/integration test or bench binary: it lives under
/// `…/target/…/deps/…`. The shipped `forge` binary and `cargo run` output live under
/// `target/<profile>/forge` (no `deps` segment) and return false.
pub fn is_cargo_test_or_bench_binary(exe: &Path) -> bool {
    let mut after_target = false;
    for c in exe.components() {
        if let Component::Normal(name) = c {
            if name == "target" {
                after_target = true;
            } else if after_target && name == "deps" {
                return true;
            }
        }
    }
    false
}

/// Panic if this process is a cargo test/bench binary, unless `FORGE_ALLOW_REAL_SECRETS=1`.
#[cfg(not(feature = "test-secrets"))]
fn refuse_if_test_binary() {
    if std::env::var_os(ALLOW_REAL_SECRETS_ENV).is_some_and(|v| v == "1") {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if is_cargo_test_or_bench_binary(&exe) {
        panic!(
            "tests must not touch real secrets (secret_store) — enable the test-secrets feature; \
             set {ALLOW_REAL_SECRETS_ENV}=1 only if you truly mean it"
        );
    }
}

// =============================================================================================
// Real backends (keyring + encrypted file) — only reached when `test-secrets` is off
// =============================================================================================

/// Max time to wait for the OS keyring backend to answer a probe before declaring it unusable for
/// the session. Generous enough for a slow-but-live Secret Service, short enough that a *dead* one
/// (the WSL/headless case) doesn't stall startup.
#[cfg(not(feature = "test-secrets"))]
const KEYRING_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(800);

#[cfg(any(not(feature = "test-secrets"), test))]
fn call_with_timeout<T: Send + 'static>(
    timeout: std::time::Duration,
    call: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(call());
    });
    rx.recv_timeout(timeout).ok()
}

#[cfg(not(feature = "test-secrets"))]
static KEYRING_TIMED_OUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Run every real keyring operation under the same bound as the availability probe. A Secret
/// Service can answer the initial probe and still wedge a later credential lookup; after the first
/// such timeout, disable it for this process so bulk provider-key injection leaks at most one
/// blocked worker and immediately falls back to the encrypted file store.
#[cfg(not(feature = "test-secrets"))]
fn keyring_call<T: Send + 'static>(
    operation: &str,
    call: impl FnOnce() -> Result<T, keyring::Error> + Send + 'static,
) -> Option<Result<T, keyring::Error>> {
    let result = call_with_timeout(KEYRING_PROBE_TIMEOUT, call);
    if result.is_none() && !KEYRING_TIMED_OUT.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!(
            "OS keyring {operation} did not respond within {}ms — disabling it for this session",
            KEYRING_PROBE_TIMEOUT.as_millis()
        );
    }
    result
}

/// Whether the OS keyring backend is reachable — probed ONCE, with a timeout, and cached for the
/// process. This exists because on some boxes (WSL / headless Linux with an activatable-but-dead
/// `org.freedesktop.secrets`) a keyring call **blocks forever** instead of returning an error,
/// which hung `forge chat` before the TUI ever drew its first frame. We run the probe on a detached
/// thread and wait at most [`KEYRING_PROBE_TIMEOUT`]; if it doesn't answer we treat the keyring as
/// unavailable for the whole session and use the encrypted file store exclusively. A box with a
/// live keyring answers in milliseconds, so this is invisible there.
#[cfg(not(feature = "test-secrets"))]
fn keyring_available() -> bool {
    if KEYRING_TIMED_OUT.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let available = *AVAILABLE.get_or_init(|| {
        match call_with_timeout(KEYRING_PROBE_TIMEOUT, || {
            // Any return (Ok OR Err) means the backend ANSWERED within the window — real calls will
            // then also return promptly (and fall back to the file on their own Err). Only a true
            // hang never sends, tripping the recv timeout below. The detached thread is left to
            // unblock on its own rather than wedging the main path.
            keyring::Entry::new(KEYRING_SERVICE, "__forge_probe__")
                .map(|entry| entry.get_password())
        }) {
            Some(_) => true,
            None => {
                tracing::warn!(
                    "OS keyring did not respond within {}ms — using the encrypted file store for \
                     this session (secrets are still durable)",
                    KEYRING_PROBE_TIMEOUT.as_millis()
                );
                false
            }
        }
    });
    available && !KEYRING_TIMED_OUT.load(std::sync::atomic::Ordering::Relaxed)
}

// --- encrypted file fallback ------------------------------------------------------------------

#[cfg(not(feature = "test-secrets"))]
fn secrets_path() -> Option<PathBuf> {
    crate::config_dir().map(|d| d.join("secrets.enc"))
}

#[cfg(not(feature = "test-secrets"))]
fn keyfile_path() -> Option<PathBuf> {
    crate::config_dir().map(|d| d.join("secret.key"))
}

/// Load (or create) the 32-byte file-store key. Stored `0600` next to the encrypted blob.
#[cfg(not(feature = "test-secrets"))]
fn load_or_create_key() -> Result<Key, ConfigError> {
    let path = keyfile_path().ok_or_else(|| ConfigError::Keyring("no config dir".into()))?;
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            return Key::try_from(bytes.as_slice())
                .map_err(|e| ConfigError::Keyring(e.to_string()));
        }
        // A truncated/corrupt key file is NOT "no key": silently minting a new one would make
        // every stored secret undecryptable. Make the user decide.
        if !bytes.is_empty() {
            return Err(ConfigError::Keyring(format!(
                "{} is {} bytes, expected 32 — it looks corrupted. Restore it from a backup, or \
                 delete it (and secrets.enc) to start over; re-add keys with `forge auth`.",
                path.display(),
                bytes.len()
            )));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    }
    let raw: [u8; 32] = rand::random();
    write_private(&path, &raw)?;
    Key::try_from(raw.as_slice()).map_err(|e| ConfigError::Keyring(e.to_string()))
}

/// Write a file readable/writable only by the owner (`0600` on Unix).
#[cfg(not(feature = "test-secrets"))]
fn write_private(path: &PathBuf, bytes: &[u8]) -> Result<(), ConfigError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    // Write beside the target and rename over it: truncating in place leaves a corrupt file if the
    // process dies mid-write.
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let mut f = opts
        .open(&tmp)
        .map_err(|e| ConfigError::Keyring(e.to_string()))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| ConfigError::Keyring(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    Ok(())
}

#[cfg(not(feature = "test-secrets"))]
fn cipher() -> Result<ChaCha20Poly1305, ConfigError> {
    Ok(ChaCha20Poly1305::new(&load_or_create_key()?))
}

/// The on-disk map is `name -> base64(nonce ‖ ciphertext)`.
#[cfg(not(feature = "test-secrets"))]
fn read_map() -> BTreeMap<String, String> {
    secrets_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Like [`read_map`] but an unreadable or corrupt store is an error, never "empty": the write
/// paths rebuild the file from this map, so treating garbage as empty destroyed every other key.
#[cfg(not(feature = "test-secrets"))]
fn read_map_strict() -> Result<BTreeMap<String, String>, ConfigError> {
    let Some(path) = secrets_path() else {
        return Ok(BTreeMap::new());
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            ConfigError::Keyring(format!(
                "{} is corrupt ({e}); refusing to overwrite it. Move it aside to start over",
                path.display()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(ConfigError::Keyring(format!(
            "reading {}: {e}",
            path.display()
        ))),
    }
}

#[cfg(not(feature = "test-secrets"))]
fn write_map(map: &BTreeMap<String, String>) -> Result<(), ConfigError> {
    let path = secrets_path().ok_or_else(|| ConfigError::Keyring("no config dir".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    }
    let body = serde_json::to_vec_pretty(map).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    write_private(&path, &body)
}

/// Serialize read-modify-write of `secrets.enc` across processes (two `forge auth` runs, the
/// daemon): an exclusive `flock` on a sibling lock file, polled for up to 10s. Best effort — if the
/// lock cannot be taken the write proceeds unlocked, as before.
#[cfg(not(feature = "test-secrets"))]
fn with_store_lock<T>(f: impl FnOnce() -> Result<T, ConfigError>) -> Result<T, ConfigError> {
    #[cfg(unix)]
    let _guard = secrets_path().and_then(|p| {
        use std::os::fd::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(p.with_extension("enc.lock"))
            .ok()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        // SAFETY: the descriptor is owned by `file` for the duration of each call.
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        Some(file)
    });
    f()
}

#[cfg(not(feature = "test-secrets"))]
fn file_set(key: &str, value: &str) -> Result<(), ConfigError> {
    let cipher = cipher()?;
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce =
        Nonce::try_from(nonce_bytes.as_slice()).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    let ct = cipher
        .encrypt(&nonce, value.as_bytes())
        .map_err(|e| ConfigError::Keyring(e.to_string()))?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ct);
    let encoded = base64::engine::general_purpose::STANDARD.encode(blob);
    with_store_lock(|| {
        let mut map = read_map_strict()?;
        map.insert(key.to_string(), encoded);
        write_map(&map)
    })
}

#[cfg(not(feature = "test-secrets"))]
fn file_get(key: &str) -> Option<String> {
    let encoded = read_map().get(key)?.clone();
    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if blob.len() < 12 {
        return None;
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let nonce = Nonce::try_from(nonce_bytes).ok()?;
    let pt = cipher().ok()?.decrypt(&nonce, ct).ok()?;
    String::from_utf8(pt).ok()
}

#[cfg(not(feature = "test-secrets"))]
fn file_delete(key: &str) -> Result<bool, ConfigError> {
    with_store_lock(|| {
        let mut map = read_map_strict()?;
        if map.remove(key).is_some() {
            write_map(&map)?;
            return Ok(true);
        }
        Ok(false)
    })
}

// =============================================================================================
// Tests
// =============================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_call_returns_prompt_results_and_times_out_stalls() {
        assert_eq!(
            call_with_timeout(std::time::Duration::from_millis(100), || 42),
            Some(42)
        );
        assert_eq!(
            call_with_timeout(std::time::Duration::from_millis(5), || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                42
            }),
            None
        );
    }

    #[test]
    fn read_cache_memoizes_hits_and_misses_until_the_ttl_elapses() {
        let t0 = std::time::Instant::now();
        let mut cache = ReadCache::new();
        assert_eq!(
            cache.lookup("groq", t0),
            None,
            "cold cache asks the backend"
        );
        cache.store("groq", None, t0);
        cache.store("openai", Some("sk-1".into()), t0);
        // A confirmed miss is memoized too: that is the keyless-startup saving.
        assert_eq!(cache.lookup("groq", t0), Some(None));
        assert_eq!(cache.lookup("openai", t0), Some(Some("sk-1".into())));
        let within = t0 + READ_CACHE_TTL - std::time::Duration::from_millis(1);
        assert_eq!(cache.lookup("groq", within), Some(None));
        assert_eq!(cache.lookup("groq", t0 + READ_CACHE_TTL), None, "expired");
    }

    #[test]
    fn read_cache_forgets_a_key_on_write_or_delete_but_keeps_the_others() {
        let t0 = std::time::Instant::now();
        let mut cache = ReadCache::new();
        cache.store("groq", None, t0);
        cache.store("openai", Some("sk-1".into()), t0);
        cache.invalidate("groq");
        assert_eq!(
            cache.lookup("groq", t0),
            None,
            "`forge auth` must be visible at once"
        );
        assert_eq!(cache.lookup("openai", t0), Some(Some("sk-1".into())));
        cache.store("openai", Some("sk-2".into()), t0);
        assert_eq!(cache.lookup("openai", t0), Some(Some("sk-2".into())));
        assert_eq!(
            cache.entries.len(),
            1,
            "re-store replaces, never duplicates"
        );
    }

    #[test]
    fn tripwire_detects_cargo_test_deps_paths() {
        assert!(is_cargo_test_or_bench_binary(Path::new(
            "/home/u/proj/target/debug/deps/forge_config-abc123"
        )));
        assert!(is_cargo_test_or_bench_binary(Path::new(
            "/home/u/proj/target/x86_64-unknown-linux-gnu/release/deps/foo-deadbeef"
        )));
        // Windows-style separators: Path components are platform-native, so use forward
        // slashes (accepted by Windows Path) to keep this assertion host-independent.
        assert!(is_cargo_test_or_bench_binary(Path::new(
            "C:/Users/u/proj/target/debug/deps/forge_config-abc123.exe"
        )));
    }

    #[test]
    fn tripwire_allows_shipped_and_cargo_run_binaries() {
        // cargo run / release install: target/<profile>/<bin> — no `deps` segment.
        assert!(!is_cargo_test_or_bench_binary(Path::new(
            "/home/u/proj/target/debug/forge"
        )));
        assert!(!is_cargo_test_or_bench_binary(Path::new(
            "/home/u/proj/target/release/forge"
        )));
        assert!(!is_cargo_test_or_bench_binary(Path::new(
            "/usr/local/bin/forge"
        )));
        assert!(!is_cargo_test_or_bench_binary(Path::new(
            "/home/u/proj/target/debug/examples/demo"
        )));
        // A path that merely contains the words, not as path segments after target.
        assert!(!is_cargo_test_or_bench_binary(Path::new(
            "/home/u/deps-project/src/main"
        )));
    }

    #[cfg(feature = "test-secrets")]
    #[test]
    fn test_secrets_feature_reports_a_blind_in_memory_backend() {
        // This crate's own unit tests carry the feature, exactly like a `forge` bin that cargo
        // built alongside the test suite. The report must name the feature and the rebuild
        // command, because that bin's "no keys configured" is what gets mistaken for a lost key.
        let backend = backend();
        assert_eq!(backend, SecretBackend::InMemoryTest);
        assert!(backend.is_blind());
        let text = backend.describe();
        assert!(text.contains("test-secrets"), "{text}");
        assert!(text.contains("cargo test"), "{text}");
        assert!(text.contains("cargo build --bin forge"), "{text}");
    }

    #[test]
    fn real_backends_name_the_store_they_read() {
        let file = Some(PathBuf::from("/home/u/.config/forge/secrets.enc"));
        let keyring = SecretBackend::OsKeyring {
            fallback: file.clone(),
        };
        assert!(!keyring.is_blind());
        let text = keyring.describe();
        assert!(text.contains("OS keyring (service `forge`)"), "{text}");
        assert!(text.contains("/home/u/.config/forge/secrets.enc"), "{text}");

        let file_only = SecretBackend::EncryptedFile(file);
        assert!(!file_only.is_blind());
        let text = file_only.describe();
        assert!(
            text.starts_with("encrypted file /home/u/.config/forge/secrets.enc"),
            "{text}"
        );
        assert!(text.contains("keyring unreachable"), "{text}");

        let text = SecretBackend::EncryptedFile(None).describe();
        assert!(text.contains("<no config dir>"), "{text}");
    }

    #[cfg(feature = "test-secrets")]
    #[test]
    fn memory_backend_set_get_delete_round_trip() {
        // Unique key so parallel tests in this process don't collide on the shared map.
        let key = format!(
            "test-secrets-roundtrip-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        assert_eq!(get(&key), None);
        set(&key, "s3cret").unwrap();
        assert_eq!(get(&key).as_deref(), Some("s3cret"));
        assert!(delete(&key).unwrap());
        assert_eq!(get(&key), None);
        assert!(!delete(&key).unwrap(), "idempotent delete");
    }

    #[cfg(feature = "test-secrets")]
    #[test]
    fn memory_backend_is_compiled_not_keyring() {
        // Structural: with the feature on, the public API is the memory module. This test
        // existing under `cfg(feature = "test-secrets")` is itself proof the feature is active
        // for this crate's unit tests (self dev-dep). A call that succeeds without panicking
        // the tripwire further proves we never entered the real path.
        let key = "test-secrets-structural-probe";
        let _ = delete(key);
        set(key, "ok").unwrap();
        assert_eq!(get(key).as_deref(), Some("ok"));
        let _ = delete(key);
    }
}
