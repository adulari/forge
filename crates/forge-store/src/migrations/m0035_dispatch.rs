//! Migration #35, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #35: plan and dispatch (`forge_core::dispatch`). A coordinator session proposes a
/// split of the user's request into work items; the user approves it; the daemon starts one
/// session per item and tracks each to completion. The proposal and every item's progress must
/// outlive the board and the daemon process, so they live here rather than in memory.
pub(super) fn migration_0035(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dispatch (
            id                     TEXT PRIMARY KEY,
            coordinator_session_id TEXT NOT NULL,
            cwd                    TEXT NOT NULL,
            prompt                 TEXT NOT NULL,
            summary                TEXT NOT NULL DEFAULT '',
            status                 TEXT NOT NULL,   -- planning | proposed | running | done | cancelled
            worktree               INTEGER NOT NULL DEFAULT 1,
            permission_mode        TEXT,
            max_running            INTEGER NOT NULL DEFAULT 4,
            max_items              INTEGER NOT NULL DEFAULT 8,
            created_at             INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            updated_at             INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         );
         CREATE INDEX IF NOT EXISTS idx_dispatch_coordinator ON dispatch(coordinator_session_id);
         CREATE INDEX IF NOT EXISTS idx_dispatch_status ON dispatch(status);
         CREATE TABLE IF NOT EXISTS dispatch_item (
            dispatch_id TEXT NOT NULL REFERENCES dispatch(id) ON DELETE CASCADE,
            idx         INTEGER NOT NULL,   -- 1-based position in the proposal
            title       TEXT NOT NULL,
            prompt      TEXT NOT NULL,
            depends_on  TEXT NOT NULL DEFAULT '[]',   -- JSON array of 1-based indices
            status      TEXT NOT NULL,
            session_id  TEXT,
            outcome     TEXT,
            started_at  INTEGER,
            finished_at INTEGER,
            PRIMARY KEY (dispatch_id, idx)
         );
         CREATE INDEX IF NOT EXISTS idx_dispatch_item_session ON dispatch_item(session_id)",
    )
}
