//! The approval history behind "you've allowed this command shape N times here"
//! (docs/features/approvals-learn.md). One row per answered permission prompt, keyed by the
//! project and the normalized shape the rule engine would be given.

use crate::{project_key, Result, Store};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Store {
    /// Remember one answered prompt. `shape` is empty for a call that has none to learn from.
    pub fn record_permission_decision(
        &self,
        project: &str,
        tool: &str,
        shape: &str,
        decision: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO permission_decision (project, tool, shape, decision, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (project_key(project), tool, shape, decision, now_ms()),
        )?;
        Ok(())
    }

    /// Allows of `shape` in `project` since the last deny or declined offer for it: a deny wipes
    /// the streak, and so does turning an offer down.
    pub fn permission_allow_streak(&self, project: &str, tool: &str, shape: &str) -> Result<u32> {
        let conn = self.lock()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM permission_decision
             WHERE project = ?1 AND tool = ?2 AND shape = ?3
               AND decision IN ('allow', 'session', 'always')
               AND id > COALESCE((SELECT MAX(id) FROM permission_decision
                                  WHERE project = ?1 AND tool = ?2 AND shape = ?3
                                    AND decision IN ('deny', 'suggestion_declined')), 0)",
            (project_key(project), tool, shape),
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn allows_count_and_a_deny_resets_the_streak() {
        let s = store();
        let p = "/work/app";
        for _ in 0..3 {
            s.record_permission_decision(p, "shell", "cargo test *", "allow")
                .unwrap();
        }
        assert_eq!(
            s.permission_allow_streak(p, "shell", "cargo test *")
                .unwrap(),
            3
        );
        s.record_permission_decision(p, "shell", "cargo test *", "deny")
            .unwrap();
        assert_eq!(
            s.permission_allow_streak(p, "shell", "cargo test *")
                .unwrap(),
            0
        );
        s.record_permission_decision(p, "shell", "cargo test *", "session")
            .unwrap();
        assert_eq!(
            s.permission_allow_streak(p, "shell", "cargo test *")
                .unwrap(),
            1
        );
    }

    #[test]
    fn streaks_are_per_project_and_per_shape_and_worktrees_share_their_checkout() {
        let s = store();
        s.record_permission_decision("/work/app", "shell", "cargo test *", "allow")
            .unwrap();
        s.record_permission_decision(
            "/work/app/.forge/worktrees/x",
            "shell",
            "cargo test *",
            "allow",
        )
        .unwrap();
        s.record_permission_decision("/work/app", "shell", "cargo build *", "allow")
            .unwrap();
        assert_eq!(
            s.permission_allow_streak("/work/app", "shell", "cargo test *")
                .unwrap(),
            2
        );
        assert_eq!(
            s.permission_allow_streak("/work/other", "shell", "cargo test *")
                .unwrap(),
            0
        );
    }

    #[test]
    fn a_declined_offer_resets_the_streak() {
        let s = store();
        let p = "/work/app";
        for _ in 0..3 {
            s.record_permission_decision(p, "shell", "npm test *", "allow")
                .unwrap();
        }
        s.record_permission_decision(p, "shell", "npm test *", "suggestion_declined")
            .unwrap();
        assert_eq!(
            s.permission_allow_streak(p, "shell", "npm test *").unwrap(),
            0
        );
    }
}
