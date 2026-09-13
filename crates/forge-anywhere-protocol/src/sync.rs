//! Plaintext sync records that are always carried inside an encrypted envelope.

use serde::{Deserialize, Serialize};

use crate::DeviceId;

/// Maximum size of one complete encoded sync-record envelope.
///
/// Sync was the only Anywhere transport without a cap — commands, shares, capsules and relay blobs
/// all check before sending. An oversized record was reserved anyway, the service answered a bare
/// HTTP 400, and because the outbox drains strictly in `id` order and aborts the cycle on the first
/// error, that one record blocked every later one indefinitely. A 14.9 MB tool result (sealed into
/// a ~51 MiB envelope, since `serde_json` encodes `Vec<u8>` as a decimal array at ~3.6x) stalled a
/// host's entire sync for three days. Matches the download-side limit so both directions agree.
pub const MAX_SYNC_UPLOAD_ENVELOPE_BYTES: u64 = 32 * 1024 * 1024;

/// V1 record classes eligible for encrypted cloud sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncRecordKind {
    Session,
    Message,
    Checkpoint,
    ToolCall,
    RoutingDecision,
    Usage,
    Compaction,
    Memory,
    UserSetting,
    Command,
    Skill,
    Agent,
    Workflow,
    File,
}

/// Whether this revision updates a record or deletes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOperation {
    Upsert,
    Tombstone,
}

/// An idempotent, encrypted sync journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRecord {
    pub stable_id: String,
    pub kind: SyncRecordKind,
    pub revision: u64,
    pub logical_clock: u64,
    pub device_id: DeviceId,
    pub operation: SyncOperation,
    /// Required for files; allows divergence to create a conflict copy instead of overwriting.
    pub base_hash: Option<[u8; 32]>,
    pub content_hash: [u8; 32],
    /// Record content. Empty for tombstones.
    #[serde(default)]
    pub payload: Vec<u8>,
}

impl SyncRecord {
    /// Compare deterministic mutable-metadata clocks: `(logical_clock, device_id)`.
    pub fn is_newer_than(&self, other: &Self) -> bool {
        (self.logical_clock, self.device_id) > (other.logical_clock, other.device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_breaks_logical_clock_ties() {
        let first = SyncRecord {
            stable_id: "setting/theme".into(),
            kind: SyncRecordKind::UserSetting,
            revision: 1,
            logical_clock: 4,
            device_id: [1; 16],
            operation: SyncOperation::Upsert,
            base_hash: None,
            content_hash: [0; 32],
            payload: Vec::new(),
        };
        let mut second = first.clone();
        second.device_id = [2; 16];
        assert!(second.is_newer_than(&first));
    }
}
