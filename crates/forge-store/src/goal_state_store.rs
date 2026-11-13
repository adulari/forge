//! Durable `/goal` record for a session whose goal carries a spend envelope, so a pause at the
//! cap survives a daemon restart. The store only keeps the opaque JSON; its meaning belongs to
//! the run driver.

use rusqlite::OptionalExtension;

use crate::{Result, Store};

impl Store {
    /// Replace the session's persisted goal record; `None` clears it.
    pub fn set_session_goal_state(&self, session_id: &str, json: Option<&str>) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET goal_state = ?2 WHERE id = ?1",
            rusqlite::params![session_id, json],
        )?;
        Ok(())
    }

    /// The most recently created session that still has a goal record, with its JSON: what
    /// `forge goal show` prints when no session is named.
    pub fn latest_goal_state(&self) -> Result<Option<(String, String)>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT id, goal_state FROM session WHERE goal_state IS NOT NULL \
                 ORDER BY rowid DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?)
    }

    pub fn session_goal_state(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT goal_state FROM session WHERE id = ?1",
                [session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn the_goal_record_round_trips_and_clears() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "Default").unwrap();
        assert_eq!(store.session_goal_state(&id).unwrap(), None);
        store
            .set_session_goal_state(&id, Some("{\"a\":1}"))
            .unwrap();
        assert_eq!(
            store.session_goal_state(&id).unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        store.set_session_goal_state(&id, None).unwrap();
        assert_eq!(store.session_goal_state(&id).unwrap(), None);
    }

    #[test]
    fn the_latest_goal_state_is_the_newest_session_that_has_one() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.latest_goal_state().unwrap(), None);
        let old = store.create_session("/tmp", "Default").unwrap();
        let new = store.create_session("/tmp", "Default").unwrap();
        let none = store.create_session("/tmp", "Default").unwrap();
        store
            .set_session_goal_state(&old, Some("{\"n\":1}"))
            .unwrap();
        store
            .set_session_goal_state(&new, Some("{\"n\":2}"))
            .unwrap();
        assert_eq!(
            store.latest_goal_state().unwrap(),
            Some((new.clone(), "{\"n\":2}".to_string()))
        );
        let _ = none;
    }
}
