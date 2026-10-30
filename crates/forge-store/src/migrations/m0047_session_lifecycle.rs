//! Migration #47, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #47: `session.starred` and `session.done_at`.
///
/// Starred sessions float to the top of every list; a session marked done sinks into a collapsed
/// section. `done_at` is a unix timestamp so a client can tell when, and NULL means not done.
/// Purely additive.
pub(super) fn migration_0047(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE session ADD COLUMN starred INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN done_at INTEGER")
}
