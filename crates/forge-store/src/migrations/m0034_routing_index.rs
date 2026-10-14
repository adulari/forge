//! Migration #34, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #34: index `routing_decision(message_id)`.
///
/// `session_models` and the fleet presence listing join `routing_decision` to `message` per
/// session. With no index on the join column the planner walked `routing_decision` for every
/// message row: resuming a 23k-message session spent 5.5 s in that one statement before the
/// first frame (found with `FORGE_SQL_PROFILE`). Same shape as `idx_usage_message`.
pub(super) fn migration_0034(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_routing_decision_message ON routing_decision(message_id)",
    )
}
