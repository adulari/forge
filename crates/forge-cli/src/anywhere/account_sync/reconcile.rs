//! The reconcile pass: compares what this device has, what it last synced and what other devices
//! published, then uploads, applies or records a conflict per item.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use forge_store::{AccountSyncItem, Store};

use super::apply::{self, Outcome};
use super::catalog;
use super::vault::Vault;
use super::{Body, Category, LocalItem, Payload, SCHEMA};
use crate::anywhere::{decode_hex_array, now_ms, LocalState};

pub(super) struct Ctx<'a> {
    pub store: &'a Store,
    pub vault: &'a Vault,
    pub device_id: [u8; 16],
}

pub(super) struct Remote {
    pub clock: u64,
    pub mine: bool,
    pub payload: Option<Payload>,
}

pub(super) fn kind_for(item: &str) -> &'static str {
    match item.split_once('/').map(|(k, _)| k) {
        Some("skill") => "skill",
        Some("command") => "command",
        Some("agent") => "agent",
        _ => "user_setting",
    }
}

fn rank(item: &str) -> u8 {
    match item.split_once('/').map(|(k, _)| k) {
        Some("marketplace") => 0,
        Some("pack") => 2,
        _ => 1,
    }
}

pub(super) fn remotes(ctx: &Ctx, category: Category) -> Result<BTreeMap<String, Remote>> {
    let mut out = BTreeMap::new();
    for kind in category.record_kinds() {
        for record in ctx.store.portable_sync_records(kind)? {
            let mine = record.sender_device_id == ctx.device_id;
            let payload = if record.deleted {
                None
            } else {
                match serde_json::from_slice::<Payload>(&record.payload) {
                    Ok(p) if p.schema == SCHEMA && p.category == category.id() => Some(p),
                    _ => continue,
                }
            };
            out.insert(
                record.stable_id,
                Remote {
                    clock: record.logical_clock,
                    mine,
                    payload,
                },
            );
        }
    }
    Ok(out)
}

