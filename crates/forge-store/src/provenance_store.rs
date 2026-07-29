//! Source-edit provenance and turn-context queries for forge blame.

use super::*;

impl Store {
    /// Every recorded `write_file`/`edit_file` call whose `path` matches (as a suffix) the given
    /// `filename_suffix`, oldest first — the raw material `forge blame` (docs/features/forge-blame.md)
    /// attributes source lines from. Joined to the owning session (for `cwd`, to resolve a relative
    /// `path` the same way the tool did) and to the assistant message that made the call (for
    /// `model`); `routing_decision.chosen_model` fills in when the message's own `model` is NULL
    /// (older rows, or a message that predates routing being recorded for it).
    pub fn file_edits(&self, filename_suffix: &str) -> Result<Vec<FileEditRow>> {
        let conn = self.lock()?;
        let pattern = escape_like_pattern(filename_suffix);
        let mut stmt = conn.prepare(
            "SELECT tc.tool_name, tc.args_json, tc.path, m.session_id, s.cwd,
                    COALESCE(m.model, r.chosen_model), m.seq, tc.created_at
             FROM tool_call tc
             JOIN message m ON m.id = tc.message_id
             JOIN session s ON s.id = m.session_id
             LEFT JOIN routing_decision r ON r.message_id = m.id
             WHERE tc.path IS NOT NULL
               AND tc.tool_name IN ('write_file', 'edit_file')
               AND tc.status = 'ok'
               AND tc.path LIKE '%' || ?1 ESCAPE '\\'
             ORDER BY tc.created_at ASC",
        )?;
        let rows = stmt.query_map([pattern], |row| {
            Ok(FileEditRow {
                tool_name: row.get(0)?,
                args_json: row.get(1)?,
                path: row.get(2)?,
                session_id: row.get(3)?,
                session_cwd: row.get(4)?,
                model: row.get(5)?,
                seq: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The provenance context of one turn: the nearest user prompt at or before `seq`, and the
    /// content of the assistant message AT `seq` (the one that made the edit `forge blame` is
    /// explaining). Either half is `None` if no matching row exists — e.g. `seq` is a virtual
    /// subagent turn with no direct user prompt in this session.
    pub fn turn_context(&self, session_id: &str, seq: i64) -> Result<TurnContext> {
        let conn = self.lock()?;
        let user_prompt = conn
            .query_row(
                "SELECT content FROM message WHERE session_id = ?1 AND role = 'user' AND seq <= ?2 \
                 ORDER BY seq DESC LIMIT 1",
                (session_id, seq),
                |r| r.get(0),
            )
            .optional()?;
        let assistant_content = conn
            .query_row(
                "SELECT content FROM message WHERE session_id = ?1 AND role = 'assistant' AND seq = ?2",
                (session_id, seq),
                |r| r.get(0),
            )
            .optional()?;
        Ok(TurnContext {
            user_prompt,
            assistant_content,
        })
    }
}
