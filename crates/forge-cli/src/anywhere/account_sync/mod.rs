//! Account sync: keys, skills, settings, MCP servers and user instructions follow the account to
//! the user's other machines over the existing end-to-end-encrypted Anywhere sync.
//!
//! No new record kind and no service change: items ride `user_setting` / `skill` / `command` /
//! `agent` records whose ids are keyed hashes, so the service cannot tell a theme from an API key.
//! Every category is a per-device opt-in under `[anywhere.account_sync]`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod apply;
mod catalog;
pub(crate) mod cli;
mod files;
mod reconcile;
pub(crate) mod rotate;
mod status;
mod vault;

pub(crate) use reconcile::run_cycle;
#[cfg(test)]
pub(crate) use status::status_fixture;
pub(crate) use status::{status_snapshot, AccountSyncStatus};

pub(crate) const SCHEMA: &str = "forge-account-sync-v1";
/// Items over this size are skipped and reported rather than uploaded.
pub(crate) const MAX_ITEM_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    Keys,
    Skills,
    Settings,
    Mcp,
    Instructions,
}

impl Category {
    pub(crate) const ALL: [Category; 5] = [
        Category::Keys,
        Category::Skills,
        Category::Settings,
        Category::Mcp,
        Category::Instructions,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Category::Keys => "keys",
            Category::Skills => "skills",
            Category::Settings => "settings",
            Category::Mcp => "mcp",
            Category::Instructions => "instructions",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Category::Keys => "Keys & tokens",
            Category::Skills => "Skills & agents",
            Category::Settings => "Settings",
            Category::Mcp => "MCP servers",
            Category::Instructions => "Instructions",
        }
    }

    pub(crate) fn parse(id: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.id() == id)
    }

    pub(crate) fn enabled(self, config: &forge_config::AccountSyncConfig) -> bool {
        config.enabled(self.id())
    }

    pub(crate) fn record_kinds(self) -> &'static [&'static str] {
        match self {
            Category::Skills => &["skill", "command", "agent", "user_setting"],
            _ => &["user_setting"],
        }
    }
}

/// The JSON carried inside the sealed sync envelope for every account-sync record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Payload {
    pub schema: String,
    pub category: String,
    pub item: String,
    pub label: String,
    pub kind: String,
    pub updated_at: u64,
    pub hash: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub sealed: Option<vault::Sealed>,
}

/// What one local item holds.
#[derive(Debug, Clone)]
pub(crate) enum Body {
    Plain(Value),
    Secret(String),
}

#[derive(Debug, Clone)]
pub(crate) struct LocalItem {
    pub category: Category,
    pub item: String,
    pub label: String,
    pub kind: &'static str,
    pub body: Body,
    pub modified_ms: Option<u64>,
}

impl LocalItem {
    pub(crate) fn canonical(&self) -> String {
        match &self.body {
            Body::Plain(value) => canonical_json(value),
            Body::Secret(secret) => secret.clone(),
        }
    }
}

/// An item the catalog could not offer for sync, with a reason a person can act on.
#[derive(Debug, Clone)]
pub(crate) struct Skip {
    pub label: String,
    pub reason: String,
}

pub(crate) fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical_json(&map[k])))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(items) => {
            let body: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", body.join(","))
        }
        other => other.to_string(),
    }
}

static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static LAST_CYCLE_MS: AtomicU64 = AtomicU64::new(0);
static SYNC_NOW: tokio::sync::Notify = tokio::sync::Notify::const_new();

pub(crate) fn record_cycle(error: Option<String>) {
    LAST_CYCLE_MS.store(super::now_ms(), Ordering::Relaxed);
    if let Ok(mut slot) = LAST_ERROR.lock() {
        *slot = error;
    }
}

pub(crate) fn last_error() -> Option<String> {
    LAST_ERROR.lock().ok().and_then(|slot| slot.clone())
}

pub(crate) fn request_sync_now() {
    SYNC_NOW.notify_one();
}

pub(crate) async fn wait_for_sync_request(delay: std::time::Duration) {
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        _ = SYNC_NOW.notified() => {}
    }
}

/// The local-first reason a feature is unavailable, used by the CLI and daemon alike.
pub(crate) fn not_signed_in_message() -> &'static str {
    "Sign in to Forge Anywhere first (`forge anywhere setup`)"
}

#[cfg(test)]
mod tests;
