//! Durable per-session prompt queue, so prompts lined up behind a running turn survive a daemon
//! restart (docs/features/session-queue.md).
//!
//! The payload is the driver's visible queue in order: plain prompts and queued `/commands`. The
//! store only keeps and returns it; ordering and meaning belong to the driver.

use rusqlite::OptionalExtension;

use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError};

/// One persisted queue entry. `had_images` records that pasted images were queued with the text:
/// the pixels live in memory only, so a restore can say they are gone rather than run without them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedPrompt {
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub had_images: bool,
}

impl QueuedPrompt {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            had_images: false,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredEntry {
    Plain(String),
    Entry(QueuedPrompt),
}

impl Store {
    /// Replace the session's persisted queue. An empty queue clears the column.
    pub fn set_session_queued_prompts(
        &self,
        session_id: &str,
        queue: &[QueuedPrompt],
    ) -> Result<()> {
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
    pub fn session_queued_prompts(&self, session_id: &str) -> Result<Vec<QueuedPrompt>> {
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
            .and_then(|json| serde_json::from_str::<Vec<StoredEntry>>(&json).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| match entry {
                StoredEntry::Plain(text) => QueuedPrompt::text(text),
                StoredEntry::Entry(entry) => entry,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::QueuedPrompt;
    use crate::Store;

    #[test]
    fn the_queue_round_trips_in_order_and_clears() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "Default").unwrap();
        assert!(store.session_queued_prompts(&id).unwrap().is_empty());
        let queue = vec![
            QueuedPrompt::text("first"),
            QueuedPrompt::text("/compact"),
            QueuedPrompt {
                text: "third".into(),
                had_images: true,
            },
        ];
        store.set_session_queued_prompts(&id, &queue).unwrap();
        assert_eq!(store.session_queued_prompts(&id).unwrap(), queue);
        store.set_session_queued_prompts(&id, &[]).unwrap();
        assert!(store.session_queued_prompts(&id).unwrap().is_empty());
    }

    #[test]
    fn rows_written_as_plain_strings_still_read() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "Default").unwrap();
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE session SET queued_prompts = ?2 WHERE id = ?1",
                rusqlite::params![id, r#"["old","/compact"]"#],
            )
            .unwrap();
        assert_eq!(
            store.session_queued_prompts(&id).unwrap(),
            vec![QueuedPrompt::text("old"), QueuedPrompt::text("/compact")]
        );
    }

    #[test]
    fn an_unknown_session_has_an_empty_queue() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.session_queued_prompts("nope").unwrap().is_empty());
    }
}
