//! Resume-time routing lookups.

use super::*;

impl Store {
    /// The model the Mesh most recently routed this session to. Resume only needs this one row;
    /// [`session_models`](Self::session_models) joined and sorted every routing decision of the
    /// session first (180 ms on a 38k-message session, before the first frame).
    pub fn session_last_model(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT r.chosen_model FROM routing_decision r \
                 JOIN message m ON m.id = r.message_id \
                 WHERE m.session_id = ?1 ORDER BY m.seq DESC LIMIT 1",
                [session_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Whether the newest model-visible message of a session is a final assistant answer. A tool
    /// call or user message at the tail means the last turn did not finish with an answer
    /// (interrupted, crashed, or still pending), which callers report as unknown rather than guess.
    pub fn session_last_is_answer(&self, session_id: &str) -> Result<bool> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT role = 'assistant' AND tool_calls_json IS NULL AND TRIM(content) != '' \
                 FROM message WHERE session_id = ?1 AND active = 1 AND visibility = 'llm' \
                 ORDER BY seq DESC LIMIT 1",
                [session_id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::Role;

    #[test]
    fn last_is_answer_only_when_the_tail_is_a_final_assistant_message() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        assert!(
            !store.session_last_is_answer(&sid).unwrap(),
            "empty session"
        );
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        assert!(
            !store.session_last_is_answer(&sid).unwrap(),
            "pending prompt"
        );
        store
            .add_message(&sid, 1, Role::Assistant, "done", Some("ollama::m"))
            .unwrap();
        assert!(store.session_last_is_answer(&sid).unwrap());
        store
            .add_message(&sid, 2, Role::User, "more", None)
            .unwrap();
        assert!(
            !store.session_last_is_answer(&sid).unwrap(),
            "a new prompt reopens it"
        );
    }
}
