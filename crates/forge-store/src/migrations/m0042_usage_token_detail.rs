//! Migration #42, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #42: `usage.cache_write_tokens` and `usage.reasoning_tokens`.
///
/// Bench reports need the cache-write share to explain cost on Anthropic/Bedrock routes, where a
/// cache write is billed at a premium, and the reasoning share of output to judge reasoning-effort
/// changes. Both are nullable for the same reason `cached_input_tokens` is: NULL means the provider
/// did not report the split, which is not a claim that it was zero. Purely additive.
pub(super) fn migration_0042(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE usage ADD COLUMN cache_write_tokens INTEGER",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE usage ADD COLUMN reasoning_tokens INTEGER",
    )
}
