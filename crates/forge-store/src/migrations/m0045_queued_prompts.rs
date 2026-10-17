//! Migration #45, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #45: `session.queued_prompts`, the JSON array of prompts a daemon-hosted session had
/// queued behind its running turn.
///
/// The queue lived only in the driver's memory, so a daemon restart silently dropped every prompt
/// the user (or a fleet peer) had lined up. The driver now mirrors the queue here whenever it
/// changes and restores it when the session is resumed. NULL means "nothing queued". Purely
/// additive.
pub(super) fn migration_0045(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN queued_prompts TEXT")
}
