//! What each session did since a point in time, for the "while you were away" digest.

use super::*;

/// Longest message excerpt read per session; the caller truncates further for display.
const EXCERPT_BYTES: i64 = 600;

#[derive(Debug, Clone, PartialEq)]
pub struct BriefSessionRow {
    pub id: String,
    pub title: Option<String>,
    pub preview: Option<String>,
    pub cwd: String,
    pub worktree_path: Option<String>,
    pub last_activity: i64,
    pub done_at: Option<i64>,
    /// Cost of provider calls recorded at or after the cutoff.
    pub spend_usd: f64,
    /// Role and text of the newest visible message (a failure note is a `system` row).
    pub last_role: Option<String>,
    pub last_content: Option<String>,
    /// Text of the newest assistant reply that said something.
    pub last_assistant: Option<String>,
}

impl Store {
    /// Top-level sessions with a message at or after `since`, newest activity first, at most
    /// `limit`. Per-session lookups ride `idx_message_session_created` and `idx_message_session`,
    /// and the spend join rides `idx_usage_created_at`, so the cost scales with the cap and the
    /// window rather than with the whole history.
    pub fn brief_sessions_since(&self, since: i64, limit: usize) -> Result<Vec<BriefSessionRow>> {
        let conn = self.lock()?;
        let mut spend: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare_cached(
                "SELECT m.session_id, COALESCE(SUM(u.cost_usd), 0.0)
                 FROM usage u JOIN message m ON m.id = u.message_id
                 WHERE u.created_at >= ?1
                 GROUP BY m.session_id",
            )?;
            let rows = stmt.query_map([since], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })?;
            for row in rows {
                let (id, cost) = row?;
                spend.insert(id, cost);
            }
        }
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.title, s.cwd, s.worktree_path, s.done_at,
                    (SELECT content FROM message m WHERE m.session_id = s.id
                       AND m.role = 'user' AND m.active = 1 AND m.nudge = 0
                       ORDER BY m.seq LIMIT 1),
                    a.last_activity
             FROM (SELECT s2.id AS id,
                          COALESCE((SELECT MAX(m.created_at) FROM message m
                                    WHERE m.session_id = s2.id), s2.created_at) AS last_activity
                   FROM session s2 WHERE s2.parent_session_id IS NULL) a
             JOIN session s ON s.id = a.id
             WHERE a.last_activity >= ?1
             ORDER BY a.last_activity DESC
             LIMIT ?2",
        )?;
        let mut rows = stmt
            .query_map(rusqlite::params![since, limit as i64], |r| {
                Ok(BriefSessionRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    cwd: r.get(2)?,
                    worktree_path: r.get(3)?,
                    done_at: r.get(4)?,
                    preview: r
                        .get::<_, Option<String>>(5)?
                        .map(|p| p.chars().take(200).collect()),
                    last_activity: r.get(6)?,
                    spend_usd: 0.0,
                    last_role: None,
                    last_content: None,
                    last_assistant: None,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut last = conn.prepare_cached(
            "SELECT role, substr(content, 1, ?2) FROM message
             WHERE session_id = ?1 AND active = 1 AND visibility != 'llm_only'
             ORDER BY seq DESC LIMIT 1",
        )?;
        let mut reply = conn.prepare_cached(
            "SELECT substr(content, 1, ?2) FROM message
             WHERE session_id = ?1 AND active = 1 AND role = 'assistant'
               AND visibility = 'llm' AND TRIM(content) != ''
             ORDER BY seq DESC LIMIT 1",
        )?;
        for row in &mut rows {
            row.spend_usd = spend.get(&row.id).copied().unwrap_or(0.0);
            if let Some((role, content)) = last
                .query_row(rusqlite::params![row.id, EXCERPT_BYTES], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .optional()?
            {
                row.last_role = Some(role);
                row.last_content = Some(content);
            }
            row.last_assistant = reply
                .query_row(rusqlite::params![row.id, EXCERPT_BYTES], |r| r.get(0))
                .optional()?;
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_sessions_active_since_with_their_tail_and_spend() {
        let store = Store::open_in_memory().unwrap();
        let old = store.create_session("/old", "default").unwrap();
        store
            .add_message(&old, 0, Role::User, "ancient", None)
            .unwrap();
        let fresh = store.create_session("/fresh", "default").unwrap();
        store
            .add_message(&fresh, 0, Role::User, "do the thing", None)
            .unwrap();
        store
            .add_message(
                &fresh,
                1,
                Role::Assistant,
                "Done. Added the flag.\nMore detail.",
                None,
            )
            .unwrap();
        store.note_turn_failure(&fresh, "provider 503").unwrap();
        let conn = store.lock().unwrap();
        conn.execute(
            "UPDATE message SET created_at = 100 WHERE session_id = ?1",
            [&old],
        )
        .unwrap();
        conn.execute(
            "UPDATE message SET created_at = 5000 WHERE session_id = ?1",
            [&fresh],
        )
        .unwrap();
        drop(conn);

        let rows = store.brief_sessions_since(1000, 10).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, fresh);
        assert_eq!(row.preview.as_deref(), Some("do the thing"));
        assert_eq!(row.last_role.as_deref(), Some("system"));
        assert_eq!(
            row.last_content.as_deref(),
            Some("⚠ Turn failed: provider 503")
        );
        assert_eq!(
            row.last_assistant.as_deref(),
            Some("Done. Added the flag.\nMore detail.")
        );
        assert_eq!(row.last_activity, 5000);
    }

    #[test]
    fn the_cap_keeps_the_newest_sessions() {
        let store = Store::open_in_memory().unwrap();
        for n in 0..4 {
            let id = store.create_session("/p", "default").unwrap();
            store.add_message(&id, 0, Role::User, "go", None).unwrap();
            store
                .lock()
                .unwrap()
                .execute(
                    "UPDATE message SET created_at = ?2 WHERE session_id = ?1",
                    rusqlite::params![id, 2000 + n],
                )
                .unwrap();
        }
        let rows = store.brief_sessions_since(1000, 2).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.last_activity).collect::<Vec<_>>(),
            vec![2003, 2002]
        );
    }
}
