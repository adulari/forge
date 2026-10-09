//! Migration #38, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #38: `message.provider_items_json`, the encrypted provider items (Codex reasoning)
/// replayed ahead of an assistant reply. Nullable and additive: older rows and every non-Codex
/// reply stay NULL. Without it a resumed session lost the reasoning chain and the prompt-cache
/// prefix, because the items only lived in the in-memory transcript.
pub(super) fn migration_0038(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE message ADD COLUMN provider_items_json TEXT",
    )
}

/// Migration #36: a real marker for a harness-injected continuation/empty-response nudge.
///
/// These are synthesized by forge-core (e.g. `EMPTY_DIFF_NUDGE`, the empty-response and
/// followup-intent nudges) and persisted with `role='user'` so a thinking-mode/tool-calling
/// provider's next request still ends on a legal user turn — but that means the ONLY signal any
/// client had for telling a nudge apart from a real, person-typed message was matching the exact,
/// well-known nudge text (see mobile's `lib/harnessNudge.ts`), which breaks the moment a nudge is
/// reworded or a new one is added. `nudge=1` marks the row at the point it's created, independent
/// of its wording, while the message itself is still stored/replayed as an ordinary `Role::User`
/// turn for the model.
pub(super) fn migration_0036(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE message ADD COLUMN nudge INTEGER NOT NULL DEFAULT 0",
    )
}
