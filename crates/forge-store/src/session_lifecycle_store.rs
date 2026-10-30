//! Star, done and auto-archive state for sessions.

use super::*;

impl Store {
    /// Star or unstar a session. Starred sessions float to the top of every list and are never
    /// auto-archived.
    pub fn set_session_starred(&self, session_id: &str, starred: bool) -> Result<bool> {
        let n = self.lock()?.execute(
            "UPDATE session SET starred = ?2 WHERE id = ?1",
            rusqlite::params![session_id, i64::from(starred)],
        )?;
        Ok(n > 0)
    }

    /// Mark a session done (`Some(unix seconds)`) or not done (`None`).
    pub fn set_session_done_at(&self, session_id: &str, done_at: Option<i64>) -> Result<bool> {
        let n = self.lock()?.execute(
            "UPDATE session SET done_at = ?2 WHERE id = ?1",
            rusqlite::params![session_id, done_at],
        )?;
        Ok(n > 0)
    }

    /// `(starred, done_at)` for each known id. Unknown ids are absent from the map.
    pub fn session_lifecycle(
        &self,
        ids: &[String],
    ) -> Result<std::collections::HashMap<String, (bool, Option<i64>)>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self.lock()?;
        let marks = vec!["?"; ids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT id, starred, done_at FROM session WHERE id IN ({marks})"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, i64>(1)? != 0, r.get::<_, Option<i64>>(2)?),
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(StoreError::from)
    }

    /// Top-level sessions the daemon is not hosting that nothing has touched since `cutoff`
    /// (unix seconds), are not starred and are not already archived: what the auto-archive sweep
    /// archives. Sessions that own a worktree are returned with it so the caller can leave them.
    pub fn sessions_idle_since(&self, cutoff: i64) -> Result<Vec<(String, Option<String>)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.worktree_path,
                    COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                             s.created_at) AS last_activity
             FROM session s
             WHERE s.parent_session_id IS NULL AND s.archived = 0 AND s.starred = 0
               AND s.daemon_live = 0 AND s.agent_active = 0
               AND EXISTS (SELECT 1 FROM message m WHERE m.session_id = s.id AND m.role = 'user')
             AND last_activity < ?1",
        )?;
        let rows = stmt.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;
    use forge_types::Role;

    fn used(store: &Store) -> String {
        let id = store.create_session("/w", "default").unwrap();
        store.add_message(&id, 0, Role::User, "hi", None).unwrap();
        id
    }

    #[test]
    fn star_and_done_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let id = used(&store);
        assert_eq!(
            store.session_lifecycle(std::slice::from_ref(&id)).unwrap()[&id],
            (false, None)
        );
        assert!(store.set_session_starred(&id, true).unwrap());
        assert!(store.set_session_done_at(&id, Some(42)).unwrap());
        assert_eq!(
            store.session_lifecycle(std::slice::from_ref(&id)).unwrap()[&id],
            (true, Some(42))
        );
        store.set_session_done_at(&id, None).unwrap();
        assert_eq!(
            store.session_lifecycle(std::slice::from_ref(&id)).unwrap()[&id].1,
            None
        );
        assert!(!store.set_session_starred("missing", true).unwrap());
    }

    #[test]
    fn idle_sweep_skips_starred_live_and_archived_sessions() {
        let store = Store::open_in_memory().unwrap();
        let stale = used(&store);
        let starred = used(&store);
        let live = used(&store);
        let archived = used(&store);
        store.set_session_starred(&starred, true).unwrap();
        store.set_session_daemon_live(&live, true).unwrap();
        store.archive_session(&archived).unwrap();
        let future = i64::MAX / 2;
        let ids: Vec<String> = store
            .sessions_idle_since(future)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec![stale]);
        assert!(store.sessions_idle_since(0).unwrap().is_empty());
    }
}
