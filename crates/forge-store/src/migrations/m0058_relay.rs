//! Migration #58, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #58: `session_relay`, the lineage of fresh-worker hand-offs (a goal that wedged or
/// wore its session out continues in a new session on the same worktree), and a compaction count
/// on `session_compaction` so "compacted N times" survives a restart. Purely additive.
pub(super) fn migration_0058(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_relay (
    successor_id   TEXT PRIMARY KEY,
    predecessor_id TEXT NOT NULL,
    reason         TEXT NOT NULL,
    detail         TEXT NOT NULL DEFAULT '',
    brief          TEXT NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_relay_predecessor ON session_relay(predecessor_id);",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE session_compaction ADD COLUMN compactions INTEGER NOT NULL DEFAULT 1",
    )
}
