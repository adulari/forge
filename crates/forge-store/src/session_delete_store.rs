//! Permanent session deletion: what a delete would remove (for the confirmation), and the delete
//! itself, scrubbed so the content does not linger in free pages or the write-ahead log.

use super::*;

/// One session in the tree a delete removes (the session itself, then its subagent descendants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    pub id: String,
    pub cwd: String,
    pub worktree: Option<String>,
}

/// What deleting a session removes, for the confirmation and the refusal checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDeletePreview {
    pub id: String,
    pub title: Option<String>,
    /// Active user messages in the root session.
    pub turns: i64,
    /// Messages across the whole tree.
    pub messages: i64,
    pub tool_calls: i64,
    /// Subagent sessions below the root.
    pub children: usize,
    /// The root's recorded worktree, whether or not it still exists on disk.
    pub worktree: Option<String>,
    /// Another device may hold this session (a tombstone is written so it disappears there too).
    pub synced: bool,
    /// An Anywhere handoff froze this tree.
    pub frozen: bool,
    /// Detached subagents of the tree whose task is still running somewhere.
    pub running_subagents: i64,
    /// Saved memories distilled from the tree; they are the person's own notes and are kept.
    pub memories_kept: i64,
    /// Root first, then descendants.
    pub targets: Vec<DeleteTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDeleteReport {
    pub deleted: Vec<DeleteTarget>,
    /// The write-ahead log was truncated, so no earlier copy of the content is left in it. False
    /// when another connection held it open; the next checkpoint finishes the job.
    pub wal_truncated: bool,
}

const TREE_CTE: &str = "WITH RECURSIVE session_tree(id) AS (
        SELECT id FROM session WHERE id = ?1
        UNION ALL
        SELECT child.id FROM session child
        JOIN session_tree parent ON child.parent_session_id = parent.id
    )";

/// Journal kinds whose payload names its session directly (`session_id` in the JSON).
const SESSION_KEYED_KINDS: &str = "('message','compaction','checkpoint')";

/// Journal kinds keyed to a message instead (`message_id` in the JSON, no `session_id`).
const MESSAGE_KEYED_KINDS: &str = "('tool_call','routing_decision','usage')";

