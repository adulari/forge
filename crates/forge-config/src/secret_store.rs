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

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;

use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore as _;

use crate::ConfigError;

const KEYRING_SERVICE: &str = "forge";

/// Store `value` under `key`: OS keyring first, encrypted file on keyring failure.
pub fn set(key: &str, value: &str) -> Result<(), ConfigError> {
    match keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.set_password(value)) {
        Ok(()) => Ok(()),
        Err(_) => file_set(key, value),
    }
}

/// Read the secret for `key`: env-independent. Keyring first, then the encrypted file.
pub fn get(key: &str) -> Option<String> {
    if let Ok(v) = keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.get_password()) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    file_get(key)
}

/// Remove `key` from wherever it lives. `Ok(true)` if something was removed (from either store),
/// `Ok(false)` if nothing was stored — so removal stays idempotent.
pub fn delete(key: &str) -> Result<bool, ConfigError> {
    let mut removed = false;
    match keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.delete_credential()) {
        Ok(()) => removed = true,
        Err(keyring::Error::NoEntry) => {}
        Err(_) => {} // keyring unreachable — fall through to the file store
    }
    removed |= file_delete(key)?;
    Ok(removed)
}

// --- encrypted file fallback ------------------------------------------------------------------

fn secrets_path() -> Option<PathBuf> {
    crate::config_dir().map(|d| d.join("secrets.enc"))
}

fn keyfile_path() -> Option<PathBuf> {
    crate::config_dir().map(|d| d.join("secret.key"))
}

/// Load (or create) the 32-byte file-store key. Stored `0600` next to the encrypted blob.
fn load_or_create_key() -> Result<Key, ConfigError> {
    let path = keyfile_path().ok_or_else(|| ConfigError::Keyring("no config dir".into()))?;
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            return Ok(*Key::from_slice(&bytes));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    }
    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    write_private(&path, &raw)?;
    Ok(*Key::from_slice(&raw))
}

/// Write a file readable/writable only by the owner (`0600` on Unix).
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

fn cipher() -> Result<ChaCha20Poly1305, ConfigError> {
    Ok(ChaCha20Poly1305::new(&load_or_create_key()?))
}

/// The on-disk map is `name -> base64(nonce ‖ ciphertext)`.
fn read_map() -> BTreeMap<String, String> {
    secrets_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_map(map: &BTreeMap<String, String>) -> Result<(), ConfigError> {
    let path = secrets_path().ok_or_else(|| ConfigError::Keyring("no config dir".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    }
    let body = serde_json::to_vec_pretty(map).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    write_private(&path, &body)
}

fn file_set(key: &str, value: &str) -> Result<(), ConfigError> {
    let cipher = cipher()?;
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, value.as_bytes())
        .map_err(|e| ConfigError::Keyring(e.to_string()))?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ct);
    let encoded = base64::engine::general_purpose::STANDARD.encode(blob);
    let mut map = read_map();
    map.insert(key.to_string(), encoded);
    write_map(&map)
}

fn file_get(key: &str) -> Option<String> {
    let encoded = read_map().get(key)?.clone();
    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if blob.len() < 12 {
        return None;
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let pt = cipher()
        .ok()?
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .ok()?;
    String::from_utf8(pt).ok()
}

fn file_delete(key: &str) -> Result<bool, ConfigError> {
    let mut map = read_map();
    if map.remove(key).is_some() {
        write_map(&map)?;
        return Ok(true);
    }
    Ok(false)
}
