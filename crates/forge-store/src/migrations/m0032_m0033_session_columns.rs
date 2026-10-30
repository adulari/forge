//! Migrations #32 and #33, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #33: whether a session has released its subagents from the active model pin.
///
/// `mesh.subagents.inherit_pin` is a config-file default, so the only way to let one pinned
/// session fan out onto other models was to edit the config and restart. `/subagents free` sets
/// this per session instead; like the model and effort pins it lived nowhere durable, so a daemon
/// restart or a resume would silently drag the children back onto the parent's pin. NULL = follow
/// the config default.
pub(super) fn migration_0033(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE session ADD COLUMN subagent_pin_free INTEGER",
    )
}

/// Migration #32: the loopback control endpoint a terminal-local session publishes for the daemon.
///
/// Terminal sessions have been visible in the fleet since [`migration_0026`], but only as
/// read-only rows: there was no way for the phone or desktop app to answer a permission prompt
/// raised on the laptop. A terminal chat now opens a token-gated control channel on loopback and
/// records its URL here; `forge serve` proxies an app's WebSocket to whatever this column holds.
///
/// Cleared on clean exit, and — like the rest of the presence row — only trusted while
/// `local_last_seen` is fresh, so a killed terminal's dead port is never handed to a client.
pub(super) fn migration_0032(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE session ADD COLUMN local_control_url TEXT",
    )
}
