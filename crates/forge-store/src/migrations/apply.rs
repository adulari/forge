//! Applying the migration list: version bookkeeping and the pre-release version reconciliation.

use super::*;

/// Lowest `user_version` inside the ambiguous pre-release window (see [`run_migrations`]). Equals
/// the first version the unreleased Forge Anywhere branch stamped, i.e. one past the last
/// unambiguously public version at the time that branch forked.
pub(crate) const ANYWHERE_PRERELEASE_MIN_VERSION: i64 = 18;

/// Highest `user_version` the unreleased Forge Anywhere branch stamped.
pub(crate) const ANYWHERE_PRERELEASE_MAX_VERSION: i64 = 21;

/// Whether public migration 18's column is present.
///
/// This is the evidence that separates a database written by a PUBLIC build from one stamped by the
/// unreleased Forge Anywhere branch inside the ambiguous window: that branch forked while
/// [`SCHEMA_VERSION`] was still 17, before [`migration_0018`] existed, and `CREATE TABLE IF NOT
/// EXISTS usage` in the base schema cannot add a column to a table that already exists. So no
/// pre-release database can carry this column, and every public database at 18 or above must.
fn public_v18_marker_present(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare("SELECT 1 FROM pragma_table_info('usage') WHERE name = 'cached_input_tokens'")?
        .exists([])
}

/// Apply every migration the DB hasn't seen yet, bumping `PRAGMA user_version` after each so a
/// crash mid-run resumes cleanly. Refuses (with [`StoreError::SchemaTooNew`]) to open a DB written
/// by a newer build, rather than silently misreading it.
///
/// Opening an already-current database performs NO write at all, which matters because Forge runs
/// many short-lived processes (CLI subcommands, statusline probes, `mcp-serve`) that would otherwise
/// each contend for the single WAL writer just to open a database they have no work to do on.
pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    debug_assert_eq!(MIGRATIONS.len() as i64, SCHEMA_VERSION);
    let mut current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    // Forge Anywhere's additive tables briefly consumed versions 18-21 on its unreleased feature
    // branch. They now live in the idempotent base schema so the previous public binary can open a
    // database after rollback. A number in that window is therefore AMBIGUOUS: it may come from that
    // branch (which never ran the public steps 18+) or from a public build that legitimately reached
    // it. Resolve it by replaying the public steps rather than by trusting the number.
    if (ANYWHERE_PRERELEASE_MIN_VERSION..=ANYWHERE_PRERELEASE_MAX_VERSION).contains(&current) {
        // Read the marker BEFORE the replay below adds the column it looks for.
        let from_public_build = public_v18_marker_present(conn)?;
        // Every step from 18 on is additive and idempotent by contract (`CREATE ... IF NOT EXISTS` /
        // `add_column_if_missing`), so replaying them is a pure no-op — and writes nothing — on a
        // database that really is at `current`, while a pre-release database that merely reused these
        // numbers gets the public tables/columns it is missing. The previous form rewound
        // `user_version` to 17 instead, which wrote page 1 twice on EVERY open of an already-migrated
        // database and made startup fail with "database is locked" whenever a long lattice write held
        // the writer.
        for step in ANYWHERE_PRERELEASE_MIN_VERSION..=current.min(SCHEMA_VERSION) {
            MIGRATIONS[(step - 1) as usize](conn)?;
        }
        // A version above what this build ships, inside the window, and WITHOUT the public marker can
        // only have come from the pre-release branch — renumber it down once so it stops tripping the
        // refusal below. With the marker present the number is trustworthy, so a database from a
        // genuinely newer public build is refused instead of being silently downgraded.
        if current > SCHEMA_VERSION && !from_public_build {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            current = SCHEMA_VERSION;
        }
    }
    if current > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for (v, migrate) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        migrate(conn)?;
        conn.pragma_update(None, "user_version", (v + 1) as i64)?;
    }
    Ok(())
}
