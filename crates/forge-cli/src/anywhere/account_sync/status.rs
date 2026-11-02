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

/// The upload outbox as a whole: every journal row not yet acknowledged, not just the account-sync
/// kinds, plus why the drain is stuck when it is.
#[derive(Debug, Default, Serialize)]
pub(crate) struct OutboxStatus {
    pub pending: usize,
    /// Epoch seconds the oldest waiting row was journaled.
    pub oldest_at: Option<i64>,
    /// Waiting rows and a recorded upload failure: nothing behind the oldest row can upload.
    pub blocked: bool,
    /// The service's error code for the failure, e.g. `replay_detected`.
    pub code: Option<String>,
    pub error: Option<String>,
    pub blocked_since_ms: Option<u64>,
    pub attempts: u32,
}

impl OutboxStatus {
    pub(crate) fn from_parts(
        backlog: forge_store::SyncOutboxBacklog,
        failure: Option<forge_config::account_sync::SyncOutboxFailure>,
    ) -> Self {
        let blocked = backlog.pending > 0 && failure.is_some();
        Self {
            pending: backlog.pending,
            oldest_at: backlog.oldest_created_at,
            blocked,
            code: failure.as_ref().and_then(|f| f.code.clone()),
            error: failure.as_ref().map(|f| f.message.clone()),
            blocked_since_ms: failure.as_ref().map(|f| f.since_ms),
            attempts: failure.map_or(0, |f| f.attempts),
        }
    }

    /// One line for the CLI and the TUI card, or `None` when there is nothing to say.
    pub(crate) fn line(&self) -> Option<String> {
        if self.blocked {
            let why = match (&self.code, &self.error) {
                (Some(code), _) => code.clone(),
                (None, Some(error)) => error.clone(),
                (None, None) => "upload failing".into(),
            };
            Some(format!(
                "Upload outbox BLOCKED: {} waiting, oldest row cannot upload ({why})",
                self.pending
            ))
        } else if self.pending > 0 {
            Some(format!("Upload outbox: {} waiting", self.pending))
        } else {
            None
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct AccountSyncStatus {
    pub outbox: OutboxStatus,
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
        outbox: OutboxStatus::from_parts(
            store.sync_outbox_backlog()?,
            forge_config::account_sync::read_outbox_failure(),
        ),
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

/// A signed-in status with one of everything, for tests of the layers that present it.
#[cfg(test)]
pub(crate) fn status_fixture() -> AccountSyncStatus {
    let category = |id: &'static str, label: &'static str, enabled: bool| CategoryStatus {
        id,
        label,
        enabled,
        items: 2,
        pending: 0,
        last_sync_at: None,
        remote_only: None,
        conflicts: Vec::new(),
        skipped: Vec::new(),
        toggle_requires_direct: id == "keys",
    };
    let mut categories = vec![
        category("keys", "Keys & tokens", false),
        category("mcp", "MCP servers", true),
    ];
    categories[0].remote_only = Some(3);
    categories[1].skipped.push(Skipped {
        item: "mcp: github".into(),
        reason: "needs `npx`, not installed here".into(),
    });
    AccountSyncStatus {
        outbox: OutboxStatus::default(),
        signed_in: true,
        anywhere_enabled: true,
        last_sync_at: None,
        last_error: None,
        categories,
        sign_ins: vec![SignIn {
            provider: "codex".into(),
            label: "Codex (ChatGPT)".into(),
            command: "forge auth codex-oauth".into(),
        }],
        missing_tools: Vec::new(),
    }
}

#[cfg(test)]
mod outbox_tests {
    use super::*;
    use forge_config::account_sync::SyncOutboxFailure;
    use forge_store::SyncOutboxBacklog;

    fn failure(code: Option<&str>) -> SyncOutboxFailure {
        SyncOutboxFailure {
            since_ms: 1,
            updated_at_ms: 2,
            code: code.map(str::to_owned),
            message: "boom".into(),
            journal_id: Some(7),
            attempts: 3,
        }
    }

    #[test]
    fn a_backlog_with_a_failure_is_blocked_and_names_the_code() {
        let backlog = SyncOutboxBacklog {
            pending: 226,
            oldest_created_at: Some(5),
            oldest_id: Some(7),
        };
        let status = OutboxStatus::from_parts(backlog, Some(failure(Some("replay_detected"))));
        assert!(status.blocked);
        let line = status.line().expect("a blocked outbox is reported");
        assert!(
            line.contains("BLOCKED") && line.contains("226") && line.contains("replay_detected")
        );
    }

    #[test]
    fn a_failure_without_a_backlog_is_not_blocked_and_a_backlog_alone_is_just_waiting() {
        let empty = OutboxStatus::from_parts(SyncOutboxBacklog::default(), Some(failure(None)));
        assert!(!empty.blocked && empty.line().is_none());
        let waiting = OutboxStatus::from_parts(
            SyncOutboxBacklog {
                pending: 4,
                ..Default::default()
            },
            None,
        );
        assert!(!waiting.blocked);
        assert_eq!(waiting.line().as_deref(), Some("Upload outbox: 4 waiting"));
    }
}
