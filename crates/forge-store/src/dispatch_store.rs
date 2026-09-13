//! Plan-and-dispatch persistence (`forge_core::dispatch`, migration 35): one `dispatch` row per
//! coordinator session and one `dispatch_item` row per proposed work item. The proposal, the user's
//! approval, and every item's progress live here so a dispatch survives the board closing and the
//! daemon restarting.

use super::*;

/// Lifecycle of a whole dispatch.
pub mod dispatch_status {
    /// The coordinator is reading the project and has not proposed a split yet.
    pub const PLANNING: &str = "planning";
    /// A split is waiting for the user's approval.
    pub const PROPOSED: &str = "proposed";
    /// Approved; items are queued or running.
    pub const RUNNING: &str = "running";
    /// Every item reached a terminal state.
    pub const DONE: &str = "done";
    /// The user cancelled it.
    pub const CANCELLED: &str = "cancelled";

    /// Whether nothing more will happen to this dispatch.
    pub fn is_terminal(status: &str) -> bool {
        status == DONE || status == CANCELLED
    }
}

/// Lifecycle of one item.
pub mod dispatch_item_status {
    /// Part of a proposal the user has not decided on.
    pub const PROPOSED: &str = "proposed";
    /// The user left it out of the approval.
    pub const SKIPPED: &str = "skipped";
    /// Approved; waiting for its dependencies or a free slot.
    pub const QUEUED: &str = "queued";
    /// Its session is working.
    pub const RUNNING: &str = "running";
    /// Its session finished a turn successfully.
    pub const SUCCEEDED: &str = "succeeded";
    /// Its session finished a turn without succeeding.
    pub const FAILED: &str = "failed";
    /// Its session went away (archived, or it died) before finishing.
    pub const STOPPED: &str = "stopped";
    /// Cancelled before it started — by the user, or because a dependency did not succeed.
    pub const CANCELLED: &str = "cancelled";
    /// Its worktree was merged back.
    pub const MERGED: &str = "merged";
    /// Its worktree was discarded.
    pub const DISCARDED: &str = "discarded";

    /// Whether nothing more is expected from this item.
    pub fn is_terminal(status: &str) -> bool {
        matches!(
            status,
            SUCCEEDED | FAILED | STOPPED | CANCELLED | SKIPPED | MERGED | DISCARDED
        )
    }
}

/// One dispatch with its items (ascending by index).
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchRow {
    pub id: String,
    pub coordinator_session_id: String,
    pub cwd: String,
    /// The user's original request.
    pub prompt: String,
    /// The coordinator's summary of its latest proposal ("" while planning).
    pub summary: String,
    pub status: String,
    /// Whether each item runs in its own worktree.
    pub worktree: bool,
    /// Permission mode for the item sessions (`None` = the daemon default).
    pub permission_mode: Option<String>,
    pub max_running: i64,
    /// Most items the coordinator may propose.
    pub max_items: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub items: Vec<DispatchItemRow>,
}

/// One work item of a dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchItemRow {
    pub dispatch_id: String,
    /// 1-based position in the proposal.
    pub idx: i64,
    pub title: String,
    pub prompt: String,
    /// 1-based indices of the items this one waits for.
    pub depends_on: Vec<i64>,
    pub status: String,
    pub session_id: Option<String>,
    /// The session's last turn outcome (`success` / `failed`) or stop reason, when known.
    pub outcome: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

/// A proposed item as the store receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDispatchItem {
    pub title: String,
    pub prompt: String,
    pub depends_on: Vec<i64>,
}

const DISPATCH_COLUMNS: &str =
    "id, coordinator_session_id, cwd, prompt, summary, status, worktree, \
     permission_mode, max_running, max_items, created_at, updated_at";
const ITEM_COLUMNS: &str =
    "dispatch_id, idx, title, prompt, depends_on, status, session_id, outcome, started_at, finished_at";

