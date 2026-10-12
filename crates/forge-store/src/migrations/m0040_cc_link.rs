//! Migration #40, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #40: `cc_session_link`, the bookkeeping that lets one conversation move between a
/// Forge session and a Claude Code transcript repeatedly without duplicating or losing history.
///
/// A row says "this Forge session and this Claude Code session are the same conversation, and
/// everything up to `cc_synced_bytes` / `cc_last_uuid` on the Claude Code side and up to
/// `forge_synced_seq` on the Forge side has already been mirrored to the other". One side maps to
/// at most one other (both ids are unique). Purely additive: no existing table is touched.
pub(super) fn migration_0040(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cc_session_link (
            forge_session_id TEXT PRIMARY KEY REFERENCES session(id) ON DELETE CASCADE,
            cc_session_id    TEXT NOT NULL UNIQUE,
            cc_path          TEXT NOT NULL,
            cc_synced_bytes  INTEGER NOT NULL DEFAULT 0,
            cc_last_uuid     TEXT,
            forge_synced_seq INTEGER NOT NULL DEFAULT -1,
            forge_summary    TEXT,
            last_target      TEXT,
            updated_at       INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         )",
    )
}
