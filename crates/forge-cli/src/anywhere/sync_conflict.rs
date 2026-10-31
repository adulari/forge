//! A reserved (kind, id, revision) is immutable on the service, so a second device writing the same
//! portable record at the same revision gets a conflict. Left alone that row fails first in the
//! strict-order outbox on every pass and nothing behind it ever uploads.

use anyhow::Result;
use forge_store::{Store, SyncJournalEntry};

const PORTABLE_KINDS: [&str; 4] = ["user_setting", "skill", "command", "agent"];

pub(super) fn is_revision_conflict(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("code").and_then(|c| c.as_str().map(str::to_owned)))
        .is_some_and(|code| code == "sync_revision_conflict")
}

/// Retires the rejected journal row. When this device's record is still the stored winner for the
/// id, it is re-journaled at the next revision so it reaches the account; when another device's
/// record replaced it, that record is already what this device holds and nothing is lost.
/// Returns whether the entry was handled.
pub(super) fn supersede(
    store: &Store,
    local_device: [u8; 16],
    entry: &SyncJournalEntry,
    now_secs: i64,
) -> Result<bool> {
    if !PORTABLE_KINDS.contains(&entry.record_kind.as_str()) {
        return Ok(false);
    }
    store.mark_sync_journal_uploaded(&[entry.id], now_secs)?;
    let Some(record) = store.portable_sync_record(&entry.record_kind, &entry.stable_id)? else {
        return Ok(true);
    };
    if record.sender_device_id == local_device && record.logical_clock <= entry.revision {
        let payload = (!record.deleted).then_some(record.payload.as_slice());
        store.write_portable_sync_record(
            local_device,
            &entry.record_kind,
            &entry.stable_id,
            payload,
        )?;
    }
    Ok(true)
}
