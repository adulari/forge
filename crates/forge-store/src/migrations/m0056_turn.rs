//! Migration #56, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #56: `turn`, one row per prompt-started turn with the reason it ended. Until now the
/// store kept no end reason: a failed, stalled, cancelled or crashed turn was only a raw
/// transcript tail. `ended_at` stays NULL while the turn runs, which is also how a crash shows.
/// Purely additive.
pub(super) fn migration_0056(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS turn (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT NOT NULL,
    prompt_seq      INTEGER NOT NULL,
    started_at      INTEGER NOT NULL,
    ended_at        INTEGER,
    end_reason      TEXT,
    detail_json     TEXT,
    last_role_shape TEXT
);
CREATE INDEX IF NOT EXISTS idx_turn_session_started ON turn(session_id, started_at);",
    )
}
