//! The lineage of fresh-worker hand-offs (migration 58): a goal that wedged or wore its session
//! out continues in a new session on the same worktree, and the successor may search and read its
//! predecessors, and only them.

use super::*;

/// How many predecessors a chain walk follows; a corrupt cycle ends here rather than looping.
const MAX_CHAIN: usize = 32;

/// One hand-off: `predecessor_id` gave the goal to `successor_id`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RelayLink {
    pub predecessor_id: String,
    pub successor_id: String,
    /// `wedged`, `stalled` or `worn`.
    pub reason: String,
    /// The line that said why (the evaluator's reason, the failing assertions).
    pub detail: String,
    pub created_at: i64,
}

const COLS: &str = "predecessor_id, successor_id, reason, detail, created_at";

fn link_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<RelayLink> {
    Ok(RelayLink {
        predecessor_id: row.get(0)?,
        successor_id: row.get(1)?,
        reason: row.get(2)?,
        detail: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl Store {
    /// Record that `predecessor` handed its goal to `successor`, with the brief the successor
    /// starts from. Recording twice for one successor keeps the first.
    pub fn record_relay(
        &self,
        predecessor: &str,
        successor: &str,
        reason: &str,
        detail: &str,
        brief: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT OR IGNORE INTO session_relay \
             (successor_id, predecessor_id, reason, detail, brief, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                successor,
                predecessor,
                reason,
                detail,
                brief,
                chrono::Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    /// Create the fresh session a goal moves to: same workspace, worktree, permission mode and
    /// model/effort pin as `predecessor`, no messages, linked to it with the brief. The
    /// predecessor's newest turn is marked `handed_off`. Returns the successor's id.
    pub fn create_relay_successor(
        &self,
        predecessor: &str,
        reason: &str,
        detail: &str,
        brief: &str,
    ) -> Result<String> {
        let cwd = self.session_cwd(predecessor)?.unwrap_or_default();
        let mode = self.session_mode(predecessor)?;
        let successor = self.create_session(&cwd, &mode)?;
        if let Some(worktree) = self.session_worktree(predecessor)? {
            self.set_session_worktree(&successor, &worktree)?;
        }
        if let Some(pin) = self.session_pinned_model(predecessor)? {
            self.set_session_pinned_model(&successor, Some(&pin))?;
        }
        if let Some(effort) = self.session_pinned_effort(predecessor)? {
            self.set_session_pinned_effort(&successor, Some(&effort))?;
        }
        if let Some(title) = self.session_title(predecessor)?.filter(|t| !t.is_empty()) {
            let n = self.relay_predecessors(predecessor)?.len() + 2;
            self.set_session_title(&successor, &format!("{title} (worker {n})"))?;
        }
        self.hand_over_custody(predecessor, &successor, reason, detail, brief)?;
        self.mark_last_turn_handed_off(predecessor)?;
        Ok(successor)
    }

    /// The hand-off row and everything the predecessor owned move in one transaction: the
    /// successor holds the worktree and the fleet slot, the predecessor lets go of both and is
    /// done. Like a Move, the predecessor keeps no worktree record, so nothing it does later
    /// (a resume, a ship, a discard) can reach the directory the successor now works in, and a
    /// daemon that dies at any point restarts with exactly one live holder of the goal.
    fn hand_over_custody(
        &self,
        predecessor: &str,
        successor: &str,
        reason: &str,
        detail: &str,
        brief: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO session_relay \
             (successor_id, predecessor_id, reason, detail, brief, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![successor, predecessor, reason, detail, brief, now],
        )?;
        tx.execute(
            "UPDATE session SET worktree_path = NULL, daemon_live = 0, \
             done_at = COALESCE(done_at, ?2) WHERE id = ?1",
            rusqlite::params![predecessor, now],
        )?;
        tx.execute(
            "UPDATE session SET daemon_live = 1 WHERE id = ?1",
            [successor],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The session that holds `session_id`'s goal now: the end of its hand-off chain, or the
    /// session itself when it never handed the goal on.
    pub fn relay_tail(&self, session_id: &str) -> Result<String> {
        let mut seen: Vec<String> = Vec::new();
        let mut at = session_id.to_string();
        while seen.len() < MAX_CHAIN {
            let Some(link) = self.relay_successor(&at)? else {
                break;
            };
            if link.successor_id == session_id || seen.contains(&link.successor_id) {
                break;
            }
            seen.push(link.successor_id.clone());
            at = link.successor_id;
        }
        Ok(at)
    }

    /// The hand-off that created `session_id`, if it is a successor.
    pub fn relay_predecessor(&self, session_id: &str) -> Result<Option<RelayLink>> {
        Ok(self
            .lock()?
            .query_row(
                &format!("SELECT {COLS} FROM session_relay WHERE successor_id = ?1"),
                [session_id],
                link_of,
            )
            .optional()?)
    }

    /// The hand-off `session_id` made, if it handed its goal on.
    pub fn relay_successor(&self, session_id: &str) -> Result<Option<RelayLink>> {
        Ok(self
            .lock()?
            .query_row(
                &format!(
                    "SELECT {COLS} FROM session_relay WHERE predecessor_id = ?1 \
                     ORDER BY created_at DESC LIMIT 1"
                ),
                [session_id],
                link_of,
            )
            .optional()?)
    }

    /// The brief a successor was started from.
    pub fn relay_brief(&self, successor_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT brief FROM session_relay WHERE successor_id = ?1",
                [successor_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The sessions `session_id` descends from, nearest first. Empty for a session nobody
    /// handed a goal to.
    pub fn relay_predecessors(&self, session_id: &str) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let mut at = session_id.to_string();
        while out.len() < MAX_CHAIN {
            let Some(link) = self.relay_predecessor(&at)? else {
                break;
            };
            if link.predecessor_id == session_id || out.contains(&link.predecessor_id) {
                break;
            }
            out.push(link.predecessor_id.clone());
            at = link.predecessor_id;
        }
        Ok(out)
    }

    /// Every hand-off of the chain `session_id` belongs to, oldest first: walk up to the first
    /// session, then down through the successors.
    pub fn relay_chain(&self, session_id: &str) -> Result<Vec<RelayLink>> {
        let mut root = session_id.to_string();
        for pred in self.relay_predecessors(session_id)? {
            root = pred;
        }
        let mut links: Vec<RelayLink> = Vec::new();
        let mut at = root;
        while links.len() < MAX_CHAIN {
            let Some(link) = self.relay_successor(&at)? else {
                break;
            };
            if links.iter().any(|l| l.successor_id == link.successor_id) {
                break;
            }
            at = link.successor_id.clone();
            links.push(link);
        }
        Ok(links)
    }

    /// How many times this session has been compacted (0 when never).
    pub fn compaction_count(&self, session_id: &str) -> Result<u32> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT compactions FROM session_compaction WHERE session_id = ?1",
                [session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
            .max(0) as u32)
    }

    /// The session's newest closed turn ends as `handed_off`: its goal moved to a fresh worker.
    /// The reason it had stays in the detail as `was`. False when the session has no closed turn.
    pub fn mark_last_turn_handed_off(&self, session_id: &str) -> Result<bool> {
        let Some(row) = self.last_closed_turn(session_id)? else {
            return Ok(false);
        };
        if row.end_reason.as_deref() == Some("handed_off") {
            return Ok(true);
        }
        let mut detail = row
            .detail_json
            .as_deref()
            .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        detail["was"] = row.end_reason.clone().unwrap_or_default().into();
        let changed = self.lock()?.execute(
            "UPDATE turn SET end_reason = 'handed_off', detail_json = ?2 WHERE id = ?1",
            rusqlite::params![row.id, detail.to_string()],
        )?;
        Ok(changed > 0)
    }

    /// Whether `recall` has anything to offer this session: it folded history, or it descends
    /// from a session that handed it a goal.
    pub fn has_recallable_history(&self, session_id: &str) -> bool {
        self.has_folded_history(session_id).unwrap_or(false)
            || !self
                .relay_predecessors(session_id)
                .unwrap_or_default()
                .is_empty()
    }

    /// Search what the predecessors of `session_id` said, nearest predecessor first, at most
    /// `limit` hits in all. Never reaches a session that is not an ancestor.
    pub fn search_predecessors(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, FoldedHit)>> {
        let mut out = Vec::new();
        for pred in self.relay_predecessors(session_id)? {
            if out.len() >= limit {
                break;
            }
            for hit in self.search_session_history(&pred, query, limit - out.len())? {
                out.push((pred.clone(), hit));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::Role;

    fn three() -> (Store, String, String, String) {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_session("/w", "default").unwrap();
        let b = store.create_session("/w", "default").unwrap();
        let c = store.create_session("/w", "default").unwrap();
        store
            .record_relay(&a, &b, "wedged", "same failing set", "brief b")
            .unwrap();
        store
            .record_relay(&b, &c, "worn", "4 compactions", "brief c")
            .unwrap();
        (store, a, b, c)
    }

    #[test]
    fn the_chain_is_walked_from_any_member() {
        let (store, a, b, c) = three();
        assert_eq!(
            store.relay_predecessors(&c).unwrap(),
            vec![b.clone(), a.clone()]
        );
        assert!(store.relay_predecessors(&a).unwrap().is_empty());
        for member in [&a, &b, &c] {
            let chain = store.relay_chain(member).unwrap();
            let pairs: Vec<_> = chain
                .iter()
                .map(|l| {
                    (
                        l.predecessor_id.as_str(),
                        l.successor_id.as_str(),
                        l.reason.as_str(),
                    )
                })
                .collect();
            assert_eq!(
                pairs,
                vec![
                    (a.as_str(), b.as_str(), "wedged"),
                    (b.as_str(), c.as_str(), "worn")
                ]
            );
        }
        assert_eq!(store.relay_brief(&c).unwrap().as_deref(), Some("brief c"));
    }

    #[test]
    fn a_successor_inherits_the_workspace_pin_and_closes_the_predecessor_turn() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_session("/repo/wt", "bypass").unwrap();
        store.set_session_worktree(&a, "/repo/wt").unwrap();
        store
            .set_session_pinned_model(&a, Some("opencode::muse"))
            .unwrap();
        store.set_session_title(&a, "ship parser").unwrap();
        let turn = store.open_turn(&a, 0).unwrap();
        store.close_turn(turn, "final_answer", None).unwrap();
        let b = store
            .create_relay_successor(&a, "wedged", "same set", "BRIEF")
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(store.session_cwd(&b).unwrap().as_deref(), Some("/repo/wt"));
        assert_eq!(store.session_mode(&b).unwrap(), "bypass");
        assert_eq!(
            store.session_worktree(&b).unwrap().as_deref(),
            Some("/repo/wt")
        );
        assert_eq!(
            store.session_pinned_model(&b).unwrap().as_deref(),
            Some("opencode::muse")
        );
        assert_eq!(
            store.session_title(&b).unwrap().as_deref(),
            Some("ship parser (worker 2)")
        );
        assert_eq!(store.relay_brief(&b).unwrap().as_deref(), Some("BRIEF"));
        assert_eq!(
            store
                .last_closed_turn(&a)
                .unwrap()
                .unwrap()
                .end_reason
                .as_deref(),
            Some("handed_off")
        );
        assert!(store.load_messages(&b).unwrap().is_empty());
    }

    #[test]
    fn custody_moves_with_the_goal_in_one_step() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_session("/repo/wt", "bypass").unwrap();
        store.set_session_worktree(&a, "/repo/wt").unwrap();
        store.set_session_daemon_live(&a, true).unwrap();
        let b = store
            .create_relay_successor(&a, "wedged", "same set", "BRIEF")
            .unwrap();
        let c = store
            .create_relay_successor(&b, "worn", "", "BRIEF 2")
            .unwrap();
        assert_eq!(store.session_worktree(&a).unwrap(), None);
        assert_eq!(store.session_worktree(&b).unwrap(), None);
        assert_eq!(
            store.session_worktree(&c).unwrap().as_deref(),
            Some("/repo/wt")
        );
        assert!(!store.session_daemon_live(&a).unwrap());
        assert!(!store.session_daemon_live(&b).unwrap());
        assert!(store.session_daemon_live(&c).unwrap());
        assert_eq!(store.relay_tail(&a).unwrap(), c);
        assert_eq!(store.relay_tail(&c).unwrap(), c);
        let done = store.session_lifecycle(std::slice::from_ref(&a)).unwrap();
        assert!(done[&a].1.is_some(), "the predecessor is done");
    }

    #[test]
    fn a_hand_off_is_recorded_once_per_successor() {
        let (store, a, b, _) = three();
        store.record_relay(&a, &b, "stalled", "x", "other").unwrap();
        assert_eq!(
            store.relay_predecessor(&b).unwrap().unwrap().reason,
            "wedged"
        );
    }

    #[test]
    fn a_predecessor_cycle_ends_the_walk() {
        let (store, a, _b, c) = three();
        store.record_relay(&c, &a, "worn", "", "").unwrap();
        assert!(store.relay_predecessors(&c).unwrap().len() <= 3);
        let _ = store.relay_chain(&a).unwrap();
    }

    #[test]
    fn search_reaches_ancestors_and_nobody_else() {
        let (store, a, b, c) = three();
        let stranger = store.create_session("/w", "default").unwrap();
        store
            .add_message(
                &a,
                0,
                Role::Assistant,
                "decided on exponential backoff",
                None,
            )
            .unwrap();
        store
            .add_message(&b, 0, Role::Assistant, "chose the sqlite journal", None)
            .unwrap();
        store
            .add_message(
                &stranger,
                0,
                Role::Assistant,
                "exponential backoff elsewhere",
                None,
            )
            .unwrap();
        let hits = store.search_predecessors(&c, "backoff", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, a);
        let hits = store.search_predecessors(&c, "journal", 10).unwrap();
        assert_eq!(hits[0].0, b);
        assert!(store
            .search_predecessors(&stranger, "backoff", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn compactions_are_counted_and_the_last_turn_can_be_handed_off() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/w", "default").unwrap();
        for i in 0..8i64 {
            store
                .add_message(&sid, i, Role::User, &format!("m{i}"), None)
                .unwrap();
        }
        assert_eq!(store.compaction_count(&sid).unwrap(), 0);
        store.compact_session_store(&sid, "one", 3).unwrap();
        store.compact_session_store(&sid, "two", 2).unwrap();
        assert_eq!(store.compaction_count(&sid).unwrap(), 2);

        assert!(!store.mark_last_turn_handed_off(&sid).unwrap());
        let turn = store.open_turn(&sid, 0).unwrap();
        store
            .close_turn(turn, "stalled", Some("{\"open_tasks\":2}"))
            .unwrap();
        assert!(store.mark_last_turn_handed_off(&sid).unwrap());
        let row = store.last_closed_turn(&sid).unwrap().unwrap();
        assert_eq!(row.end_reason.as_deref(), Some("handed_off"));
        assert!(row.detail_json.unwrap().contains("\"was\":\"stalled\""));
    }
}
