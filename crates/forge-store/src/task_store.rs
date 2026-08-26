//! Durable per-session task list persistence.

use super::*;

impl Store {
    /// Replace a session's task list (the `update_tasks` tool). Stored as one JSON row so a
    /// resumed session restores its tasks. An empty list clears it.
    pub fn set_tasks(&self, session_id: &str, tasks: &[forge_types::TodoItem]) -> Result<()> {
        let json = serde_json::to_string(tasks)
            .map_err(|error| StoreError::Json(format!("session tasks: {error}")))?;
        self.lock()?.execute(
            "INSERT INTO session_tasks (session_id, tasks_json, updated_at)
             VALUES (?1, ?2, strftime('%s','now'))
             ON CONFLICT(session_id) DO UPDATE SET
               tasks_json = excluded.tasks_json, updated_at = excluded.updated_at",
            (session_id, json),
        )?;
        Ok(())
    }

    /// The session's persisted task list (empty if none). Malformed JSON is returned as a
    /// diagnostic so resume callers do not mistake corrupt history for an intentional empty list.
    pub fn tasks(&self, session_id: &str) -> Result<Vec<forge_types::TodoItem>> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row(
                "SELECT tasks_json FROM session_tasks WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map_or_else(
            || Ok(Vec::new()),
            |json| {
                serde_json::from_str(&json)
                    .map_err(|error| StoreError::Json(format!("session tasks: {error}")))
            },
        )
    }
}
