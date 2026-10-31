//! Migration #50, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #50: per-project scratchpad and to-do list (the Workbench Notes surface).
///
/// `project_note` holds one markdown scratchpad per project root; `project_todo` holds the
/// checklist items, each optionally linked to the session started from it. Purely additive.
pub(super) fn migration_0050(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_note (
    project    TEXT PRIMARY KEY,
    body       TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS project_todo (
    id         TEXT PRIMARY KEY,
    project    TEXT NOT NULL,
    text       TEXT NOT NULL,
    done       INTEGER NOT NULL DEFAULT 0,
    position   INTEGER NOT NULL DEFAULT 0,
    session_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_project_todo_project ON project_todo(project, position);",
    )
}
