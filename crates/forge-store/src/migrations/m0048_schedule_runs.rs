//! Migration #48, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #48: per-schedule policy columns and the `schedule_run` history table.
///
/// `forge schedule` ticks were fire-and-forget `forge run` processes, so nothing recorded whether a
/// tick ran, failed or was cut off by a restart. The `schedule` row gains the knobs a tick now
/// honours (time zone, runtime cap, iteration cap, retries, misfire policy, stop-on-error, a
/// natural-language stop condition) and every tick writes a `schedule_run` row. Purely additive.
pub(super) fn migration_0048(conn: &Connection) -> rusqlite::Result<()> {
    for sql in [
        "ALTER TABLE schedule ADD COLUMN tz TEXT",
        "ALTER TABLE schedule ADD COLUMN max_runtime_secs INTEGER",
        "ALTER TABLE schedule ADD COLUMN max_iterations INTEGER",
        "ALTER TABLE schedule ADD COLUMN retries INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE schedule ADD COLUMN backoff_secs INTEGER NOT NULL DEFAULT 60",
        "ALTER TABLE schedule ADD COLUMN misfire TEXT NOT NULL DEFAULT 'run'",
        "ALTER TABLE schedule ADD COLUMN stop_on_error INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE schedule ADD COLUMN stop_when TEXT",
    ] {
        add_column_if_missing(conn, sql)?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schedule_run (
            id          TEXT PRIMARY KEY,
            schedule_id TEXT NOT NULL REFERENCES schedule(id) ON DELETE CASCADE,
            started_at  INTEGER NOT NULL,
            finished_at INTEGER,
            status      TEXT NOT NULL,
            trigger     TEXT NOT NULL DEFAULT 'timer',
            attempt     INTEGER NOT NULL DEFAULT 1,
            session_id  TEXT,
            pid         INTEGER,
            summary     TEXT,
            error       TEXT,
            read        INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_schedule_run_schedule ON schedule_run(schedule_id, started_at);",
    )
}
