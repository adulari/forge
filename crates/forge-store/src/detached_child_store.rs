//! Retained async subagents: the `detached_child` registry (docs/rfcs/retained-async-subagents.md).
//!
//! A blocking `spawn_agents` child lives and dies inside one `orchestrate()` call — nothing about
//! it needs to survive past the parent's turn. A DETACHED child is admitted immediately and then
//! runs to completion on its own, independent of the parent turn (and, across a daemon restart,
//! independent of the process that admitted it). This module is the durable half of that: one row
//! per detached child, so `list_subagents`/`cancel_subagent` and turn-boundary delivery all read
//! from the store instead of from in-memory state that a restart would erase.

use super::*;

/// A detached child's lifecycle state, persisted as the `detached_child.status` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetachedChildStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl DetachedChildStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            // Any unrecognized value defaults to Running rather than panicking — a forward-compat
            // guard, not a state this build ever writes itself.
            _ => Self::Running,
        }
    }

    pub fn is_finished(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// One row of the `detached_child` registry.
#[derive(Debug, Clone)]
pub struct DetachedChild {
    pub child_id: String,
    pub parent_session: String,
    pub name: String,
    pub model: String,
    pub status: DetachedChildStatus,
    pub result_ref: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

/// Detached results are delivered inline (no message-id indirection — the child's own session
/// transcript already holds the unbounded history), bounded to this many bytes so a runaway
/// child answer can't bloat the append-only global DB.
const MAX_DETACHED_RESULT_BYTES: usize = 16 * 1024;

/// Truncate `s` to [`MAX_DETACHED_RESULT_BYTES`] on a char boundary, same shape as
/// [`cap_result_json`] but with a smaller, spec-mandated cap for detached results specifically.
fn cap_detached_result(s: &str) -> std::borrow::Cow<'_, str> {
    if s.len() <= MAX_DETACHED_RESULT_BYTES {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut end = MAX_DETACHED_RESULT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    std::borrow::Cow::Owned(format!("{}…[truncated {} bytes]", &s[..end], s.len() - end))
}

fn row_to_detached_child(r: &rusqlite::Row) -> rusqlite::Result<DetachedChild> {
    let status: String = r.get(4)?;
    Ok(DetachedChild {
        child_id: r.get(0)?,
        parent_session: r.get(1)?,
        name: r.get(2)?,
        model: r.get(3)?,
        status: DetachedChildStatus::parse(&status),
        result_ref: r.get(5)?,
        created_at: r.get(6)?,
        finished_at: r.get(7)?,
    })
}

const SELECT_COLUMNS: &str =
    "child_id, parent_session, name, model, status, result_ref, created_at, finished_at";

impl Store {
    /// Register a just-admitted detached child (RFC retained-async-subagents). `child_id` is the
    /// child's own `session.id` (already created via [`Store::create_child_session`]); the row
    /// starts `running` with no result yet.
    pub fn create_detached_child(
        &self,
        child_id: &str,
        parent_session: &str,
        name: &str,
        model: &str,
    ) -> Result<()> {
        with_busy_retry(|| {
            let conn = self.lock()?;
            conn.execute(
                "INSERT INTO detached_child (child_id, parent_session, name, model, status) \
                 VALUES (?1, ?2, ?3, ?4, 'running')",
                (child_id, parent_session, name, model),
            )?;
            Ok(())
        })
    }

    /// Record a detached child's completion (successful or not). A no-op (returns `Ok(0)` rows
    /// affected, silently) if the child was already terminal — e.g. `cancel_detached_child` raced
    /// the child's own natural completion; the row that got there first wins.
    pub fn finish_detached_child(&self, child_id: &str, ok: bool, result_text: &str) -> Result<()> {
        let status = if ok {
            DetachedChildStatus::Done
        } else {
            DetachedChildStatus::Failed
        };
        let bounded = cap_detached_result(result_text);
        with_busy_retry(|| {
            let conn = self.lock()?;
            conn.execute(
                "UPDATE detached_child SET status = ?2, result_ref = ?3, \
                 finished_at = strftime('%s','now') \
                 WHERE child_id = ?1 AND status = 'running'",
                (child_id, status.as_str(), bounded.as_ref()),
            )?;
            Ok(())
        })
    }

    /// Best-effort cancel: flips a still-`running` row to `cancelled`. Returns `true` iff this
    /// call is the one that made the transition (so the caller knows whether to also abort the
    /// in-memory task, and doesn't report success for a child that had already finished).
    pub fn cancel_detached_child(&self, child_id: &str) -> Result<bool> {
        with_busy_retry(|| {
            let conn = self.lock()?;
            let n = conn.execute(
                "UPDATE detached_child SET status = 'cancelled', \
                 finished_at = strftime('%s','now') \
                 WHERE child_id = ?1 AND status = 'running'",
                [child_id],
            )?;
            Ok(n == 1)
        })
    }

    /// Every detached child spawned by `parent_session`, oldest first — the data behind
    /// `list_subagents`.
    pub fn list_detached_children(&self, parent_session: &str) -> Result<Vec<DetachedChild>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM detached_child WHERE parent_session = ?1 \
             ORDER BY created_at, child_id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([parent_session], row_to_detached_child)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// Finished-but-not-yet-injected-into-the-transcript children, oldest first — drained at the
    /// parent's next turn boundary (ADR-0004: delivery goes through the session queue as a
    /// labeled turn, not a new presenter surface).
    pub fn undelivered_detached_children(
        &self,
        parent_session: &str,
    ) -> Result<Vec<DetachedChild>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM detached_child \
             WHERE parent_session = ?1 AND status IN ('done', 'failed', 'cancelled') \
             AND delivered = 0 ORDER BY created_at, child_id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([parent_session], row_to_detached_child)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// Mark a detached child's result as delivered (injected into the parent transcript), so a
    /// later turn boundary doesn't repeat it.
    pub fn mark_detached_delivered(&self, child_id: &str) -> Result<()> {
        with_busy_retry(|| {
            let conn = self.lock()?;
            conn.execute(
                "UPDATE detached_child SET delivered = 1 WHERE child_id = ?1",
                [child_id],
            )?;
            Ok(())
        })
    }

    /// Reconcile `parent_session`'s detached children after a resume: a row still `running`
    /// belonged to an in-memory task on whatever process last admitted it. If THIS resume is
    /// happening in a fresh process (the common case — a daemon restart, or a fresh `forge run
    /// --resume`), that task no longer exists to ever call `finish_detached_child`, so the row
    /// would otherwise claim "running" forever. Flip it to `failed` with a clear note. Returns how
    /// many rows were reconciled.
    ///
    /// Must be called at most once per process per session (session construction/resume), NOT on
    /// every turn — a detached child legitimately still running in THIS process must never be
    /// swept by a later call.
    pub fn reconcile_running_detached_children(&self, parent_session: &str) -> Result<usize> {
        with_busy_retry(|| {
            let conn = self.lock()?;
            let n = conn.execute(
                "UPDATE detached_child SET status = 'failed', \
                 result_ref = 'error: interrupted — the process running this detached agent \
                 restarted before it finished', \
                 finished_at = strftime('%s','now') \
                 WHERE parent_session = ?1 AND status = 'running'",
                [parent_session],
            )?;
            Ok(n)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_parent_and_child() -> (Store, String, String) {
        let store = Store::open_in_memory().unwrap();
        let parent = store.create_session(".", "default").unwrap();
        let child = store.create_child_session(".", "default", &parent).unwrap();
        (store, parent, child)
    }

    #[test]
    fn create_then_list_round_trips() {
        let (store, parent, child) = store_with_parent_and_child();
        store
            .create_detached_child(&child, &parent, "researcher", "openai::gpt-test")
            .unwrap();
        let rows = store.list_detached_children(&parent).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].child_id, child);
        assert_eq!(rows[0].name, "researcher");
        assert_eq!(rows[0].model, "openai::gpt-test");
        assert_eq!(rows[0].status, DetachedChildStatus::Running);
        assert!(rows[0].result_ref.is_none());
    }

    #[test]
    fn finish_marks_done_and_stores_the_result() {
        let (store, parent, child) = store_with_parent_and_child();
        store
            .create_detached_child(&child, &parent, "researcher", "m")
            .unwrap();
        store
            .finish_detached_child(&child, true, "the answer")
            .unwrap();
        let rows = store.list_detached_children(&parent).unwrap();
        assert_eq!(rows[0].status, DetachedChildStatus::Done);
        assert_eq!(rows[0].result_ref.as_deref(), Some("the answer"));
        assert!(rows[0].finished_at.is_some());
    }

    #[test]
    fn finish_marks_failed_on_error() {
        let (store, parent, child) = store_with_parent_and_child();
        store
            .create_detached_child(&child, &parent, "n", "m")
            .unwrap();
        store.finish_detached_child(&child, false, "boom").unwrap();
        let rows = store.list_detached_children(&parent).unwrap();
        assert_eq!(rows[0].status, DetachedChildStatus::Failed);
    }

    #[test]
    fn finish_clamps_an_oversized_result() {
        let (store, parent, child) = store_with_parent_and_child();
        store
            .create_detached_child(&child, &parent, "n", "m")
            .unwrap();
        let huge = "A".repeat(MAX_DETACHED_RESULT_BYTES * 3);
        store.finish_detached_child(&child, true, &huge).unwrap();
        let rows = store.list_detached_children(&parent).unwrap();
        let stored = rows[0].result_ref.as_deref().unwrap();
        assert!(stored.len() < huge.len());
        assert!(stored.contains("truncated"));
    }

    #[test]
    fn undelivered_only_returns_finished_and_not_yet_delivered() {
        let (store, parent, child) = store_with_parent_and_child();
        store
            .create_detached_child(&child, &parent, "n", "m")
            .unwrap();
        assert!(store
            .undelivered_detached_children(&parent)
            .unwrap()
            .is_empty());
        store
            .finish_detached_child(&child, true, "done text")
            .unwrap();
        let pending = store.undelivered_detached_children(&parent).unwrap();
        assert_eq!(pending.len(), 1);
        store.mark_detached_delivered(&child).unwrap();
        assert!(store
            .undelivered_detached_children(&parent)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cancel_only_transitions_a_running_child() {
        let (store, parent, child) = store_with_parent_and_child();
        store
            .create_detached_child(&child, &parent, "n", "m")
            .unwrap();
        assert!(store.cancel_detached_child(&child).unwrap());
        let rows = store.list_detached_children(&parent).unwrap();
        assert_eq!(rows[0].status, DetachedChildStatus::Cancelled);
        // Already terminal: a second cancel is a no-op, not a forced re-cancel.
        assert!(!store.cancel_detached_child(&child).unwrap());
    }

    #[test]
    fn cancel_does_not_clobber_a_child_that_already_finished() {
        let (store, parent, child) = store_with_parent_and_child();
        store
            .create_detached_child(&child, &parent, "n", "m")
            .unwrap();
        store
            .finish_detached_child(&child, true, "already done")
            .unwrap();
        // The cancel lost the race — the child had already finished successfully.
        assert!(!store.cancel_detached_child(&child).unwrap());
        let rows = store.list_detached_children(&parent).unwrap();
        assert_eq!(rows[0].status, DetachedChildStatus::Done);
    }

    #[test]
    fn reconcile_fails_running_children_and_leaves_finished_ones_alone() {
        let (store, parent, child) = store_with_parent_and_child();
        let child2 = store.create_child_session(".", "default", &parent).unwrap();
        store
            .create_detached_child(&child, &parent, "a", "m")
            .unwrap();
        store
            .create_detached_child(&child2, &parent, "b", "m")
            .unwrap();
        store
            .finish_detached_child(&child2, true, "already done")
            .unwrap();

        let n = store.reconcile_running_detached_children(&parent).unwrap();
        assert_eq!(n, 1, "only the still-running child is reconciled");

        let rows = store.list_detached_children(&parent).unwrap();
        let a = rows.iter().find(|r| r.child_id == child).unwrap();
        assert_eq!(a.status, DetachedChildStatus::Failed);
        assert!(a.result_ref.as_deref().unwrap().contains("interrupted"));
        let b = rows.iter().find(|r| r.child_id == child2).unwrap();
        assert_eq!(
            b.status,
            DetachedChildStatus::Done,
            "untouched — it already finished"
        );
        assert_eq!(b.result_ref.as_deref(), Some("already done"));
    }

    #[test]
    fn reconcile_is_scoped_to_its_own_parent_session() {
        let (store, parent_a, child_a) = store_with_parent_and_child();
        let parent_b = store.create_session(".", "default").unwrap();
        let child_b = store
            .create_child_session(".", "default", &parent_b)
            .unwrap();
        store
            .create_detached_child(&child_a, &parent_a, "a", "m")
            .unwrap();
        store
            .create_detached_child(&child_b, &parent_b, "b", "m")
            .unwrap();

        store
            .reconcile_running_detached_children(&parent_a)
            .unwrap();

        let rows_b = store.list_detached_children(&parent_b).unwrap();
        assert_eq!(
            rows_b[0].status,
            DetachedChildStatus::Running,
            "a different parent's still-running child must not be touched"
        );
    }
}
