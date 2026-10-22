//! `forge run --no-session-persistence`: forget a run's conversation but not what it cost.
//!
//! Usage rows hang off messages (`ON DELETE CASCADE`), so deleting a session also deleted its
//! spend: the day/week/month caps and `forge stats` would never see an unsaved run. The rows are
//! moved to one archived ledger session first, then the session and everything it owns is deleted
//! as usual. The ledger has no transcript, never syncs (it is never journaled) and is hidden from
//! session lists and `--continue`.

use super::*;

/// The archived session that holds the usage of runs whose transcript was not kept.
pub const UNSAVED_RUNS_LEDGER: &str = "unsaved-runs-ledger";
const LEDGER_MESSAGE: &str = "unsaved-runs-ledger-message";

impl Store {
    /// Delete `session_id` (and its subagent tree) after moving its usage rows and cost onto the
    /// unsaved-runs ledger. Returns whether a session was deleted.
    pub fn forget_session_keeping_spend(&self, session_id: &str) -> Result<bool> {
        if !self.session_exists(session_id)? {
            return Ok(false);
        }
        {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO session (id, title, cwd, permission_mode, total_cost_usd, archived) \
                 VALUES (?1, 'unsaved runs (--no-session-persistence)', '', 'Default', 0, 1) \
                 ON CONFLICT(id) DO NOTHING",
                [UNSAVED_RUNS_LEDGER],
            )?;
            tx.execute(
                "INSERT INTO message (id, session_id, seq, role, content, visibility, active) \
                 VALUES (?1, ?2, 0, 'system', '', 'ui', 0) ON CONFLICT(id) DO NOTHING",
                (LEDGER_MESSAGE, UNSAVED_RUNS_LEDGER),
            )?;
            let tree = "WITH RECURSIVE tree(id) AS ( \
                            SELECT id FROM session WHERE id = ?1 \
                            UNION ALL \
                            SELECT child.id FROM session child \
                            JOIN tree parent ON child.parent_session_id = parent.id) ";
            tx.execute(
                &format!(
                    "{tree} UPDATE session SET total_cost_usd = total_cost_usd + \
                     (SELECT COALESCE(SUM(total_cost_usd), 0) FROM session \
                      WHERE id IN (SELECT id FROM tree)) WHERE id = ?2"
                ),
                (session_id, UNSAVED_RUNS_LEDGER),
            )?;
            tx.execute(
                &format!(
                    "{tree} UPDATE usage SET message_id = ?2 WHERE message_id IN \
                     (SELECT m.id FROM message m WHERE m.session_id IN (SELECT id FROM tree))"
                ),
                (session_id, LEDGER_MESSAGE),
            )?;
            tx.commit()?;
        }
        self.delete_session(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forgotten_session_is_gone_but_its_spend_still_counts() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "Default").unwrap();
        let mid = store
            .add_message(&sid, 0, Role::User, "secret prompt", None)
            .unwrap();
        store
            .record_usage(
                &sid,
                &mid,
                &Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cost_usd: 0.25,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        let before = store.spend_between(0, i64::MAX).unwrap();

        assert!(store.forget_session_keeping_spend(&sid).unwrap());

        assert!(!store.session_exists(&sid).unwrap(), "cannot be resumed");
        assert!((store.spend_between(0, i64::MAX).unwrap() - before).abs() < 1e-9);
        assert!((store.session_cost(UNSAVED_RUNS_LEDGER).unwrap() - 0.25).abs() < 1e-9);
        assert!(
            store
                .list_sessions()
                .unwrap()
                .iter()
                .all(|s| s.id != UNSAVED_RUNS_LEDGER),
            "the ledger stays out of session lists"
        );
        assert_eq!(store.most_recent_session_id().unwrap(), None);

        let second = store.create_session("/tmp", "Default").unwrap();
        assert!(
            store.forget_session_keeping_spend(&second).unwrap(),
            "ledger is reused"
        );
        assert!(!store.forget_session_keeping_spend("missing").unwrap());
    }
}
