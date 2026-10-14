//! Public handoff capsule records.
//!
//! These records are the stable data boundary for encrypted session transfer;
//! Store owns their transactional persistence and lifecycle.

use forge_types::{Role, ToolCall, Visibility};
use serde::{Deserialize, Serialize};

/// Immutable provenance for a session imported from an encrypted handoff capsule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSessionMetadata {
    pub session_id: String,
    pub source_session_id: String,
    pub source_device_id: [u8; 16],
    pub capsule_id: String,
    pub base_commit: String,
    pub worktree_path: String,
    pub imported_at: i64,
}

/// Portable session state embedded in an end-to-end encrypted handoff capsule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffSessionExport {
    pub version: u8,
    pub source_session_id: String,
    pub title: Option<String>,
    pub permission_mode: String,
    pub messages: Vec<HandoffMessage>,
    pub checkpoints: Vec<HandoffCheckpoint>,
    /// The persisted compaction summary. Without it the destination resumed only the active tail
    /// and silently lost everything the session had compacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_summary: Option<String>,
    /// The source host fingerprint (opaque `forge_core::host_move` JSON), so the destination can
    /// tell the model the session moved and map the source's paths onto its own layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffMessage {
    pub seq: i64,
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    pub visibility: Visibility,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffCheckpoint {
    pub label: Option<String>,
    pub seq: i64,
}

/// Result of importing a portable handoff session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffSessionImport {
    pub session_id: String,
    pub remapped: bool,
}

/// Source provenance supplied while atomically importing a handoff session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffImportProvenance {
    pub source_device_id: [u8; 16],
    pub capsule_id: String,
    pub base_commit: String,
    pub imported_at: i64,
}