impl Store {
    /// Start a dispatch in the `planning` state for a freshly created coordinator session.
    #[allow(clippy::too_many_arguments)]
    pub fn create_dispatch(
        &self,
        id: &str,
        coordinator_session_id: &str,
        cwd: &str,
        prompt: &str,
        worktree: bool,
        permission_mode: Option<&str>,
        max_running: i64,
        max_items: i64,
    ) -> Result<()> {
        with_busy_retry(|| {
            self.lock()?.execute(
                "INSERT INTO dispatch (id, coordinator_session_id, cwd, prompt, status, worktree, \
                 permission_mode, max_running, max_items) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    id,
                    coordinator_session_id,
                    cwd,
                    prompt,
                    dispatch_status::PLANNING,
                    i64::from(worktree),
                    permission_mode,
                    max_running,
                    max_items
                ],
            )?;
            Ok(())
        })
    }

    /// Replace the dispatch's proposal (summary + items) and mark it `proposed`. Refused unless the
    /// dispatch is still `planning` or `proposed` — an approved split is never rewritten.
    pub fn replace_dispatch_proposal(
        &self,
        id: &str,
        summary: &str,
        items: &[NewDispatchItem],
    ) -> Result<()> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let status: Option<String> = tx
                .query_row("SELECT status FROM dispatch WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            match status.as_deref() {
                None => {
                    return Err(StoreError::InvalidValue(format!("no dispatch {id}")));
                }
                Some(dispatch_status::PLANNING) | Some(dispatch_status::PROPOSED) => {}
                Some(other) => {
                    return Err(StoreError::InvalidValue(format!(
                        "the dispatch is already {other}; its split can no longer change"
                    )));
                }
            }
            tx.execute("DELETE FROM dispatch_item WHERE dispatch_id = ?1", [id])?;
            for (i, item) in items.iter().enumerate() {
                let deps = serde_json::to_string(&item.depends_on)
                    .map_err(|e| StoreError::InvalidValue(e.to_string()))?;
                tx.execute(
                    "INSERT INTO dispatch_item (dispatch_id, idx, title, prompt, depends_on, status) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        id,
                        (i + 1) as i64,
                        item.title,
                        item.prompt,
                        deps,
                        dispatch_item_status::PROPOSED
                    ],
                )?;
            }
            tx.execute(
                "UPDATE dispatch SET summary = ?2, status = ?3, updated_at = strftime('%s','now') \
                 WHERE id = ?1",
                rusqlite::params![id, summary, dispatch_status::PROPOSED],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Set the dispatch's own status.
    pub fn set_dispatch_status(&self, id: &str, status: &str) -> Result<()> {
        with_busy_retry(|| {
            self.lock()?.execute(
                "UPDATE dispatch SET status = ?2, updated_at = strftime('%s','now') WHERE id = ?1",
                (id, status),
            )?;
            Ok(())
        })
    }

    /// Update one item. `session_id`/`outcome` of `None` keep the stored value. `started_at` is
    /// stamped the first time an item becomes `running`; `finished_at` whenever it reaches a
    /// terminal state (and cleared if it runs again).
    pub fn set_dispatch_item(
        &self,
        id: &str,
        idx: i64,
        status: &str,
        session_id: Option<&str>,
        outcome: Option<&str>,
    ) -> Result<()> {
        let terminal = i64::from(dispatch_item_status::is_terminal(status));
        let running = i64::from(status == dispatch_item_status::RUNNING);
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE dispatch_item SET status = ?3, \
                   session_id = COALESCE(?4, session_id), \
                   outcome = COALESCE(?5, outcome), \
                   started_at = CASE WHEN ?7 = 1 AND started_at IS NULL \
                                     THEN strftime('%s','now') ELSE started_at END, \
                   finished_at = CASE WHEN ?6 = 1 THEN strftime('%s','now') \
                                      WHEN ?7 = 1 THEN NULL ELSE finished_at END \
                 WHERE dispatch_id = ?1 AND idx = ?2",
                rusqlite::params![id, idx, status, session_id, outcome, terminal, running],
            )?;
            tx.execute(
                "UPDATE dispatch SET updated_at = strftime('%s','now') WHERE id = ?1",
                [id],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// One dispatch with its items.
    pub fn dispatch(&self, id: &str) -> Result<Option<DispatchRow>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                &format!("SELECT {DISPATCH_COLUMNS} FROM dispatch WHERE id = ?1"),
                [id],
                map_dispatch,
            )
            .optional()?;
        row.map(|r| with_items(&conn, r)).transpose()
    }

    /// The dispatch this session coordinates, if any (the newest, should there be several).
    pub fn dispatch_for_coordinator(&self, session_id: &str) -> Result<Option<DispatchRow>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                &format!(
                    "SELECT {DISPATCH_COLUMNS} FROM dispatch WHERE coordinator_session_id = ?1 \
                     ORDER BY created_at DESC, rowid DESC LIMIT 1"
                ),
                [session_id],
                map_dispatch,
            )
            .optional()?;
        row.map(|r| with_items(&conn, r)).transpose()
    }

    /// The dispatch and item index a worker session belongs to, if any.
    pub fn dispatch_item_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<(DispatchRow, i64)>> {
        let found: Option<(String, i64)> = self
            .lock()?
            .query_row(
                "SELECT dispatch_id, idx FROM dispatch_item WHERE session_id = ?1 \
                 ORDER BY rowid DESC LIMIT 1",
                [session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((dispatch_id, idx)) = found else {
            return Ok(None);
        };
        Ok(self.dispatch(&dispatch_id)?.map(|d| (d, idx)))
    }

    /// Recent dispatches, most recently updated first.
    pub fn list_dispatches(&self, limit: usize) -> Result<Vec<DispatchRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {DISPATCH_COLUMNS} FROM dispatch ORDER BY updated_at DESC, rowid DESC LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map([limit as i64], map_dispatch)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        rows.into_iter().map(|r| with_items(&conn, r)).collect()
    }

    /// Dispatches that still need the daemon: planning, proposed, or running.
    pub fn active_dispatches(&self) -> Result<Vec<DispatchRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {DISPATCH_COLUMNS} FROM dispatch WHERE status IN (?1, ?2, ?3) \
             ORDER BY created_at, rowid"
        ))?;
        let rows = stmt
            .query_map(
                [
                    dispatch_status::PLANNING,
                    dispatch_status::PROPOSED,
                    dispatch_status::RUNNING,
                ],
                map_dispatch,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        rows.into_iter().map(|r| with_items(&conn, r)).collect()
    }
}

