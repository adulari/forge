//! What a compaction summary is built from: the session's stored tool results, which hold each
//! result as the model first saw it, before the in-memory prune pass cut it to a head.

use super::*;

/// The stored side of a compaction input, for ACTIVE rows only, so a row `/undo` rewound or an
/// earlier compaction folded never reaches a later summary.
#[derive(Debug, Clone, Default)]
pub struct CompactionSource {
    /// `(tool_call_id, full content)` of every active tool result, oldest first.
    pub tool_results: Vec<(String, String)>,
}

impl Store {
    pub fn compaction_source(&self, session_id: &str) -> Result<CompactionSource> {
        let conn = self.lock()?;
        let mut tools = conn.prepare_cached(
            "SELECT tool_call_id, content FROM message
             WHERE session_id = ?1 AND active = 1 AND role = 'tool' AND tool_call_id IS NOT NULL
             ORDER BY seq",
        )?;
        let tool_results = tools
            .query_map([session_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<(String, String)>, _>>()?;
        Ok(CompactionSource { tool_results })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_holds_the_full_tool_results_of_active_rows_only() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "fix the bug", None)
            .unwrap();
        let big = "x".repeat(40_000);
        store
            .add_message_full(&sid, 1, Role::Tool, &big, None, &[], Some("c1"))
            .unwrap();
        store
            .add_message_full(&sid, 3, Role::Tool, "old", None, &[], Some("c2"))
            .unwrap();
        store.deactivate_messages_from(&sid, 3).unwrap();

        let source = store.compaction_source(&sid).unwrap();
        assert_eq!(source.tool_results.len(), 1);
        assert_eq!(source.tool_results[0].0, "c1");
        assert_eq!(source.tool_results[0].1.len(), 40_000);
    }
}
