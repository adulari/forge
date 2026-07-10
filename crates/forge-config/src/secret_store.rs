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

#[cfg(not(feature = "test-secrets"))]
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
    if keyring_available()
        && keyring::Entry::new(KEYRING_SERVICE, key)
            .and_then(|e| e.set_password(value))
            .is_ok()
    {
        return Ok(());
    }
    file_set(key, value)
}

/// Read the secret for `key`: env-independent. Keyring first, then the encrypted file.
#[cfg(not(feature = "test-secrets"))]
pub fn get(key: &str) -> Option<String> {
    refuse_if_test_binary();
    if keyring_available() {
        if let Ok(v) = keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.get_password()) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    file_get(key)
}

/// Remove `key` from wherever it lives. `Ok(true)` if something was removed (from either store),
/// `Ok(false)` if nothing was stored — so removal stays idempotent.
#[cfg(not(feature = "test-secrets"))]
pub fn delete(key: &str) -> Result<bool, ConfigError> {
    refuse_if_test_binary();
    let mut removed = false;
    if keyring_available() {
        match keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) => removed = true,
            Err(keyring::Error::NoEntry) => {}
            Err(_) => {} // keyring unreachable — fall through to the file store
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

/// Whether the OS keyring backend is reachable — probed ONCE, with a timeout, and cached for the
/// process. This exists because on some boxes (WSL / headless Linux with an activatable-but-dead
/// `org.freedesktop.secrets`) a keyring call **blocks forever** instead of returning an error,
/// which hung `forge chat` before the TUI ever drew its first frame. We run the probe on a detached
/// thread and wait at most [`KEYRING_PROBE_TIMEOUT`]; if it doesn't answer we treat the keyring as
/// unavailable for the whole session and use the encrypted file store exclusively. A box with a
/// live keyring answers in milliseconds, so this is invisible there.
#[cfg(not(feature = "test-secrets"))]
fn keyring_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Any return (Ok OR Err) means the backend ANSWERED within the window — real calls will
            // then also return promptly (and fall back to the file on their own Err). Only a true
            // hang never sends, tripping the recv timeout below. The detached thread is left to
            // unblock on its own rather than wedging the main path.
            let _ =
                keyring::Entry::new(KEYRING_SERVICE, "__forge_probe__").map(|e| e.get_password());
            let _ = tx.send(());
        });
        match rx.recv_timeout(KEYRING_PROBE_TIMEOUT) {
            Ok(()) => true,
            Err(_) => {
                tracing::warn!(
                    "OS keyring did not respond within {}ms — using the encrypted file store for \
                     this session (secrets are still durable)",
                    KEYRING_PROBE_TIMEOUT.as_millis()
                );
                false
            }
        }
    })
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
    let mut f = opts
        .open(path)
        .map_err(|e| ConfigError::Keyring(e.to_string()))?;
    f.write_all(bytes)
        .map_err(|e| ConfigError::Keyring(e.to_string()))?;
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

#[cfg(not(feature = "test-secrets"))]
fn write_map(map: &BTreeMap<String, String>) -> Result<(), ConfigError> {
    let path = secrets_path().ok_or_else(|| ConfigError::Keyring("no config dir".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    }
    let body = serde_json::to_vec_pretty(map).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    write_private(&path, &body)
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
    let mut map = read_map();
    map.insert(key.to_string(), encoded);
    write_map(&map)
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
    let mut map = read_map();
    if map.remove(key).is_some() {
        write_map(&map)?;
        return Ok(true);
    }
    Ok(false)
}

// =============================================================================================
// Tests
// =============================================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
