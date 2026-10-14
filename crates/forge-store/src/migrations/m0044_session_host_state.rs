//! Migration #44, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #44: `session.host_state`, the JSON host fingerprint a session was last grounded on.
///
/// A session moved to another machine (store copy, Anywhere sync or handoff) kept describing the
/// first machine's filesystem, and nothing told the model the layout had changed. Resume compares
/// this fingerprint with the current host and injects one persisted notice when they differ
/// (docs/features/session-host-move.md). NULL means "never recorded" (a session from before this
/// column), which resume treats as unknown rather than as "same host". Purely additive.
pub(super) fn migration_0044(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN host_state TEXT")
}
