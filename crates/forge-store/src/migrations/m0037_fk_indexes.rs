//! Migration #37, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #37: index the `ON DELETE CASCADE` child columns that had no leading index, plus the
/// prompt-history lookup.
///
/// SQLite enforces a cascade by running `DELETE FROM child WHERE fk = ?` per deleted parent row.
/// `tool_call.message_id` had no index, so every deleted `message` full-scanned the 278 MB
/// `tool_call` table: deleting ONE 1,438-message session took 114 s (0.03 s once indexed), all of it
/// under the write lock. `workflow_run.session_id` has the same shape.
///
/// `idx_session_cwd` + `idx_message_session_role` let the prompt-history query
/// (`s.cwd = ? AND m.role = 'user' ORDER BY m.created_at DESC`) seek instead of scanning and
/// sorting every message in the store (1.7 s cold → 9 ms).
pub(super) fn migration_0037(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tool_call_message ON tool_call(message_id);
         CREATE INDEX IF NOT EXISTS idx_workflow_run_session ON workflow_run(session_id);
         CREATE INDEX IF NOT EXISTS idx_session_cwd ON session(cwd);
         CREATE INDEX IF NOT EXISTS idx_message_session_role ON message(session_id, role, created_at);",
    )
}
