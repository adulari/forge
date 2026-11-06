//! Migration #53, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #53: `session.goal_state`, the JSON record of a `/goal` that carries a spend
/// envelope, so a goal paused at its cap (or running when the daemon went down) is still there,
/// resumable, after a restart. NULL means "no goal to restore". Purely additive.
pub(super) fn migration_0053(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN goal_state TEXT")
}