impl Store {
    /// The tree rooted at `session_id` and what deleting it takes with it, or `None` if absent.
    pub fn session_delete_preview(&self, session_id: &str) -> Result<Option<SessionDeletePreview>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "{TREE_CTE} SELECT s.id, s.cwd, s.worktree_path FROM session s
             JOIN session_tree t ON t.id = s.id"
        ))?;
        let mut targets = stmt
            .query_map([session_id], |row| {
                Ok(DeleteTarget {
                    id: row.get(0)?,
                    cwd: row.get(1)?,
                    worktree: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        let Some(root) = targets.iter().position(|t| t.id == session_id) else {
            return Ok(None);
        };
        targets.swap(0, root);
        let count = |sql: &str| -> Result<i64> {
            Ok(conn.query_row(&format!("{TREE_CTE} {sql}"), [session_id], |r| r.get(0))?)
        };
        let messages = count(
            "SELECT COUNT(*) FROM message WHERE session_id IN (SELECT id FROM session_tree)",
        )?;
        let tool_calls = count(
            "SELECT COUNT(*) FROM tool_call WHERE message_id IN
               (SELECT id FROM message WHERE session_id IN (SELECT id FROM session_tree))",
        )?;
        let memories_kept = count(
            "SELECT COUNT(*) FROM memory WHERE source_session IN (SELECT id FROM session_tree)",
        )?;
        let frozen = count(
            "SELECT COUNT(*) FROM anywhere_handoff_session_state
             WHERE session_id IN (SELECT id FROM session_tree)",
        )? > 0;
        let running_subagents = count(
            "SELECT COUNT(*) FROM detached_child WHERE status = 'running'
               AND (parent_session IN (SELECT id FROM session_tree)
                    OR child_id IN (SELECT id FROM session_tree))",
        )?;
        let synced = count(
            "SELECT (SELECT COUNT(*) FROM sync_journal WHERE record_kind='session'
                       AND stable_id IN (SELECT id FROM session_tree))
                  + (SELECT COUNT(*) FROM anywhere_sync_materialized WHERE record_kind='session'
                       AND stable_id IN (SELECT id FROM session_tree))",
        )? > 0;
        let turns = conn.query_row(
            "SELECT COUNT(*) FROM message WHERE session_id = ?1 AND role = 'user' AND active = 1",
            [session_id],
            |r| r.get(0),
        )?;
        let (title, worktree) = conn.query_row(
            "SELECT title, worktree_path FROM session WHERE id = ?1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Some(SessionDeletePreview {
            id: session_id.to_string(),
            title,
            turns,
            messages,
            tool_calls,
            children: targets.len() - 1,
            worktree,
            synced,
            frozen,
            running_subagents,
            memories_kept,
            targets,
        }))
    }

    /// Permanently delete a persisted session, its subagent descendants, and every row keyed to
    /// them. Queue and schedule history is kept but detached. The caller owns the running,
    /// terminal and worktree safety checks; the store refuses frozen Anywhere handoffs itself.
    ///
    /// The delete runs with `secure_delete` so freed pages are zeroed, journalled sync copies of
    /// the content are purged, and the WAL is truncated afterwards so the old page images are not
    /// left behind. `VACUUM` (shrinking the file) is separate: see [`Store::vacuum`].
    pub fn delete_session_report(&self, session_id: &str) -> Result<Option<SessionDeleteReport>> {
        let Some(preview) = self.session_delete_preview(session_id)? else {
            return Ok(None);
        };
        let conn = self.lock()?;
        let previous: i64 = conn.query_row("PRAGMA secure_delete", [], |r| r.get(0))?;
        conn.pragma_update(None, "secure_delete", 1)?;
        let outcome = delete_tree(&conn, &preview.targets);
        let _ = conn.pragma_update(None, "secure_delete", previous);
        outcome?;
        let wal_truncated = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                r.get::<_, i64>(0)
            })
            .map(|busy| busy == 0)
            .unwrap_or(false);
        Ok(Some(SessionDeleteReport {
            deleted: preview.targets,
            wal_truncated,
        }))
    }
}

