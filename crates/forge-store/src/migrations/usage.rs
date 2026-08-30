use super::*;

/// Make `usage.cached_input_tokens` nullable so "the provider does not report prompt-cache hits"
/// stops being recorded as the factual claim "nothing was cached".
///
/// The column shipped as `INTEGER NOT NULL DEFAULT 0`, which SQLite cannot relax in place, so the
/// table is rebuilt. Existing values are carried over verbatim with one evidence-backed exception:
/// rows from the `codex-cli` bridge. Forge's bridge parser only ever read Claude's
/// `cache_read_input_tokens`, a key Codex never emits (Codex uses `cached_input_tokens`), so every
/// zero on those rows is a dropped field rather than a measurement. They become NULL. Zeros from
/// providers that DO report caching — including `codex-oauth`, which always parsed its own field —
/// are genuine and are left alone.
pub(super) fn migration_0030(conn: &Connection) -> rusqlite::Result<()> {
    let nullable = conn
        .prepare(
            "SELECT \"notnull\" FROM pragma_table_info('usage') WHERE name = 'cached_input_tokens'",
        )?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|notnull| notnull == 0)
        .unwrap_or(false);
    if nullable {
        return Ok(());
    }
    let foreign_keys: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if foreign_keys {
        conn.pragma_update(None, "foreign_keys", false)?;
    }
    let result = conn.execute_batch(
        "DROP TABLE IF EXISTS usage_new;
         CREATE TABLE usage_new (
             id            TEXT PRIMARY KEY,
             message_id    TEXT NOT NULL REFERENCES message(id) ON DELETE CASCADE,
             provider      TEXT,
             model         TEXT,
             input_tokens  INTEGER NOT NULL,
             cached_input_tokens INTEGER,
             output_tokens INTEGER NOT NULL,
             cost_usd      REAL NOT NULL,
             created_at    INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         );
         INSERT INTO usage_new
             (id, message_id, provider, model, input_tokens, cached_input_tokens, output_tokens, cost_usd, created_at)
         SELECT id, message_id, provider, model, input_tokens,
                CASE WHEN provider = 'codex-cli' AND cached_input_tokens = 0
                     THEN NULL ELSE cached_input_tokens END,
                output_tokens, cost_usd, created_at
         FROM usage;
         DROP TABLE usage;
         ALTER TABLE usage_new RENAME TO usage;
         CREATE INDEX IF NOT EXISTS idx_usage_created_at ON usage(created_at);
         CREATE INDEX IF NOT EXISTS idx_usage_message ON usage(message_id);",
    );
    if foreign_keys {
        conn.pragma_update(None, "foreign_keys", true)?;
    }
    result
}
