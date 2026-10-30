//! The status document served at `GET /api/anywhere/sync` and printed by `forge anywhere sync`.
//! Built from labels and counts only; no value ever reaches it.

use std::collections::BTreeMap;

use anyhow::Result;
use forge_store::Store;
use serde::Serialize;

use super::catalog::{self, OAUTH_LOGINS, TOOLS};
use super::reconcile::{remotes, Ctx};
use super::vault::Vault;
use super::Category;
use crate::anywhere::{decode_hex_array, StateStore};

#[derive(Debug, Serialize)]
pub(crate) struct CategoryStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    pub items: usize,
    pub pending: usize,
    pub last_sync_at: Option<i64>,
    pub remote_only: Option<usize>,
    pub conflicts: Vec<Conflict>,
    pub skipped: Vec<Skipped>,
    pub toggle_requires_direct: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct Conflict {
    pub item: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Skipped {
    pub item: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SignIn {
    pub provider: String,
    pub label: String,
    pub command: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct MissingTool {
    pub tool: String,
    pub label: String,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AccountSyncStatus {
    pub signed_in: bool,
    pub anywhere_enabled: bool,
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
    pub categories: Vec<CategoryStatus>,
    pub sign_ins: Vec<SignIn>,
    pub missing_tools: Vec<MissingTool>,
}

pub(crate) fn status_snapshot(store: &Store) -> Result<AccountSyncStatus> {
    let config = forge_config::load()?;
    let state = StateStore::platform()?.load()?;
    let signed_in = state.is_logged_in() && state.account_id.is_some();
    let vault = if signed_in {
        Vault::from_state(&state).ok()
    } else {
        None
    };
    let device_id = state
        .device_id
        .as_deref()
        .and_then(|d| decode_hex_array::<16>(d, "device id").ok());
    let ctx = match (&vault, device_id) {
        (Some(vault), Some(device_id)) => Some(Ctx {
            store,
            vault,
            device_id,
        }),
        _ => None,
    };
    let mut categories = Vec::new();
    let mut sign_ins = Vec::new();
    let mut missing_tools = Vec::new();
    let mut newest: Option<i64> = None;
    for category in Category::ALL {
        let enabled = category.enabled(&config.anywhere.account_sync);
        let rows = store.account_sync_items(Some(category.id()))?;
        let last = rows
            .iter()
            .filter(|r| r.status != "skipped")
            .map(|r| r.applied_at)
            .max();
        newest = newest.max(last);
        let mut pending = 0;
        let mut remote_only = None;
        if let Some(ctx) = &ctx {
            let (locals, _) = catalog::collect(category);
            let by_id: BTreeMap<_, _> = rows.iter().map(|r| (r.item.as_str(), r)).collect();
            if enabled {
                pending = locals
                    .iter()
                    .filter(|l| {
                        by_id
                            .get(l.item.as_str())
                            .is_none_or(|r| r.local_hash != ctx.vault.content_hash(&l.canonical()))
                    })
                    .count();
            }
            let remote = remotes(ctx, category).unwrap_or_default();
            let mine: Vec<&str> = locals.iter().map(|l| l.item.as_str()).collect();
            let waiting = remote
                .values()
                .filter(|r| !r.mine)
                .filter_map(|r| r.payload.as_ref())
                .filter(|p| {
                    !mine.contains(&p.item.as_str())
                        && !p.item.starts_with("login/")
                        && !p.item.starts_with("tool/")
                })
                .count();
            remote_only = Some(waiting);
            if enabled {
                collect_hints(category, &remote, &mut sign_ins, &mut missing_tools);
            }
        }
        categories.push(CategoryStatus {
            id: category.id(),
            label: category.label(),
            enabled,
            items: rows
                .iter()
                .filter(|r| matches!(r.status.as_str(), "synced" | "conflict"))
                .count(),
            pending,
            last_sync_at: last,
            remote_only,
            conflicts: rows
                .iter()
                .filter(|r| r.status == "conflict")
                .map(|r| Conflict {
                    item: r.label.clone(),
                    detail: r.detail.clone(),
                })
                .collect(),
            skipped: rows
                .iter()
                .filter(|r| matches!(r.status.as_str(), "skipped" | "error"))
                .map(|r| Skipped {
                    item: r.label.clone(),
                    reason: r.detail.clone(),
                })
                .collect(),
            toggle_requires_direct: category == Category::Keys,
        });
    }
    Ok(AccountSyncStatus {
        signed_in,
        anywhere_enabled: config.anywhere.enabled,
        last_sync_at: newest,
        last_error: super::last_error(),
        categories,
        sign_ins,
        missing_tools,
    })
}

fn collect_hints(
    category: Category,
    remote: &BTreeMap<String, super::reconcile::Remote>,
    sign_ins: &mut Vec<SignIn>,
    missing: &mut Vec<MissingTool>,
) {
    for payload in remote
        .values()
        .filter(|r| !r.mine)
        .filter_map(|r| r.payload.as_ref())
    {
        if category == Category::Keys {
            if let Some(provider) = payload.item.strip_prefix("login/") {
                if let Some((_, label, command)) =
                    OAUTH_LOGINS.iter().find(|(p, ..)| *p == provider)
                {
                    if forge_config::provider_oauth::load_provider_oauth_tokens(provider).is_none()
                    {
                        sign_ins.push(SignIn {
                            provider: provider.into(),
                            label: (*label).into(),
                            command: (*command).into(),
                        });
                    }
                } else if let Some(server) = provider.strip_prefix("mcp:") {
                    if forge_config::oauth::load_oauth_tokens(server).is_none() {
                        sign_ins.push(SignIn {
                            provider: provider.into(),
                            label: format!("MCP {server}"),
                            command: format!("forge mcp login {server}"),
                        });
                    }
                }
            }
        }
        if category == Category::Settings {
            if let Some(bin) = payload.item.strip_prefix("tool/") {
                if let Some((_, label, hint)) = TOOLS.iter().find(|(b, ..)| *b == bin) {
                    if !catalog::on_path(bin) {
                        missing.push(MissingTool {
                            tool: bin.into(),
                            label: (*label).into(),
                            hint: (*hint).into(),
                        });
                    }
                }
            }
        }
    }
}
