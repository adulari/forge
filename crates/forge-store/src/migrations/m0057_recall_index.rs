//! Migration #57, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #57: `message_fts`, the full-text index behind the `recall` tool and history search.
/// It starts empty on purpose. A session's rows are indexed the first time it is searched
/// (`recall_store.rs`), so upgrading never scans a multi-gigabyte store. `rowid` is the message's
/// own rowid and `message_id` guards against SQLite reusing one after a delete. Purely additive.
pub(super) fn migration_0057(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS message_fts USING fts5(
    content,
    session_id UNINDEXED,
    message_id UNINDEXED,
    tokenize = 'porter unicode61'
);",
    )
}
