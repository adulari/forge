//! Migration #43, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #43: `bench_solve`, one scored benchmark run per row, recorded by
/// `forge bench report --record` so the mesh can rank on measured cost per solve.
///
/// Keyed by `source` (the metrics sidecar the run came from), so recording the same report twice
/// replaces the row instead of counting the run twice. `cost_usd` is notional: zero-cost calls are
/// priced at list rates so subscription and metered routes share one axis. Purely additive.
pub(super) fn migration_0043(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bench_solve (
            source      TEXT PRIMARY KEY,
            model       TEXT NOT NULL,
            attempts    INTEGER NOT NULL,
            solved      INTEGER NOT NULL,
            cost_usd    REAL NOT NULL,
            recorded_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         );
         CREATE INDEX IF NOT EXISTS idx_bench_solve_model ON bench_solve(model);",
    )
}
