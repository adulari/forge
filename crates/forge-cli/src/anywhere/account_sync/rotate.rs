//! After a device is revoked the Account Data Key rotates, but the revoked device keeps every
//! ciphertext it already downloaded. Synced keys are therefore re-sealed under the new epoch and
//! the person is told which ones to rotate at the provider.

use anyhow::{Context, Result};
use forge_store::Store;

use super::reconcile::{build_payload, recorded_id, remotes, Ctx};
use super::vault::Vault;
use super::{catalog, Body, Category};
use crate::anywhere::{decode_hex_array, LocalState};

/// Labels only. Re-uploads the key items this device holds under the current epoch.
pub(crate) fn reseal_keys(store: &Store, state: &LocalState) -> Result<Vec<String>> {
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
    let config = forge_config::load()?;
    let mut labels: Vec<String> = Vec::new();
    for remote in remotes(&ctx, Category::Keys)?.values() {
        if let Some(payload) = &remote.payload {
            if payload.sealed.is_some() {
                labels.push(payload.label.clone());
            }
        }
    }
    let (locals, _) = catalog::collect(Category::Keys);
    for local in locals.iter().filter(|l| matches!(l.body, Body::Secret(_))) {
        labels.push(local.label.clone());
        if Category::Keys.enabled(&config.anywhere.account_sync) {
            let id = recorded_id(&ctx, Category::Keys, &local.item);
            let payload = serde_json::to_vec(&build_payload(&ctx, &id, local, None)?)?;
            store.write_portable_sync_record(device_id, local.kind, &id, Some(&payload))?;
        }
    }
    labels.sort();
    labels.dedup();
    Ok(labels)
}

pub(crate) fn print_checklist(labels: &[String]) {
    if labels.is_empty() {
        return;
    }
    println!("Keys synced through this account were downloadable by the revoked device.");
    println!("Rotate these at their providers:");
    for label in labels {
        println!("  - {label}");
    }
}

/// Best effort: the revocation itself has already succeeded.
pub(crate) fn after_revoke(state: &LocalState, target_is_local: bool) {
    if target_is_local {
        return;
    }
    match crate::open_store().and_then(|store| reseal_keys(&store, state)) {
        Ok(labels) => print_checklist(&labels),
        Err(error) => eprintln!("⚠ could not re-seal synced keys under the new epoch: {error:#}"),
    }
}
