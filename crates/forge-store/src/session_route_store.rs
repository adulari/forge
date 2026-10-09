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
}
