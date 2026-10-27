//! Read-only lookups behind the apps' nested view of a subagent child session.

use super::*;

/// What the apps need to render a child session as a nested, read-only session.
#[derive(Debug, Clone, PartialEq)]
pub struct ChildSessionInfo {
    pub parent_id: String,
    pub parent_title: Option<String>,
    pub title: Option<String>,
    pub cwd: String,
    pub cost_usd: f64,
    pub model: Option<String>,
    pub message_count: i64,
}

impl Store {
    /// Whether `child_id` is a subagent child session spawned by `parent_id`.
    pub fn is_child_of(&self, child_id: &str, parent_id: &str) -> Result<bool> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM session WHERE id = ?1 AND parent_session_id = ?2",
                (child_id, parent_id),
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The child's metadata, or `None` when `child_id` is missing or is not a child session.
    pub fn child_session_info(&self, child_id: &str) -> Result<Option<ChildSessionInfo>> {
        let conn = self.lock()?;
        let head = conn
            .query_row(
                "SELECT c.parent_session_id, p.title, c.title, c.cwd, c.total_cost_usd \
                 FROM session c LEFT JOIN session p ON p.id = c.parent_session_id \
                 WHERE c.id = ?1 AND c.parent_session_id IS NOT NULL",
                [child_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, f64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((parent_id, parent_title, title, cwd, cost_usd)) = head else {
            return Ok(None);
        };
        let model = conn
            .query_row(
                "SELECT model FROM message WHERE session_id = ?1 AND active = 1 \
                 AND model IS NOT NULL AND model != '' ORDER BY seq DESC LIMIT 1",
                [child_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        let message_count = conn.query_row(
            "SELECT COUNT(*) FROM message WHERE session_id = ?1 AND active = 1",
            [child_id],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(Some(ChildSessionInfo {
            parent_id,
            parent_title,
            title,
            cwd,
            cost_usd,
            model,
            message_count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_info_resolves_only_for_child_sessions() {
        let store = Store::open_in_memory().unwrap();
        let parent = store.create_session("/work", "Default").unwrap();
        store.set_session_title(&parent, "fix the bug").unwrap();
        let child = store
            .create_child_session("/work", "Default", &parent)
            .unwrap();
        store.set_session_title(&child, "explore").unwrap();
        store
            .add_message(&child, 0, Role::User, "look around", None)
            .unwrap();
        store
            .add_message(&child, 1, Role::Assistant, "done", Some("m-1"))
            .unwrap();

        let info = store.child_session_info(&child).unwrap().unwrap();
        assert_eq!(info.parent_id, parent);
        assert_eq!(info.parent_title.as_deref(), Some("fix the bug"));
        assert_eq!(info.title.as_deref(), Some("explore"));
        assert_eq!(info.model.as_deref(), Some("m-1"));
        assert_eq!(info.message_count, 2);
        assert!(store.child_session_info(&parent).unwrap().is_none());
        assert!(store.child_session_info("missing").unwrap().is_none());
        assert!(store.is_child_of(&child, &parent).unwrap());
        assert!(!store.is_child_of(&parent, &child).unwrap());
    }
}
