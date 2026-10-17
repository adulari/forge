//! Durable per-session prompt queue, so prompts lined up behind a running turn survive a daemon
//! restart (docs/features/session-queue.md).
//!
//! The payload is the driver's visible queue in order: plain prompts and queued `/commands`. The
//! store only keeps and returns it; ordering and meaning belong to the driver.

use rusqlite::OptionalExtension;

use crate::{Result, Store, StoreError};

impl Store {
    /// Replace the session's persisted queue. An empty queue clears the column.
    pub fn set_session_queued_prompts(&self, session_id: &str, queue: &[String]) -> Result<()> {
        let json = if queue.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(queue)
                    .map_err(|e| StoreError::InvalidValue(format!("queued prompts: {e}")))?,
            )
        };
        self.lock()?.execute(
            "UPDATE session SET queued_prompts = ?2 WHERE id = ?1",
            rusqlite::params![session_id, json],
        )?;
        Ok(())
    }

    /// The persisted queue, oldest first; empty when none was saved or the value is unreadable.
    pub fn session_queued_prompts(&self, session_id: &str) -> Result<Vec<String>> {
        let raw = self
            .lock()?
            .query_row(
                "SELECT queued_prompts FROM session WHERE id = ?1",
                [session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(raw
            .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn the_queue_round_trips_in_order_and_clears() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "Default").unwrap();
        assert!(store.session_queued_prompts(&id).unwrap().is_empty());
        let queue = vec![
            "first".to_string(),
            "/compact".to_string(),
            "third".to_string(),
        ];
        store.set_session_queued_prompts(&id, &queue).unwrap();
        assert_eq!(store.session_queued_prompts(&id).unwrap(), queue);
        store.set_session_queued_prompts(&id, &[]).unwrap();
        assert!(store.session_queued_prompts(&id).unwrap().is_empty());
    }

    #[test]
    fn an_unknown_session_has_an_empty_queue() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.session_queued_prompts("nope").unwrap().is_empty());
    }
}
