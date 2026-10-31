//! Which paired client registered each push subscription, so revoking a client also removes the
//! notifications it would otherwise keep receiving. Kept beside the clients file rather than in
//! the store schema; rows from before this existed have no owner and behave as before.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::serve_clients::ClientAuth;

const OWNERS_FILE: &str = "push-owners.json";
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Entry {
    client: String,
    kind: String,
    key: String,
}

fn path() -> Option<PathBuf> {
    crate::serve_access::access()
        .clients
        .sibling_path(OWNERS_FILE)
}

fn load(path: &PathBuf) -> Vec<Entry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(path: &PathBuf, entries: &[Entry]) {
    let Ok(text) = serde_json::to_string(entries) else {
        return;
    };
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn edit(change: impl FnOnce(&mut Vec<Entry>)) {
    let Some(path) = path() else { return };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load(&path);
    change(&mut entries);
    save(&path, &entries);
}

/// Tie `key` to the calling client. The owner token claims it back (no owner row).
pub(crate) fn record(auth: &ClientAuth, kind: &str, key: &str) {
    let client = auth.id.clone();
    edit(|entries| {
        entries.retain(|e| !(e.kind == kind && e.key == key));
        if let Some(client) = client {
            entries.push(Entry {
                client,
                kind: kind.into(),
                key: key.into(),
            });
        }
    });
}

pub(crate) fn forget(kind: &str, key: &str) {
    edit(|entries| entries.retain(|e| !(e.kind == kind && e.key == key)));
}

/// The owner may touch anything; a paired client only its own rows (or rows nobody claimed).
pub(crate) fn may_modify(auth: &ClientAuth, kind: &str, key: &str) -> bool {
    let Some(id) = auth.id.as_deref() else {
        return true;
    };
    let Some(path) = path() else { return true };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load(&path)
        .iter()
        .find(|e| e.kind == kind && e.key == key)
        .is_none_or(|e| e.client == id)
}

/// Delete the subscriptions of every client that is no longer paired. Returns how many went.
pub(crate) fn purge_revoked(store: &forge_store::Store) -> usize {
    let Some(path) = path() else { return 0 };
    let active: HashSet<String> = crate::serve_access::access().clients.active_ids();
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let entries = load(&path);
    let (keep, gone): (Vec<Entry>, Vec<Entry>) = entries
        .into_iter()
        .partition(|e| active.contains(&e.client));
    if gone.is_empty() {
        return 0;
    }
    for entry in &gone {
        let _ = match entry.kind.as_str() {
            "webpush" => store.delete_push_subscription(&entry.key),
            "apns" => store.delete_apns_subscription(&entry.key),
            _ => store.delete_live_activity_token(&entry.key),
        };
    }
    save(&path, &keep);
    gone.len()
}

/// Revocations made from the CLI land in the clients file, not in this process; sweep for them.
pub(crate) fn spawn_sweeper(store: std::sync::Arc<forge_store::Store>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            let store = store.clone();
            let _ = tokio::task::spawn_blocking(move || purge_revoked(&store)).await;
        }
    });
}
