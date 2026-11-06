//! Migration #51, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #51: index `message(session_id, created_at)` so a session's newest message is one
/// B-tree seek. `MAX(created_at) WHERE session_id = ?` (every session's `last_activity`) otherwise
/// walked every index entry of that session. Purely additive.
pub(super) fn migration_0051(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_message_session_created ON message(session_id, created_at)",
    )
}
