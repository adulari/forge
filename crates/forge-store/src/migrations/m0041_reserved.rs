//! Migration #41, reserved for the cache-instrument work package (WP-A).

use super::*;

/// Migration #41: intentionally empty on this branch. The number belongs to the WP-A cache
/// instrument; its real body replaces this one when that branch lands. It exists only so
/// [`super::migration_0042`] sits at index 42 and `SCHEMA_VERSION` matches the list length.
pub(super) fn migration_0041(_conn: &Connection) -> rusqlite::Result<()> {
    Ok(())
}
