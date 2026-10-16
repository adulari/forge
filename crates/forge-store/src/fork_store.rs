//! Counterfactual session forks and ancestry projection.

use super::*;

impl Store {
    // --- forge fork / forge tree: counterfactual session branching ---

    /// Branch a session at a turn boundary: create a new top-level session (same cwd + mode)
    /// carrying a copy of `src`'s *active* messages with `seq < at_seq`, linked back via
    /// `forked_from`/`forked_at_seq`. The re-asked prompt itself is NOT copied — the fork's next
    /// turn supplies it (possibly against a different model), which is the whole point.
    pub fn fork_session(&self, src: &str, at_seq: i64) -> Result<String> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (cwd, mode): (String, String) = tx.query_row(
            "SELECT cwd, permission_mode FROM session WHERE id = ?1",
            [src],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let new_id = forge_types::new_id();
        tx.execute(
            "INSERT INTO session (id, cwd, permission_mode, total_cost_usd, forked_from, forked_at_seq) \
             VALUES (?1, ?2, ?3, 0, ?4, ?5)",
            (&new_id, &cwd, &mode, src, at_seq),
        )?;
        // Compaction deactivates old rows and keeps their gist in `session_compaction`. A fork at
        // or after the compacted region carries that summary with the active tail; a fork INSIDE it
        // (the summary would describe turns after the fork point) takes the verbatim old rows.
        let max_compacted: Option<i64> = tx.query_row(
            "SELECT MAX(seq) FROM message WHERE session_id = ?1 AND compacted = 1",
            [src],
            |r| r.get(0),
        )?;
        let inside_compacted = max_compacted.is_some_and(|m| at_seq <= m);
        if !inside_compacted {
            tx.execute(
                "INSERT INTO session_compaction (session_id, summary) \
                 SELECT ?2, summary FROM session_compaction WHERE session_id = ?1",
                (src, &new_id),
            )?;
        }
        let row_filter = if inside_compacted {
            "(active = 1 OR compacted = 1)"
        } else {
            "active = 1"
        };
        {
            let mut read = tx.prepare(&format!(
                "SELECT seq, role, content, model, tool_calls_json, tool_call_id, visibility, nudge \
                 FROM message WHERE session_id = ?1 AND {row_filter} AND seq < ?2 ORDER BY seq"
            ))?;
            let mut write = tx.prepare(
                "INSERT INTO message (id, session_id, seq, role, content, model, tool_calls_json, tool_call_id, visibility, nudge) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            let rows = read.query_map((src, at_seq), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })?;
            for row in rows {
                let (seq, role, content, model, tcj, tcid, vis, nudge) = row?;
                let message_id = forge_types::new_id();
                write.execute((
                    &message_id,
                    &new_id,
                    seq,
                    &role,
                    &content,
                    &model,
                    &tcj,
                    &tcid,
                    &vis,
                    nudge,
                ))?;
                let tool_calls = tcj
                    .as_deref()
                    .map(|json| {
                        serde_json::from_str::<serde_json::Value>(json).map_err(|error| {
                            StoreError::Json(format!(
                                "fork message seq {seq} has malformed tool-call metadata: {error}"
                            ))
                        })
                    })
                    .transpose()?
                    .unwrap_or_else(|| serde_json::json!([]));
                let payload = sync_json(serde_json::json!({
                    "id": message_id,
                    "session_id": new_id,
                    "seq": seq,
                    "role": role,
                    "content": content,
                    "model": model,
                    "tool_calls": tool_calls,
                    "tool_call_id": tcid,
                    "visibility": vis,
                }))?;
                append_sync_revision(
                    &tx,
                    "message",
                    &message_id,
                    SyncJournalOperation::Upsert,
                    &payload,
                )?;
            }
        }
        append_session_snapshot(&tx, &new_id)?;
        tx.commit()?;
        Ok(new_id)
    }

    /// `forge tree` shows conversations, not worker fan-out.
    pub fn fork_nodes(&self) -> Result<Vec<ForkNode>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, forked_from, forked_at_seq, created_at FROM session \
             WHERE parent_session_id IS NULL ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ForkNode {
                id: r.get(0)?,
                title: r.get(1)?,
                forked_from: r.get(2)?,
                forked_at_seq: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod compacted_fork_tests {
    use super::*;
    use forge_types::Role;

    fn compacted_session(store: &Store) -> String {
        let id = store.create_session("/tmp", "Default").unwrap();
        for (seq, role) in [
            (0, Role::User),
            (1, Role::Assistant),
            (2, Role::User),
            (3, Role::Assistant),
            (4, Role::User),
            (5, Role::Assistant),
        ] {
            store
                .add_message(&id, seq, role, &format!("m{seq}"), None)
                .unwrap();
        }
        store.compact_session_store(&id, "the summary", 2).unwrap();
        id
    }

    #[test]
    fn fork_after_compaction_keeps_the_summary() {
        let store = Store::open_in_memory().unwrap();
        let src = compacted_session(&store);
        let fork = store.fork_session(&src, 5).unwrap();
        let loaded = store.load_messages(&fork).unwrap();
        assert!(loaded.iter().any(|m| m.content.contains("the summary")));
    }

    #[test]
    fn fork_inside_the_compacted_region_takes_verbatim_rows_without_the_summary() {
        let store = Store::open_in_memory().unwrap();
        let src = compacted_session(&store);
        let fork = store.fork_session(&src, 3).unwrap();
        let loaded = store.load_messages(&fork).unwrap();
        assert!(!loaded.iter().any(|m| m.content.contains("the summary")));
        assert!(loaded.iter().any(|m| m.content == "m2"));
    }
}
