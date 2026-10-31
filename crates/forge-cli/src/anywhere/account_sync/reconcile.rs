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
    pub sender: [u8; 16],
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
                    sender: record.sender_device_id,
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

pub(super) fn build_payload(
    ctx: &Ctx,
    id: &str,
    local: &LocalItem,
    parent: Option<&str>,
) -> Result<Payload> {
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
        parent: parent.map(str::to_owned),
    })
}

/// Whether an incoming payload already says what this device holds, so nothing needs applying.
fn same_content(ctx: &Ctx, id: &str, payload: &Payload, local: &LocalItem) -> bool {
    match (&local.body, &payload.sealed, &payload.value) {
        (Body::Secret(mine), Some(sealed), _) => ctx
            .vault
            .open(id, sealed)
            .is_ok_and(|bytes| bytes == mine.as_bytes()),
        (Body::Plain(_), None, Some(value)) => super::canonical_json(value) == local.canonical(),
        _ => false,
    }
}

/// The record id an item uses on the wire. Identity is the (category, item) inside the payload,
/// never which key epochs a device holds, so the id is whatever the account already uses: the
/// smallest live id any device published, else the one this device last recorded, else a fresh one.
fn canonical_id(
    ctx: &Ctx,
    category: Category,
    name: &str,
    st: Option<&AccountSyncItem>,
    live: &[(&String, &Remote)],
) -> String {
    let recorded_live = st.and_then(|s| live.iter().find(|(id, _)| **id == s.item_id));
    live.iter()
        .map(|(id, _)| (*id).clone())
        .chain(recorded_live.map(|(id, _)| (*id).clone()))
        .min()
        .or_else(|| st.map(|s| s.item_id.clone()))
        .unwrap_or_else(|| ctx.vault.stable_id(category.id(), name))
}

pub(super) fn recorded_id(ctx: &Ctx, category: Category, name: &str) -> String {
    ctx.store
        .account_sync_items(Some(category.id()))
        .ok()
        .and_then(|rows| rows.into_iter().find(|r| r.item == name))
        .map_or_else(|| ctx.vault.stable_id(category.id(), name), |r| r.item_id)
}

struct Refresh {
    id: String,
    old_id: Option<String>,
    name: String,
    label: String,
    clock: u64,
    status: &'static str,
    detail: String,
}

