//! Migration #46, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #46: `usage.api_calls`.
///
/// A CLI-bridge turn runs the whole agent loop inside one provider call, so it stores one usage
/// row for what may be dozens of model round trips. Benchmarks that count "model calls" as usage
/// rows read 1 for such a turn. The bridge reports the real count and it is kept here. NULL means
/// the row is one call, which is every row written before this column existed and every direct
/// provider call. Purely additive.
pub(super) fn migration_0046(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "ALTER TABLE usage ADD COLUMN api_calls INTEGER")
}
