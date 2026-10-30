//! Migration #49, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #49: per-item reconcile state for Anywhere account sync (keys, skills, settings, MCP
/// servers, instructions). `local_hash` is a keyed hash of what this device last uploaded or
/// applied, so a changed local value, a removal and an incoming change can be told apart without
/// ever storing a value. Purely additive.
pub(super) fn migration_0049(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS anywhere_account_sync_item (
    category     TEXT NOT NULL,
    item_id      TEXT NOT NULL,
    item         TEXT NOT NULL DEFAULT '',
    label        TEXT NOT NULL DEFAULT '',
    local_hash   TEXT NOT NULL DEFAULT '',
    remote_clock INTEGER NOT NULL DEFAULT 0,
    applied_at   INTEGER NOT NULL DEFAULT 0,
    status       TEXT NOT NULL DEFAULT 'synced',
    detail       TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (category, item_id)
);",
    )
}
