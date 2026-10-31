//! The slice of a transcript that may vouch for a file path (workspace file route).

use super::*;
use crate::transcript_read_store::{parse_role_with_diagnostic, parse_tool_calls_with_diagnostic};

impl Store {
    /// The newest `limit` messages that can name a file the session deliberately referenced: what
    /// the user typed, and the arguments of the assistant's tool calls. Tool output, assistant
    /// prose, harness notes and anything a fetched page said are left out, and a user body is cut
    /// to 64 KiB, so a long session costs a bounded read. Oldest first.
    pub fn load_path_evidence_tail(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT role, CASE WHEN role = 'user' THEN substr(content, 1, 65536) ELSE '' END,
                    tool_calls_json
             FROM message
             WHERE session_id = ?1 AND (active = 1 OR compacted = 1) AND COALESCE(nudge, 0) = 0
               AND (role = 'user'
                    OR (role = 'assistant' AND tool_calls_json IS NOT NULL
                        AND tool_calls_json NOT IN ('', '[]')))
             ORDER BY seq DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map((session_id, limit as i64), |row| {
            let role: String = row.get(0)?;
            let tool_calls_json: Option<String> = row.get(2)?;
            Ok(StoredMessage {
                role: parse_role_with_diagnostic(session_id, None, &role),
                content: row.get(1)?,
                model: None,
                tool_calls: parse_tool_calls_with_diagnostic(
                    session_id,
                    None,
                    tool_calls_json.as_deref(),
                ),
                tool_call_id: None,
                visibility: Visibility::Llm,
                provider_items: None,
                kind: forge_types::MessageKind::Normal,
            })
        })?;
        let mut messages = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        messages.reverse();
        Ok(messages)
    }
}
