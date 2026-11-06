//! The approval history behind "you've allowed this command shape N times here"
//! (docs/features/approvals-learn.md). One row per answered permission prompt, keyed by the
//! project and the normalized shape the rule engine would be given.

use crate::{project_key, Result, Store};

/// How a prompt was answered, as stored. `suggestion_*` rows record what the person did with an
/// offered rule, so a declined offer is not repeated until the shape has earned it again.
pub const DECISIONS_THAT_ALLOW: [&str; 3] = ["allow", "session", "always"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecisionRow {
    pub id: i64,
    pub project: String,
    pub tool: String,
    pub shape: String,
    pub decision: String,
    pub created_at: i64,
}

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

    /// Every row for `project` that is a learned-rule event (accepted or declined offers), newest
    /// first: the settings list.
    pub fn permission_learned(&self, project: Option<&str>) -> Result<Vec<PermissionDecisionRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, project, tool, shape, decision, created_at FROM permission_decision
             WHERE decision IN ('suggestion_accepted', 'suggestion_declined')
               AND (?1 IS NULL OR project = ?1)
             ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([project.map(project_key)], |r| {
            Ok(PermissionDecisionRow {
                id: r.get(0)?,
                project: r.get(1)?,
                tool: r.get(2)?,
                shape: r.get(3)?,
                decision: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Forget one learned-rule event. `true` when a row was removed.
    pub fn delete_permission_learned(&self, id: i64) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM permission_decision
             WHERE id = ?1 AND decision IN ('suggestion_accepted', 'suggestion_declined')",
            [id],
        )?;
        Ok(n > 0)
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
    fn a_declined_offer_resets_and_learned_rows_list_and_delete() {
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
        s.record_permission_decision(p, "shell", "cargo test *", "suggestion_accepted")
            .unwrap();
        let rows = s.permission_learned(Some(p)).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].shape, "cargo test *");
        assert!(s.delete_permission_learned(rows[0].id).unwrap());
        assert_eq!(s.permission_learned(None).unwrap().len(), 1);
    }
}