async fn reconcile_category(ctx: &Ctx<'_>, category: Category) -> Result<()> {
    let (locals, skips) = catalog::collect(category);
    let guards: Vec<String> = skips.iter().filter_map(|s| s.guard.clone()).collect();
    let local: BTreeMap<String, LocalItem> =
        locals.into_iter().map(|i| (i.item.clone(), i)).collect();
    let all_states = ctx.store.account_sync_items(Some(category.id()))?;
    for state in all_states.iter().filter(|s| s.item.starts_with("skip/")) {
        ctx.store
            .delete_account_sync_item(category.id(), &state.item_id)?;
    }
    let states: BTreeMap<String, AccountSyncItem> = all_states
        .into_iter()
        .filter(|s| !s.item.starts_with("skip/"))
        .map(|s| (s.item.clone(), s))
        .collect();
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
    let mut live: BTreeMap<String, Vec<(&String, &Remote)>> = BTreeMap::new();
    for (id, r) in &remote {
        if let Some(p) = &r.payload {
            live.entry(p.item.clone()).or_default().push((id, r));
        }
    }

    let mut names: Vec<&String> = local
        .keys()
        .chain(states.keys())
        .chain(live.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    names.sort_by_key(|n| (rank(n), (*n).clone()));

    let mut refresh: Vec<Refresh> = Vec::new();
    for name in names {
        if guards.iter().any(|g| name.starts_with(g.as_str())) {
            continue;
        }
        let st = states.get(name);
        let loc = local.get(name);
        let cands: &[(&String, &Remote)] = live.get(name).map_or(&[], Vec::as_slice);
        let id = canonical_id(ctx, category, name, st, cands);
        let moved = st.filter(|s| s.item_id != id).map(|s| s.item_id.clone());
        // Newest record by another device: any live id for this item, or a tombstone at the id in use.
        let tomb = remote.get(&id).filter(|r| r.payload.is_none() && !r.mine);
        let newest = cands
            .iter()
            .map(|(rid, r)| (Some(*rid), *r))
            .chain(tomb.map(|t| (None, t)))
            .filter(|(_, r)| !r.mine)
            .max_by_key(|(_, r)| (r.clock, r.sender));
        let rem_payload = newest.and_then(|(_, r)| r.payload.as_ref());
        let rem_id = newest.and_then(|(rid, _)| rid.cloned());
        let remote_clock = newest.map_or(0, |(_, r)| r.clock);
        if loc.is_none() && st.is_none() && rem_payload.is_none() {
            continue;
        }
        let label = loc
            .map(|l| l.label.clone())
            .or_else(|| rem_payload.map(|p| p.label.clone()))
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
        // This device's own live record for the item, under any id. A foreign record only counts as
        // new when it beats it; otherwise applying it would overwrite what this device wrote last.
        let own_best = cands
            .iter()
            .filter(|(_, r)| r.mine)
            .map(|(_, r)| (r.clock, r.sender))
            .max();
        let foreign_beats_own =
            newest.is_some_and(|(_, r)| own_best.is_none_or(|own| (r.clock, r.sender) > own));
        // Two devices that wrote the same id at the same clock: the store keeps the higher device
        // id's record, so the loser sees a foreign record at the clock it recorded as its own.
        let tie_lost = match (newest, st) {
            (Some((rid, r)), Some(s)) => {
                !r.mine
                    && r.clock == s.remote_clock
                    && rid.map(String::as_str).unwrap_or(id.as_str()) == s.item_id
                    && match &r.payload {
                        Some(p) => p.hash != s.local_hash,
                        None => !s.local_hash.is_empty(),
                    }
            }
            _ => false,
        };
        let synced_hash = Some(st.map_or("", |s| s.local_hash.as_str()));
        // The incoming edit was made on top of a version this device never synced.
        let concurrent = rem_payload
            .and_then(|p| p.parent.as_deref())
            .is_some_and(|parent| Some(parent) != synced_hash);
        let remote_new = foreign_beats_own
            && (moved.is_some() || tie_lost || remote_clock > st.map_or(0, |s| s.remote_clock));

        let outcome = async {
            if remote_new {
                if let (Some(p), Some(l), Some(rid)) = (rem_payload, loc, rem_id.as_deref()) {
                    if same_content(ctx, rid, p, l) {
                        return Ok(Some(Step::Record(remote_clock, "synced", String::new())));
                    }
                }
            }
            match (remote_new, local_changed || local_removed) {
                (false, false) => Ok(None),
                (false, true) => upload(ctx, &id, name, loc, synced_hash).map(Some),
                (true, false) => take_remote(
                    ctx,
                    category,
                    rem_id.as_deref(),
                    rem_payload,
                    remote_clock,
                    name,
                    loc.is_some() && (moved.is_some() || tie_lost || concurrent),
                )
                .await
                .map(Some),
                (true, true) => {
                    let remote_wins = match (rem_payload, loc) {
                        (None, Some(_)) => false,
                        (Some(_), None) | (None, None) => true,
                        (Some(p), Some(l)) => p.updated_at > l.modified_ms.unwrap_or_else(now_ms),
                    };
                    let (step, kept) = if remote_wins {
                        let step = take_remote(
                            ctx,
                            category,
                            rem_id.as_deref(),
                            rem_payload,
                            remote_clock,
                            name,
                            loc.is_some(),
                        )
                        .await?;
                        (step, "the other device's")
                    } else {
                        (upload(ctx, &id, name, loc, synced_hash)?, "this device's")
                    };
                    Ok(Some(match step {
                        Step::Record(c, _, _) if loc.is_some() => Step::Record(
                            c,
                            "conflict",
                            format!("changed here and on another device; kept {kept}"),
                        ),
                        other => other,
                    }))
                }
            }
        }
        .await;
        match outcome {
            Ok(None) => {
                if moved.is_some() {
                    refresh.push(Refresh {
                        id: id.clone(),
                        old_id: moved,
                        name: name.clone(),
                        label,
                        clock: remote_clock,
                        status: "synced",
                        detail: String::new(),
                    });
                }
            }
            Ok(Some(Step::Record(clock, status, detail))) => {
                refresh.push(Refresh {
                    id: id.clone(),
                    old_id: moved,
                    name: name.clone(),
                    label,
                    clock,
                    status,
                    detail,
                });
            }
            Ok(Some(Step::Skipped(reason))) => {
                if let Some(old) = &moved {
                    ctx.store.delete_account_sync_item(category.id(), old)?;
                }
                // A deferral only retries the apply later. Forgetting the synced hash would make an
                // untouched local copy look edited and let it win a later conflict.
                let (hash, clock) = match (loc, st) {
                    (Some(_), Some(s)) => (s.local_hash.as_str(), s.remote_clock),
                    _ => ("", 0),
                };
                ctx.store.upsert_account_sync_item(&row(
                    category, &id, name, &label, hash, clock, "skipped", &reason,
                ))?;
            }
            Err(error) => {
                ctx.store.upsert_account_sync_item(&row(
                    category,
                    st.map_or(id.as_str(), |s| s.item_id.as_str()),
                    name,
                    &label,
                    st.map_or("", |s| s.local_hash.as_str()),
                    st.map_or(0, |s| s.remote_clock),
                    "error",
                    &format!("{error:#}"),
                ))?;
            }
        }
        collapse_duplicates(ctx, &id, name, loc, cands)?;
    }

    if !refresh.is_empty() {
        let (fresh, _) = catalog::collect(category);
        let fresh: BTreeMap<String, String> = fresh
            .into_iter()
            .map(|i| (i.item.clone(), ctx.vault.content_hash(&i.canonical())))
            .collect();
        for r in refresh {
            let hash = fresh.get(&r.name).cloned().unwrap_or_default();
            let status = if hash.is_empty() && r.status == "synced" {
                "removed"
            } else {
                r.status
            };
            if let Some(old) = &r.old_id {
                ctx.store.delete_account_sync_item(category.id(), old)?;
            }
            ctx.store.upsert_account_sync_item(&row(
                category, &r.id, &r.name, &r.label, &hash, r.clock, status, &r.detail,
            ))?;
        }
    }
    Ok(())
}

/// Two ids for one item (old duplicates, or two devices that created it before meeting) collapse
/// onto the smallest id. Only the device that wrote the newest of them acts, so two devices can
/// never tombstone each other's copies.
fn collapse_duplicates(
    ctx: &Ctx,
    canonical: &str,
    name: &str,
    loc: Option<&LocalItem>,
    cands: &[(&String, &Remote)],
) -> Result<()> {
    let dupes: Vec<&(&String, &Remote)> = cands
        .iter()
        .filter(|(id, _)| id.as_str() != canonical)
        .collect();
    if dupes.is_empty() {
        return Ok(());
    }
    let Some((_, newest)) = cands.iter().max_by_key(|(_, r)| (r.clock, r.sender)) else {
        return Ok(());
    };
    if !newest.mine {
        return Ok(());
    }
    let kind = kind_for(name);
    let canonical_is_mine_newest = cands
        .iter()
        .find(|(id, _)| id.as_str() == canonical)
        .is_some_and(|(_, r)| r.mine && r.clock >= newest.clock);
    if !canonical_is_mine_newest {
        if let Some(l) = loc {
            upload(ctx, canonical, name, Some(l), None)?;
        } else {
            return Ok(());
        }
    }
    for (id, _) in dupes {
        ctx.store
            .write_portable_sync_record(ctx.device_id, kind, id, None)?;
    }
    Ok(())
}

pub(super) enum Step {
    Record(u64, &'static str, String),
    Skipped(String),
}

fn upload(
    ctx: &Ctx,
    id: &str,
    name: &str,
    loc: Option<&LocalItem>,
    parent: Option<&str>,
) -> Result<Step> {
    let kind = loc.map_or_else(|| kind_for(name), |l| l.kind);
    match loc {
        Some(local) => {
            let payload = serde_json::to_vec(&build_payload(ctx, id, local, parent)?)?;
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
    Ok(Step::Record(clock, "synced", String::new()))
}

async fn take_remote(
    ctx: &Ctx<'_>,
    category: Category,
    id: Option<&str>,
    payload: Option<&Payload>,
    clock: u64,
    name: &str,
    keep_conflict: bool,
) -> Result<Step> {
    let (Some(payload), Some(id)) = (payload, id) else {
        apply::remove_local(category, name)?;
        return Ok(Step::Record(clock, "synced", String::new()));
    };
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
