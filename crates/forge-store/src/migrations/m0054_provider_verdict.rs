//! Migration #54, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #54: `provider_verdict`, one row per provider whose key the provider refused, so the
/// providers screens can say "Key expired — renew" instead of models quietly going missing.
/// Purely additive.
pub(super) fn migration_0054(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_verdict (
    provider   TEXT PRIMARY KEY,
    status     TEXT NOT NULL,
    reason     TEXT NOT NULL DEFAULT '',
    first_seen INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL
);",
    )
}
