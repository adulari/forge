//! Migration #38, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #38: `message.provider_items_json`, the encrypted provider items (Codex reasoning)
/// replayed ahead of an assistant reply. Nullable and additive: older rows and every non-Codex
/// reply stay NULL. Without it a resumed session lost the reasoning chain and the prompt-cache
/// prefix, because the items only lived in the in-memory transcript.
pub(super) fn migration_0038(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE message ADD COLUMN provider_items_json TEXT",
    )
}
