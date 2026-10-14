//! Migration #41, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #41: prompt-cache attribution on `usage`.
///
/// Each main-loop call records how many leading wire messages it shared with the previous request
/// to the same model (`prefix_reused_msgs` of `prefix_total_msgs`), whether the tool list changed,
/// and a label naming why the cache could not be reused (`append`, `harness_rewrite@<idx>(<kind>)`,
/// `floor_advance`, `provider_eviction`, ...). All nullable: side calls, bridges and every
/// historical row have no fingerprint. Purely additive.
pub(super) fn migration_0041(conn: &Connection) -> rusqlite::Result<()> {
    for sql in [
        "ALTER TABLE usage ADD COLUMN prefix_reused_msgs INTEGER",
        "ALTER TABLE usage ADD COLUMN prefix_total_msgs INTEGER",
        "ALTER TABLE usage ADD COLUMN tools_changed INTEGER",
        "ALTER TABLE usage ADD COLUMN cache_label TEXT",
    ] {
        add_column_if_missing(conn, sql)?;
    }
    Ok(())
}
