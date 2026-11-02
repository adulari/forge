//! Reading the service's error body, and recovering outbox rows whose upload can never succeed.
//!
//! The service wraps every error as `{"version":1,"error":{"code":..,"message":..}}`. This client
//! used to read a top-level `code`, so it never matched a real response: the revision-conflict
//! handler was dead code and the generic messages for read-only, expired and denied never fired.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use forge_config::account_sync::SyncOutboxFailure;
use forge_store::{RejournalOutcome, Store, SyncJournalEntry};
use reqwest::StatusCode;

/// The service's code and message from an error body, in either the nested or the flat shape.
pub(super) fn error_fields(body: &[u8]) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return (None, None);
    };
    let field = |name: &str| {
        value
            .get("error")
            .and_then(|e| e.get(name))
            .or_else(|| value.get(name))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    (field("code"), field("message"))
}

/// The code only when it is a plain identifier, so an untrusted body can never put free text in a
/// log line, a status document or a terminal.
pub(super) fn error_code(body: &[u8]) -> Option<String> {
    error_fields(body).0.filter(|code| {
        !code.is_empty()
            && code.len() <= 64
            && code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}

/// A non-success answer from the sync endpoints, keeping the service's code for the status
/// surfaces next to the human message.
#[derive(Debug)]
pub(super) struct ServiceRejected {
    pub code: Option<String>,
    pub journal_id: i64,
    message: String,
}

impl std::fmt::Display for ServiceRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceRejected {}

pub(super) fn rejected(status: StatusCode, body: &[u8], journal_id: i64) -> anyhow::Error {
    anyhow::Error::new(ServiceRejected {
        code: error_code(body),
        journal_id,
        message: super::service_error(status, body),
    })
}

/// How many times in a row a completion has failed verification for one row. The first failures
/// are retried as-is, because re-sending the same bytes repairs a half-written object.
const VERIFICATION_RETRIES: u32 = 3;
static VERIFICATION_STRIKES: Mutex<Option<HashMap<i64, u32>>> = Mutex::new(None);

pub(super) fn verification_strike(journal_id: i64) -> u32 {
    let mut guard = VERIFICATION_STRIKES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let count = guard
        .get_or_insert_with(HashMap::new)
        .entry(journal_id)
        .or_insert(0);
    *count += 1;
    *count
}

pub(super) fn verification_retries_exhausted(strikes: u32) -> bool {
    strikes >= VERIFICATION_RETRIES
}

pub(super) fn forget_strikes(journal_id: i64) {
    let mut guard = VERIFICATION_STRIKES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(map) = guard.as_mut() {
        map.remove(&journal_id);
    }
}

/// Retire a row whose reserved revision is permanently unusable and journal its payload again at
/// the record's next revision. The cached envelope goes with the retired row, so the replacement
/// is sealed afresh (new sequence, new ciphertext) and reserved under a new key.
pub(super) fn rejournal(
    store: &Store,
    entry: &SyncJournalEntry,
    code: &str,
    now_secs: i64,
) -> Result<()> {
    let outcome = store.rejournal_sync_entry(entry.id, now_secs)?;
    forget_strikes(entry.id);
    match outcome {
        RejournalOutcome::Rejournaled { new_id, revision } => eprintln!(
            "⚠ Forge Anywhere re-queued {} {} as revision {revision} (journal row {new_id}): the \
             service refused revision {} with {code}",
            entry.record_kind, entry.stable_id, entry.revision
        ),
        RejournalOutcome::Superseded => eprintln!(
            "⚠ Forge Anywhere retired {} {} revision {} ({code}); a newer revision is already queued",
            entry.record_kind, entry.stable_id, entry.revision
        ),
        RejournalOutcome::Gone => {}
    }
    Ok(())
}

/// The outbox backlog and blocked state as one line, for `status` and `doctor`.
pub(super) fn outbox_line(store: &Store) -> Option<String> {
    let backlog = store.sync_outbox_backlog().ok()?;
    super::account_sync::OutboxStatus::from_parts(
        backlog,
        forge_config::account_sync::read_outbox_failure(),
    )
    .line()
}

/// Persist the outcome of an upload pass for `forge anywhere sync status`, `doctor` and the
/// surfaces that read it. Success clears the failure; a failure keeps its first timestamp.
pub(super) fn record_outcome(
    result: &Result<(), String>,
    error: Option<&anyhow::Error>,
    now_ms: u64,
) {
    use forge_config::account_sync::{
        clear_outbox_failure, read_outbox_failure, write_outbox_failure,
    };
    let (Err(message), Some(error)) = (result, error) else {
        clear_outbox_failure();
        return;
    };
    let rejected = error.downcast_ref::<ServiceRejected>();
    let previous = read_outbox_failure();
    write_outbox_failure(&SyncOutboxFailure {
        since_ms: previous.as_ref().map_or(now_ms, |p| p.since_ms),
        updated_at_ms: now_ms,
        code: rejected.and_then(|r| r.code.clone()),
        message: message.clone(),
        journal_id: rejected.map(|r| r.journal_id),
        attempts: previous.map_or(0, |p| p.attempts).saturating_add(1),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nested_service_shape_is_read() {
        let body = br#"{"version":1,"error":{"code":"sync_revision_conflict","message":"x"}}"#;
        assert_eq!(error_code(body).as_deref(), Some("sync_revision_conflict"));
        assert_eq!(
            error_code(br#"{"code":"replay_detected"}"#).as_deref(),
            Some("replay_detected")
        );
    }

    #[test]
    fn a_code_with_free_text_is_never_echoed() {
        assert_eq!(error_code(br#"{"error":{"code":"a b\u001b[31m"}}"#), None);
        assert_eq!(error_code(b"<html>"), None);
    }
}
