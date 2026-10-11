//! Migration #39, kept beside the list so `migrations.rs` stays under the size guard.

use super::*;

/// Migration #39: the end-to-end notification key an iOS device registers next to its APNs token
/// (ADR-0012 addendum). `e2e_p256dh` is the device's P-256 public key and `e2e_auth` its 16-byte
/// auth secret, both base64url; the daemon seals notice text to them so the hosted relay and Apple
/// only ever carry ciphertext. Nullable and additive: a device that never registered a key (an
/// older app build) keeps the generic lock-screen alert.
pub(crate) fn migration_0039(conn: &Connection) -> rusqlite::Result<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='apns_subscription'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        // Hand-built historic fixtures only: migration #10 creates the table on every real store.
        return Ok(());
    }
    add_column_if_missing(
        conn,
        "ALTER TABLE apns_subscription ADD COLUMN e2e_p256dh TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE apns_subscription ADD COLUMN e2e_auth TEXT",
    )
}
