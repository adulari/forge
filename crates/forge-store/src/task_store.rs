//! Durable per-session task list persistence.

use super::*;

impl Store {
    /// Replace a session's task list (the `update_tasks` tool). Stored as one JSON row so a
    /// resumed session restores its tasks. An empty list clears it.
    pub fn set_tasks(&self, session_id: &str, tasks: &[forge_types::TodoItem]) -> Result<()> {
        let json = serde_json::to_string(tasks).unwrap_or_else(|_| "[]".to_string());
        self.lock()?.execute(
            "INSERT INTO session_tasks (session_id, tasks_json, updated_at)
             VALUES (?1, ?2, strftime('%s','now'))
             ON CONFLICT(session_id) DO UPDATE SET
               tasks_json = excluded.tasks_json, updated_at = excluded.updated_at",
            (session_id, json),
        )?;
        Ok(())
    }

    /// The session's persisted task list (empty if none/unparseable).
    pub fn tasks(&self, session_id: &str) -> Result<Vec<forge_types::TodoItem>> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row(
                "SELECT tasks_json FROM session_tasks WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default())
    }
}
