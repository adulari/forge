//! Fleet agent-to-agent messaging persistence (`forge send`, the `message_session` virtual
//! tool). One row per queued message; see [`crate::FleetMessage`] and migration 25.

use super::*;

/// A single message body is rejected above this size (checked in bytes, before the row is
/// written) — keeps a fleet message the same order of magnitude as a normal prompt, not an
/// accidental file dump routed through the wrong tool.
pub const FLEET_MESSAGE_MAX_BYTES: usize = 16 * 1024;

/// Max not-yet-delivered messages one sender may have queued to one target at a time. Bounds an
/// unresponsive/offline target from accumulating unbounded backlog from a single chatty sender;
/// deliberately no other rate limiting.
pub const FLEET_MESSAGE_PENDING_CAP: i64 = 8;

impl Store {
    /// Queue a fleet message for later (or immediate) delivery. Rejects (via
    /// [`StoreError::InvalidValue`]) a body over [`FLEET_MESSAGE_MAX_BYTES`], or a sender/target
    /// pair already at [`FLEET_MESSAGE_PENDING_CAP`] not-yet-delivered messages — both checked
    /// against the SAME connection the insert runs on, so a size/cap decision and the write it
    /// gates never observe different snapshots within one `Store` handle.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_fleet_message(
        &self,
        id: &str,
        sender_kind: &str,
        sender_id: Option<&str>,
        sender_label: &str,
        target_session_id: &str,
        body: &str,
        mode: &str,
    ) -> Result<()> {
        if body.len() > FLEET_MESSAGE_MAX_BYTES {
            return Err(StoreError::InvalidValue(format!(
                "message is {} bytes, exceeds the {FLEET_MESSAGE_MAX_BYTES}-byte limit",
                body.len()
            )));
        }
        let conn = self.lock()?;
        let pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM fleet_message \
             WHERE sender_label = ?1 AND target_session_id = ?2 AND delivered_at IS NULL",
            (sender_label, target_session_id),
            |r| r.get(0),
        )?;
        if pending >= FLEET_MESSAGE_PENDING_CAP {
            return Err(StoreError::InvalidValue(format!(
                "too many pending messages from '{sender_label}' to this target \
                 (cap {FLEET_MESSAGE_PENDING_CAP}) — wait for delivery before sending more"
            )));
        }
        conn.execute(
            "INSERT INTO fleet_message \
             (id, sender_kind, sender_id, sender_label, target_session_id, body, mode) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                id,
                sender_kind,
                sender_id,
                sender_label,
                target_session_id,
                body,
                mode,
            ),
        )?;
        Ok(())
    }

    /// Not-yet-delivered messages queued for `target_session_id`, oldest first — what a daemon
    /// drains into a session's input queue as soon as it (re)joins the live registry.
    pub fn pending_fleet_messages_for(&self, target_session_id: &str) -> Result<Vec<FleetMessage>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, sender_kind, sender_id, sender_label, target_session_id, body, mode, \
                    created_at, delivered_at \
             FROM fleet_message WHERE target_session_id = ?1 AND delivered_at IS NULL \
             ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([target_session_id], map_fleet_message)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Mark one message delivered. Returns `false` if it was already delivered (or doesn't
    /// exist) — a caller retrying a delivery attempt can't double-count it.
    pub fn mark_fleet_message_delivered(&self, id: &str, at: i64) -> Result<bool> {
        let n = self.lock()?.execute(
            "UPDATE fleet_message SET delivered_at = ?1 WHERE id = ?2 AND delivered_at IS NULL",
            (at, id),
        )?;
        Ok(n > 0)
    }
}

fn map_fleet_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<FleetMessage> {
    Ok(FleetMessage {
        id: r.get(0)?,
        sender_kind: r.get(1)?,
        sender_id: r.get(2)?,
        sender_label: r.get(3)?,
        target_session_id: r.get(4)?,
        body: r.get(5)?,
        mode: r.get(6)?,
        created_at: r.get(7)?,
        delivered_at: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_deliver_round_trips_through_the_store() {
        let store = Store::open_in_memory().unwrap();
        let target = store.create_session("/tmp/target", "Default").unwrap();
        store
            .enqueue_fleet_message(
                "m1",
                "session",
                Some("sender-1"),
                "sender-one",
                &target,
                "hello",
                "follow_up",
            )
            .unwrap();

        let pending = store.pending_fleet_messages_for(&target).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "m1");
        assert_eq!(pending[0].sender_label, "sender-one");
        assert_eq!(pending[0].body, "hello");
        assert_eq!(pending[0].mode, "follow_up");
        assert!(pending[0].delivered_at.is_none());

        assert!(store.mark_fleet_message_delivered("m1", 1000).unwrap());
        assert!(store
            .pending_fleet_messages_for(&target)
            .unwrap()
            .is_empty());
        // Delivering an already-delivered (or unknown) id is a no-op, not an error.
        assert!(!store.mark_fleet_message_delivered("m1", 1001).unwrap());
        assert!(!store.mark_fleet_message_delivered("nope", 1001).unwrap());
    }

    #[test]
    fn oversized_message_is_rejected_before_it_touches_the_table() {
        let store = Store::open_in_memory().unwrap();
        let target = store.create_session("/tmp/target", "Default").unwrap();
        let body = "x".repeat(FLEET_MESSAGE_MAX_BYTES + 1);
        let err = store
            .enqueue_fleet_message("m1", "cli", None, "cli", &target, &body, "follow_up")
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidValue(_)));
        assert!(store
            .pending_fleet_messages_for(&target)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn pending_cap_is_enforced_per_sender_per_target() {
        let store = Store::open_in_memory().unwrap();
        let target = store.create_session("/tmp/target", "Default").unwrap();
        for i in 0..FLEET_MESSAGE_PENDING_CAP {
            store
                .enqueue_fleet_message(
                    &format!("m{i}"),
                    "cli",
                    None,
                    "cli",
                    &target,
                    "msg",
                    "follow_up",
                )
                .unwrap();
        }
        let err = store
            .enqueue_fleet_message("over", "cli", None, "cli", &target, "msg", "follow_up")
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidValue(_)));
        assert_eq!(
            store.pending_fleet_messages_for(&target).unwrap().len(),
            FLEET_MESSAGE_PENDING_CAP as usize
        );

        // A different sender to the same target is a separate cap bucket.
        store
            .enqueue_fleet_message(
                "other-sender",
                "cli",
                None,
                "someone-else",
                &target,
                "msg",
                "follow_up",
            )
            .unwrap();
        assert_eq!(
            store.pending_fleet_messages_for(&target).unwrap().len(),
            FLEET_MESSAGE_PENDING_CAP as usize + 1
        );

        // Delivering one frees a cap slot for the same sender.
        assert!(store.mark_fleet_message_delivered("m0", 1000).unwrap());
        store
            .enqueue_fleet_message("m-again", "cli", None, "cli", &target, "msg", "follow_up")
            .unwrap();
    }
}