fn map_dispatch(r: &rusqlite::Row<'_>) -> rusqlite::Result<DispatchRow> {
    Ok(DispatchRow {
        id: r.get(0)?,
        coordinator_session_id: r.get(1)?,
        cwd: r.get(2)?,
        prompt: r.get(3)?,
        summary: r.get(4)?,
        status: r.get(5)?,
        worktree: r.get::<_, i64>(6)? != 0,
        permission_mode: r.get(7)?,
        max_running: r.get(8)?,
        max_items: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
        items: Vec::new(),
    })
}

fn with_items(conn: &Connection, mut row: DispatchRow) -> Result<DispatchRow> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ITEM_COLUMNS} FROM dispatch_item WHERE dispatch_id = ?1 ORDER BY idx"
    ))?;
    row.items = stmt
        .query_map([&row.id], |r| {
            let deps: String = r.get(4)?;
            Ok(DispatchItemRow {
                dispatch_id: r.get(0)?,
                idx: r.get(1)?,
                title: r.get(2)?,
                prompt: r.get(3)?,
                depends_on: serde_json::from_str(&deps).unwrap_or_default(),
                status: r.get(5)?,
                session_id: r.get(6)?,
                outcome: r.get(7)?,
                started_at: r.get(8)?,
                finished_at: r.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, deps: &[i64]) -> NewDispatchItem {
        NewDispatchItem {
            title: title.into(),
            prompt: format!("do {title}"),
            depends_on: deps.to_vec(),
        }
    }

    #[test]
    fn a_dispatch_moves_from_planning_to_proposed_with_its_items() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_dispatch(
                "d1",
                "coord",
                "/repo",
                "split it",
                true,
                Some("accept-edits"),
                3,
                8,
            )
            .unwrap();
        let d = store.dispatch("d1").unwrap().unwrap();
        assert_eq!(d.status, dispatch_status::PLANNING);
        assert!(d.items.is_empty());
        assert!(d.worktree);
        assert_eq!(d.permission_mode.as_deref(), Some("accept-edits"));
        assert_eq!(d.max_running, 3);
        assert_eq!(d.max_items, 8);

        store
            .replace_dispatch_proposal("d1", "the plan", &[item("a", &[]), item("b", &[1])])
            .unwrap();
        let d = store.dispatch("d1").unwrap().unwrap();
        assert_eq!(d.status, dispatch_status::PROPOSED);
        assert_eq!(d.summary, "the plan");
        assert_eq!(d.items.len(), 2);
        assert_eq!(d.items[1].idx, 2);
        assert_eq!(d.items[1].depends_on, vec![1]);
        assert_eq!(d.items[0].status, dispatch_item_status::PROPOSED);
    }

    #[test]
    fn a_revision_replaces_the_items_but_an_approved_split_is_frozen() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_dispatch("d1", "coord", "/repo", "p", false, None, 4, 8)
            .unwrap();
        store
            .replace_dispatch_proposal(
                "d1",
                "v1",
                &[item("a", &[]), item("b", &[]), item("c", &[])],
            )
            .unwrap();
        store
            .replace_dispatch_proposal("d1", "v2", &[item("only", &[])])
            .unwrap();
        let d = store.dispatch("d1").unwrap().unwrap();
        assert_eq!(d.summary, "v2");
        assert_eq!(d.items.len(), 1);
        assert_eq!(d.items[0].title, "only");

        store
            .set_dispatch_status("d1", dispatch_status::RUNNING)
            .unwrap();
        let err = store
            .replace_dispatch_proposal("d1", "v3", &[item("x", &[])])
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidValue(_)));
    }

    #[test]
    fn item_updates_stamp_start_and_finish_and_keep_unspecified_fields() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_dispatch("d1", "coord", "/repo", "p", true, None, 4, 8)
            .unwrap();
        store
            .replace_dispatch_proposal("d1", "s", &[item("a", &[])])
            .unwrap();
        store
            .set_dispatch_item("d1", 1, dispatch_item_status::RUNNING, Some("sess-a"), None)
            .unwrap();
        let it = &store.dispatch("d1").unwrap().unwrap().items[0];
        assert_eq!(it.session_id.as_deref(), Some("sess-a"));
        assert!(it.started_at.is_some());
        assert!(it.finished_at.is_none());

        store
            .set_dispatch_item(
                "d1",
                1,
                dispatch_item_status::SUCCEEDED,
                None,
                Some("success"),
            )
            .unwrap();
        let it = &store.dispatch("d1").unwrap().unwrap().items[0];
        assert_eq!(
            it.session_id.as_deref(),
            Some("sess-a"),
            "None keeps the session id"
        );
        assert_eq!(it.outcome.as_deref(), Some("success"));
        assert!(it.finished_at.is_some());

        let (d, idx) = store.dispatch_item_for_session("sess-a").unwrap().unwrap();
        assert_eq!((d.id.as_str(), idx), ("d1", 1));
        assert!(store.dispatch_item_for_session("nobody").unwrap().is_none());
    }

    #[test]
    fn coordinator_lookup_and_the_active_list_follow_status() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_dispatch("d1", "coord-1", "/repo", "p", true, None, 4, 8)
            .unwrap();
        store
            .create_dispatch("d2", "coord-2", "/repo", "p", true, None, 4, 8)
            .unwrap();
        assert_eq!(
            store
                .dispatch_for_coordinator("coord-2")
                .unwrap()
                .unwrap()
                .id,
            "d2"
        );
        assert!(store.dispatch_for_coordinator("coord-9").unwrap().is_none());
        store
            .set_dispatch_status("d1", dispatch_status::DONE)
            .unwrap();
        let active: Vec<String> = store
            .active_dispatches()
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(active, vec!["d2".to_string()]);
        assert_eq!(store.list_dispatches(10).unwrap().len(), 2);
    }

    #[test]
    fn terminal_states_are_exactly_the_ones_that_expect_nothing_more() {
        for s in [
            dispatch_item_status::SUCCEEDED,
            dispatch_item_status::FAILED,
            dispatch_item_status::STOPPED,
            dispatch_item_status::CANCELLED,
            dispatch_item_status::SKIPPED,
            dispatch_item_status::MERGED,
            dispatch_item_status::DISCARDED,
        ] {
            assert!(dispatch_item_status::is_terminal(s), "{s}");
        }
        for s in [
            dispatch_item_status::PROPOSED,
            dispatch_item_status::QUEUED,
            dispatch_item_status::RUNNING,
        ] {
            assert!(!dispatch_item_status::is_terminal(s), "{s}");
        }
        assert!(dispatch_status::is_terminal(dispatch_status::DONE));
        assert!(!dispatch_status::is_terminal(dispatch_status::RUNNING));
    }
}