fn delete_tree(conn: &rusqlite::Connection, targets: &[DeleteTarget]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for t in targets {
        let blocked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM anywhere_handoff_session_state WHERE session_id=?1)",
            [&t.id],
            |row| row.get(0),
        )?;
        if blocked {
            return Err(StoreError::InvalidValue(
                "session tree is frozen by an Anywhere handoff".into(),
            ));
        }
    }
    for t in targets {
        let id = &t.id;
        // Tables with no foreign key to `session`: nothing session-scoped may outlive it.
        tx.execute(
            "DELETE FROM live_activity_token WHERE session_id = ?1",
            [id],
        )?;
        tx.execute("DELETE FROM mesh_outcome WHERE session_id = ?1", [id])?;
        tx.execute("DELETE FROM harness_refinement WHERE session_id = ?1", [id])?;
        tx.execute(
            "DELETE FROM fleet_message WHERE target_session_id = ?1",
            [id],
        )?;
        tx.execute(
            "DELETE FROM dispatch WHERE coordinator_session_id = ?1",
            [id],
        )?;
        // Run history is user-visible execution history, not an owned artifact: keep it detached.
        tx.execute(
            "UPDATE queue_task SET session_id = NULL WHERE session_id = ?1",
            [id],
        )?;
        tx.execute(
            "UPDATE schedule_run SET session_id = NULL WHERE session_id = ?1",
            [id],
        )?;
        tx.execute(
            "UPDATE dispatch_item SET session_id = NULL WHERE session_id = ?1",
            [id],
        )?;
    }
    for t in targets.iter().rev() {
        let id = &t.id;
        let synced: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_journal WHERE record_kind = 'session' AND stable_id = ?1)
                 OR EXISTS(SELECT 1 FROM anywhere_sync_materialized
                           WHERE record_kind = 'session' AND stable_id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        if synced {
            crate::append_sync_revision(
                &tx,
                "session",
                id,
                crate::SyncJournalOperation::Tombstone,
                &[],
            )?;
        }
        // The journal keeps every revision of a synced record. Purge the content copies (and any
        // queued upload envelopes, which cascade); the tombstone is what other devices need.
        // Must run before the session row goes: it resolves message ids through `message`.
        tx.execute(
            &format!(
                "DELETE FROM sync_journal WHERE record_kind IN {MESSAGE_KEYED_KINDS}
                   AND json_extract(CAST(payload AS TEXT), '$.message_id') IN
                       (SELECT id FROM message WHERE session_id = ?1)"
            ),
            [id],
        )?;
        tx.execute(
            &format!(
                "DELETE FROM sync_journal WHERE record_kind IN {SESSION_KEYED_KINDS}
                   AND json_extract(CAST(payload AS TEXT), '$.session_id') = ?1"
            ),
            [id],
        )?;
        tx.execute(
            "DELETE FROM sync_journal WHERE record_kind = 'session' AND stable_id = ?1
               AND operation = 'upsert'",
            [id],
        )?;
        tx.execute("DELETE FROM session WHERE id = ?1", [id])?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::Role;

    const SECRET: &str = "ZEBRA-QUINCE-SECRET-4471";

    fn session_with_content(store: &Store) -> (String, String) {
        let sid = store.create_session("/proj", "Smith").unwrap();
        let child = store.create_child_session("/proj", "Smith", &sid).unwrap();
        store
            .add_message(&sid, 0, Role::User, &format!("hello {SECRET}"), None)
            .unwrap();
        store
            .add_message(&child, 0, Role::Assistant, "child work", None)
            .unwrap();
        (sid, child)
    }

    #[test]
    fn preview_counts_the_tree() {
        let store = Store::open_in_memory().unwrap();
        let (sid, child) = session_with_content(&store);
        store.set_session_title(&sid, "my title").unwrap();
        let p = store.session_delete_preview(&sid).unwrap().unwrap();
        assert_eq!(p.title.as_deref(), Some("my title"));
        assert_eq!((p.turns, p.messages, p.children), (1, 2, 1));
        assert_eq!(p.targets[0].id, sid);
        assert_eq!(p.targets[1].id, child);
        assert!(!p.frozen && !p.synced);
        assert!(store.session_delete_preview("nope").unwrap().is_none());
    }

    #[test]
    fn delete_removes_messages_children_and_detaches_history() {
        let store = Store::open_in_memory().unwrap();
        let (sid, child) = session_with_content(&store);
        {
            let conn = store.lock().unwrap();
            conn.execute(
                "INSERT INTO queue_task (id, task, cwd, status, session_id) VALUES ('q','t','/','done',?1)",
                [&sid],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO fleet_message (id, sender_kind, sender_label, target_session_id, body, mode)
                 VALUES ('f','cli','cli',?1,'b','steer')",
                [&child],
            )
            .unwrap();
        }
        let report = store.delete_session_report(&sid).unwrap().unwrap();
        assert_eq!(report.deleted.len(), 2);
        assert!(store.load_all_messages(&sid).unwrap().is_empty());
        assert!(!store.session_exists(&child).unwrap());
        let conn = store.lock().unwrap();
        let fleet: i64 = conn
            .query_row("SELECT COUNT(*) FROM fleet_message", [], |r| r.get(0))
            .unwrap();
        let queue: Option<String> = conn
            .query_row("SELECT session_id FROM queue_task WHERE id='q'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!((fleet, queue), (0, None));
    }

    #[test]
    fn frozen_handoff_refuses_and_leaves_the_session() {
        let store = Store::open_in_memory().unwrap();
        let (sid, _) = session_with_content(&store);
        store
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO anywhere_handoff_session_state (session_id, capsule_id, state) VALUES (?1, 'cap', 'source_pending')",
                [&sid],
            )
            .unwrap();
        assert!(store.session_delete_preview(&sid).unwrap().unwrap().frozen);
        assert!(store.delete_session_report(&sid).is_err());
        assert!(store.session_exists(&sid).unwrap());
    }

    #[test]
    fn synced_session_gets_a_tombstone_and_loses_journalled_content() {
        let store = Store::open_in_memory().unwrap();
        store.set_sync_journal_enabled(true).unwrap();
        let (sid, _) = session_with_content(&store);
        let journalled = |kind: &str| -> i64 {
            store
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM sync_journal WHERE record_kind = ?1",
                    [kind],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let mid = store
            .add_message(&sid, 1, Role::Assistant, "ran a tool", None)
            .unwrap();
        store
            .record_tool_call(&mid, "shell", "{}", "TOOL-SECRET", "auto", "ok")
            .unwrap();
        store
            .record_routing(&mid, forge_types::TaskTier::Standard, "m", "why")
            .unwrap();
        store
            .record_usage(&sid, &mid, &forge_types::Usage::default(), Some("m"))
            .unwrap();
        let other = store.create_session("/other", "Smith").unwrap();
        let other_msg = store
            .add_message(&other, 0, Role::Assistant, "keep", None)
            .unwrap();
        store
            .record_tool_call(&other_msg, "shell", "{}", "KEEP", "auto", "ok")
            .unwrap();
        for kind in ["message", "tool_call", "routing_decision", "usage"] {
            assert!(journalled(kind) > 0, "{kind} was never journalled");
        }
        assert!(store.session_delete_preview(&sid).unwrap().unwrap().synced);
        store.delete_session(&sid).unwrap();
        assert_eq!(journalled("message"), 1, "only the other session's remains");
        assert_eq!(journalled("tool_call"), 1);
        assert_eq!(journalled("routing_decision"), 0);
        assert_eq!(journalled("usage"), 0);
        let ops: Vec<String> = {
            let conn = store.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT operation FROM sync_journal WHERE record_kind='session' AND stable_id=?1")
                .unwrap();
            stmt.query_map([&sid], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(ops, vec!["tombstone".to_string()]);
    }

    #[test]
    fn deleted_content_bytes_are_not_left_in_the_database_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forge.db");
        let store = Store::open(&path).unwrap();
        let (sid, _) = session_with_content(&store);
        let other = store.create_session("/other", "Smith").unwrap();
        store
            .add_message(&other, 0, Role::User, "keep me", None)
            .unwrap();
        let on_disk = |needle: &str| {
            ["", "-wal"].iter().any(|ext| {
                std::fs::read(format!("{}{ext}", path.display()))
                    .map(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes()))
                    .unwrap_or(false)
            })
        };
        assert!(
            on_disk(SECRET),
            "precondition: content is on disk before the delete"
        );
        let report = store.delete_session_report(&sid).unwrap().unwrap();
        assert!(report.wal_truncated);
        assert!(
            !on_disk(SECRET),
            "deleted content survived in the db or its WAL"
        );
        assert!(on_disk("keep me"));
        let secure: i64 = store
            .lock()
            .unwrap()
            .query_row("PRAGMA secure_delete", [], |r| r.get(0))
            .unwrap();
        assert_eq!(secure, 0, "secure_delete is restored after the delete");
    }

    #[test]
    fn vacuum_after_a_delete_leaves_no_trace_even_after_rebalancing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forge.db");
        let store = Store::open(&path).unwrap();
        let mut victim = String::new();
        for i in 0..40 {
            let sid = store.create_session("/proj", "Smith").unwrap();
            let text = format!("MARK-{i:02}-END {}", "x".repeat(300));
            store.add_message(&sid, 0, Role::User, &text, None).unwrap();
            if i == 17 {
                victim = sid;
            }
        }
        store.delete_session_report(&victim).unwrap();
        store.vacuum().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let has = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
        assert!(!has("MARK-17-END"), "the deleted session survived a vacuum");
        assert!(has("MARK-16-END") && has("MARK-18-END"));
    }
}
