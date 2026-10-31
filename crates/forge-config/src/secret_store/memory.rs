//! In-memory secret backend for the `test-secrets` feature: two stores and the ways a reader
//! can disagree with the truth, so tests can stage what several Forge processes do to each other.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use crate::ConfigError;

/// Two stores plus the two ways a reader can disagree with the truth, so tests can stage what
/// the real backends do across processes: `primary` is the OS keyring, `shadow` the encrypted
/// file, `pinned` the process-local read memo (stale until this process writes) and `hidden`
/// a keyring this process cannot reach.
#[derive(Default)]
pub(super) struct Stores {
    primary: BTreeMap<String, String>,
    shadow: BTreeMap<String, String>,
    pinned: BTreeMap<String, Option<String>>,
    hidden: BTreeSet<String>,
}

pub(super) fn stores() -> std::sync::MutexGuard<'static, Stores> {
    static STORES: Mutex<Stores> = Mutex::new(Stores {
        primary: BTreeMap::new(),
        shadow: BTreeMap::new(),
        pinned: BTreeMap::new(),
        hidden: BTreeSet::new(),
    });
    STORES.lock().unwrap_or_else(|e| e.into_inner())
}

impl Stores {
    fn reachable_primary(&self, key: &str) -> Option<String> {
        if self.hidden.contains(key) {
            return None;
        }
        self.primary.get(key).cloned()
    }

    fn truth(&self, key: &str) -> Option<String> {
        self.reachable_primary(key)
            .or_else(|| self.shadow.get(key).cloned())
    }

    pub(super) fn pin(&mut self, key: &str, value: Option<String>) {
        self.pinned.insert(key.to_string(), value);
    }

    pub(super) fn write_primary(&mut self, key: &str, value: &str) {
        self.primary.insert(key.to_string(), value.to_string());
    }

    pub(super) fn write_shadow(&mut self, key: &str, value: &str) {
        self.shadow.insert(key.to_string(), value.to_string());
    }

    pub(super) fn hide_primary(&mut self, key: &str, hidden: bool) {
        if hidden {
            self.hidden.insert(key.to_string());
        } else {
            self.hidden.remove(key);
        }
    }
}

pub(super) fn set(key: &str, value: &str) -> Result<(), ConfigError> {
    let mut stores = stores();
    stores.pinned.remove(key);
    if stores.hidden.contains(key) {
        stores.shadow.insert(key.to_string(), value.to_string());
    } else {
        stores.primary.insert(key.to_string(), value.to_string());
    }
    Ok(())
}

pub(super) fn set_all(key: &str, value: &str) -> Result<(), ConfigError> {
    let mut stores = stores();
    stores.pinned.remove(key);
    if !stores.hidden.contains(key) {
        stores.primary.insert(key.to_string(), value.to_string());
    }
    stores.shadow.insert(key.to_string(), value.to_string());
    Ok(())
}

/// Like the real memoized read: a pinned (stale) answer wins until this process writes.
pub(super) fn get(key: &str) -> Option<String> {
    let stores = stores();
    match stores.pinned.get(key) {
        Some(pinned) => pinned.clone(),
        None => stores.truth(key),
    }
}

pub(super) fn get_fresh(key: &str) -> Option<String> {
    let mut stores = stores();
    stores.pinned.remove(key);
    stores.truth(key)
}

pub(super) fn get_each(key: &str) -> Vec<String> {
    let stores = stores();
    let mut found = Vec::new();
    found.extend(stores.reachable_primary(key));
    if let Some(v) = stores.shadow.get(key) {
        if !found.contains(v) {
            found.push(v.clone());
        }
    }
    found
}

pub(super) fn delete(key: &str) -> Result<bool, ConfigError> {
    let mut stores = stores();
    stores.pinned.remove(key);
    let primary = stores.primary.remove(key).is_some();
    let shadow = stores.shadow.remove(key).is_some();
    Ok(primary || shadow)
}

/// Staging helpers for tests in other crates: they reproduce, with the in-memory backend, what two
/// Forge processes sharing one keyring and one encrypted file do to each other.
pub mod staging {
    use super::stores;

    /// Make this process's memoized read of `key` keep answering `value` (a stale copy another
    /// process has since replaced) until this process itself writes or reads it fresh.
    pub fn pin_stale_read(key: &str, value: Option<&str>) {
        stores().pin(key, value.map(str::to_string));
    }

    /// Write only the keyring copy, as a process that can reach the keyring does via `set`.
    pub fn write_keyring_only(key: &str, value: &str) {
        stores().write_primary(key, value);
    }

    /// Write only the encrypted-file copy, as a process whose keyring is unreachable does.
    pub fn write_file_only(key: &str, value: &str) {
        stores().write_shadow(key, value);
    }

    /// Make the keyring unreachable (or reachable again) for `key`, like an SSH shell without the
    /// session bus.
    pub fn set_keyring_reachable(key: &str, reachable: bool) {
        stores().hide_primary(key, !reachable);
    }
}
