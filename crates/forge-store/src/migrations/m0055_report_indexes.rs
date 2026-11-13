//! Migration #55, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #55: covering indexes for the model report card (`model_report_store.rs`).
/// `tool_call` keeps `result_json` ahead of `status`, `permission` and `created_at`, so without an
/// index every in-window row's result overflow chain was walked just to read those columns; the
/// index answers the report from its own pages. `mesh_outcome` only had `(model, completed_at)`,
/// which cannot serve a filter on `completed_at` alone. Purely additive; a table a hand-built
/// older store lacks is skipped (a real store has both).
pub(super) fn migration_0055(conn: &Connection) -> rusqlite::Result<()> {
    for (table, index) in [
        (
            "tool_call",
            "CREATE INDEX IF NOT EXISTS idx_tool_call_report
                 ON tool_call(created_at, message_id, status, permission)",
        ),
        (
            "mesh_outcome",
            "CREATE INDEX IF NOT EXISTS idx_mesh_outcome_completed
                 ON mesh_outcome(completed_at)",
        ),
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |r| r.get(0),
        )?;
        if exists {
            conn.execute_batch(index)?;
        }
    }
    Ok(())
}