pub(crate) async fn run_cycle(
    store: &Store,
    state: &LocalState,
    config: &forge_config::AccountSyncConfig,
) -> Result<()> {
    if !config.any_enabled() {
        forge_config::account_sync::clear_sync_badge();
        return Ok(());
    }
    let vault = Vault::from_state(state)?;
    let device_id = decode_hex_array::<16>(
        state
            .device_id
            .as_deref()
            .context("Anywhere device id is missing")?,
        "device id",
    )?;
    let ctx = Ctx {
        store,
        vault: &vault,
        device_id,
    };
    let mut first_error = None;
    for category in Category::ALL.into_iter().filter(|c| c.enabled(config)) {
        if let Err(error) = reconcile_category(&ctx, category).await {
            first_error.get_or_insert(format!("{}: {error:#}", category.label()));
        }
    }
    let conflicts = store
        .account_sync_items(None)?
        .iter()
        .filter(|row| row.status == "conflict")
        .count();
    forge_config::account_sync::write_sync_badge(&forge_config::account_sync::SyncBadge {
        updated_at_ms: now_ms(),
        pending: store.pending_account_sync_uploads()?,
        conflicts,
        error: first_error.is_some(),
    });
    super::record_cycle(first_error);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn row(
    category: Category,
    id: &str,
    item: &str,
    label: &str,
    hash: &str,
    clock: u64,
    status: &str,
    detail: &str,
) -> AccountSyncItem {
    AccountSyncItem {
        category: category.id().into(),
        item_id: id.into(),
        item: item.into(),
        label: label.into(),
        local_hash: hash.into(),
        remote_clock: clock,
        applied_at: now_ms() as i64,
        status: status.into(),
        detail: detail.into(),
    }
}

pub(super) fn build_payload(ctx: &Ctx, id: &str, local: &LocalItem) -> Result<Payload> {
    let hash = ctx.vault.content_hash(&local.canonical());
    let (value, sealed) = match &local.body {
        Body::Plain(value) => (Some(value.clone()), None),
        Body::Secret(secret) => (None, Some(ctx.vault.seal(id, secret.as_bytes())?)),
    };
    Ok(Payload {
        schema: SCHEMA.into(),
        category: local.category.id().into(),
        item: local.item.clone(),
        label: local.label.clone(),
        kind: local.kind.into(),
        updated_at: local.modified_ms.unwrap_or_else(now_ms),
        hash,
        value,
        sealed,
    })
}

async fn reconcile_category(ctx: &Ctx<'_>, category: Category) -> Result<()> {
    let (locals, skips) = catalog::collect(category);
    let local: BTreeMap<String, LocalItem> = locals
        .into_iter()
        .map(|i| (ctx.vault.stable_id(category.id(), &i.item), i))
        .collect();
    let states: BTreeMap<String, AccountSyncItem> = ctx
        .store
        .account_sync_items(Some(category.id()))?
        .into_iter()
        .map(|s| (s.item_id.clone(), s))
        .collect();
    for state in states.values().filter(|s| s.item.starts_with("skip/")) {
        ctx.store
            .delete_account_sync_item(category.id(), &state.item_id)?;
    }
    for skip in &skips {
        let name = format!("skip/{}", skip.label);
        let id = ctx.vault.stable_id(category.id(), &name);
        ctx.store.upsert_account_sync_item(&row(
            category,
            &id,
            &name,
            &skip.label,
            "",
            0,
            "skipped",
            &skip.reason,
        ))?;
    }
    let remote = remotes(ctx, category)?;

    let mut ids: Vec<&String> = local
        .keys()
        .chain(states.keys())
        .chain(remote.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let name_of = |id: &String| -> String {
        local
            .get(id)
            .map(|l| l.item.clone())
            .or_else(|| {
                remote
                    .get(id)
                    .and_then(|r| r.payload.as_ref())
                    .map(|p| p.item.clone())
            })
            .or_else(|| states.get(id).map(|s| s.item.clone()))
            .unwrap_or_default()
    };
    ids.sort_by_key(|id| (rank(&name_of(id)), name_of(id)));

    let mut refresh: Vec<(String, String, String, u64, &'static str, String)> = Vec::new();
    for id in ids {
        let st = states.get(id).filter(|s| !s.item.starts_with("skip/"));
        let loc = local.get(id);
        let rem = remote.get(id).filter(|r| !r.mine);
        if loc.is_none() && st.is_none() && rem.is_some_and(|r| r.payload.is_none()) {
            continue;
        }
        let name = name_of(id);
        if name.is_empty() || name.starts_with("skip/") {
            continue;
        }
        let label = loc
            .map(|l| l.label.clone())
            .or_else(|| {
                rem.and_then(|r| r.payload.as_ref())
                    .map(|p| p.label.clone())
            })
            .or_else(|| st.map(|s| s.label.clone()))
            .unwrap_or_else(|| name.clone());
        let local_hash = loc.map(|l| ctx.vault.content_hash(&l.canonical()));
        let was_synced = st.is_some_and(|s| !s.local_hash.is_empty());
        let local_changed = match (&local_hash, st) {
            (Some(hash), Some(s)) => s.local_hash != *hash,
            (Some(_), None) => true,
            _ => false,
        };
        let local_removed = loc.is_none() && was_synced;
        let remote_new = rem.is_some_and(|r| r.clock > st.map_or(0, |s| s.remote_clock));
        let remote_payload = rem.and_then(|r| r.payload.as_ref());
        let remote_clock = rem.map_or(0, |r| r.clock);

        let outcome = async {
            match (remote_new, local_changed || local_removed) {
                (false, false) => Ok(None),
                (false, true) => upload(ctx, category, id, &name, &label, loc).map(Some),
                (true, false) => take_remote(
                    ctx,
                    category,
                    id,
                    remote_payload,
                    remote_clock,
                    &name,
                    &label,
                    false,
                )
                .await
                .map(Some),
                (true, true) => {
                    if let (Some(p), Some(h)) = (remote_payload, &local_hash) {
                        if p.hash == *h {
                            return Ok(Some(Step::Record(remote_clock, "synced", String::new())));
                        }
                    }
                    let remote_wins = match (remote_payload, loc) {
                        (None, Some(_)) => false,
                        (Some(_), None) => true,
                        (None, None) => true,
                        (Some(p), Some(l)) => p.updated_at > l.modified_ms.unwrap_or_else(now_ms),
                    };
                    if remote_wins {
                        let step = take_remote(
                            ctx,
                            category,
                            id,
                            remote_payload,
                            remote_clock,
                            &name,
                            &label,
                            loc.is_some(),
                        )
                        .await?;
                        Ok(Some(match step {
                            Step::Record(c, _, _) if loc.is_some() => Step::Record(
                                c,
                                "conflict",
                                "changed here and on another device; kept the other device's"
                                    .into(),
                            ),
                            other => other,
                        }))
                    } else {
                        let step = upload(ctx, category, id, &name, &label, loc)?;
                        Ok(Some(match step {
                            Step::Record(c, _, _) => Step::Record(
                                c,
                                "conflict",
                                "changed here and on another device; kept this device's".into(),
                            ),
                            other => other,
                        }))
                    }
                }
            }
        }
        .await;
        match outcome {
            Ok(None) => {}
            Ok(Some(Step::Record(clock, status, detail))) => {
                refresh.push((id.clone(), name, label, clock, status, detail))
            }
            Ok(Some(Step::Skipped(reason))) => ctx.store.upsert_account_sync_item(&row(
                category, id, &name, &label, "", 0, "skipped", &reason,
            ))?,
            Err(error) => ctx.store.upsert_account_sync_item(&row(
                category,
                id,
                &name,
                &label,
                st.map_or("", |s| s.local_hash.as_str()),
                st.map_or(0, |s| s.remote_clock),
                "error",
                &format!("{error:#}"),
            ))?,
        }
    }

    if !refresh.is_empty() {
        let (fresh, _) = catalog::collect(category);
        let fresh: BTreeMap<String, String> = fresh
            .into_iter()
            .map(|i| {
                (
                    ctx.vault.stable_id(category.id(), &i.item),
                    ctx.vault.content_hash(&i.canonical()),
                )
            })
            .collect();
        for (id, name, label, clock, status, detail) in refresh {
            let hash = fresh.get(&id).cloned().unwrap_or_default();
            let status = if hash.is_empty() && status == "synced" {
                "removed"
            } else {
                status
            };
            ctx.store.upsert_account_sync_item(&row(
                category, &id, &name, &label, &hash, clock, status, &detail,
            ))?;
        }
    }
    Ok(())
}

pub(super) enum Step {
    Record(u64, &'static str, String),
    Skipped(String),
}

fn upload(
    ctx: &Ctx,
    category: Category,
    id: &str,
    name: &str,
    label: &str,
    loc: Option<&LocalItem>,
) -> Result<Step> {
    let kind = loc.map_or_else(|| kind_for(name), |l| l.kind);
    match loc {
        Some(local) => {
            let payload = serde_json::to_vec(&build_payload(ctx, id, local)?)?;
            ctx.store
                .write_portable_sync_record(ctx.device_id, kind, id, Some(&payload))?;
        }
        None => ctx
            .store
            .write_portable_sync_record(ctx.device_id, kind, id, None)?,
    }
    let clock = ctx
        .store
        .portable_sync_record(kind, id)?
        .map_or(0, |r| r.logical_clock);
    let _ = (category, label);
    Ok(Step::Record(clock, "synced", String::new()))
}

#[allow(clippy::too_many_arguments)]
async fn take_remote(
    ctx: &Ctx<'_>,
    category: Category,
    id: &str,
    payload: Option<&Payload>,
    clock: u64,
    name: &str,
    _label: &str,
    keep_conflict: bool,
) -> Result<Step> {
    let Some(payload) = payload else {
        apply::remove_local(category, name)?;
        return Ok(Step::Record(clock, "synced", String::new()));
    };
    if ctx.vault.stable_id(category.id(), &payload.item) != id {
        anyhow::bail!("a synced item does not match its record id");
    }
    let secret = match &payload.sealed {
        Some(sealed) => Some(
            String::from_utf8(ctx.vault.open(id, sealed)?).context("synced secret is not text")?,
        ),
        None => None,
    };
    Ok(
        match apply::apply(category, payload, secret, keep_conflict).await? {
            Outcome::Applied => Step::Record(clock, "synced", String::new()),
            Outcome::Skipped(reason) => Step::Skipped(reason),
        },
    )
}
