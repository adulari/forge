//! Migration #52, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #52: the approval history behind "you've allowed this shape N times here".
///
/// One row per answered permission prompt. Purely additive.
pub(super) fn migration_0052(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS permission_decision (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    project    TEXT NOT NULL,
    tool       TEXT NOT NULL,
    shape      TEXT NOT NULL DEFAULT '',
    decision   TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_permission_decision_shape ON permission_decision(project, tool, shape, id);",
    )
}
