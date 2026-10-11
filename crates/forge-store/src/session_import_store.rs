//! Bulk import of an externally recorded conversation (a Claude Code transcript) as a new session,
//! with the older part kept in history but out of the model's context.
//!
//! The compaction representation is the one `/compact` already uses: soft-deleted rows
//! (`active = 0, compacted = 1`) plus a `session_compaction` summary that `load_messages` prepends.
//! `/uncompact` therefore works on an imported session exactly as on a native one.

use super::*;

/// Where and how an imported session starts out.
pub struct ImportedSessionMeta<'a> {
    pub cwd: &'a str,
    pub mode: &'a str,
    pub title: Option<&'a str>,
    pub pinned_model: Option<&'a str>,
}

/// One message to append to an imported session.
pub struct ImportedRow {
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    /// `false` keeps the row in history/replay but out of the model context (compacted away).
    pub active: bool,
    pub created_at: Option<i64>,
}

/// Streaming writer handed to [`Store::import_session_with`]; every push lands in the one open
/// transaction, so a failed import leaves nothing behind.
pub struct ImportSink<'t> {
    tx: &'t rusqlite::Transaction<'t>,
    session_id: &'t str,
    seq: i64,
    journal: bool,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
}

impl ImportSink<'_> {
    pub fn push(&mut self, row: ImportedRow) -> Result<()> {
        let id = forge_types::new_id();
        let tool_calls_json = if row.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&row.tool_calls).unwrap_or_default())
        };
        let ts = row.created_at.unwrap_or_else(|| {
            self.last_ts
                .unwrap_or_else(|| chrono::Utc::now().timestamp())
        });
        self.tx
            .prepare_cached(
                "INSERT INTO message (id, session_id, seq, role, content, model, tool_calls_json, \
             tool_call_id, visibility, active, compacted, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'llm', ?9, ?10, ?11)",
            )?
            .execute(rusqlite::params![
                id,
                self.session_id,
                self.seq,
                row.role.as_str(),
                row.content,
                row.model,
                tool_calls_json,
                row.tool_call_id,
                row.active,
                !row.active,
                ts,
            ])?;
        if self.journal {
            let payload = sync_json(serde_json::json!({
                "id": id,
                "session_id": self.session_id,
                "seq": self.seq,
                "role": row.role.as_str(),
                "content": row.content,
                "model": row.model,
                "tool_calls": row.tool_calls,
                "tool_call_id": row.tool_call_id,
                "visibility": "llm",
                "nudge": false,
            }))?;
            append_sync_revision(
                self.tx,
                "message",
                &id,
                SyncJournalOperation::Upsert,
                &payload,
            )?;
        }
        self.first_ts.get_or_insert(ts);
        self.last_ts = Some(ts);
        self.seq += 1;
        Ok(())
    }

    /// Messages pushed so far.
    pub fn count(&self) -> i64 {
        self.seq
    }
}

impl Store {
    /// Create a session and fill it through `fill` in one transaction. When `summary` is returned
    /// it is stored as the session's compaction summary (see the module docs). Returns the new
    /// session id. Nothing is written if `fill` fails.
    pub fn import_session_with<F>(&self, meta: &ImportedSessionMeta<'_>, fill: F) -> Result<String>
    where
        F: FnOnce(&mut ImportSink<'_>) -> Result<Option<String>>,
    {
        let id = forge_types::new_id();
        {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO session (id, title, cwd, permission_mode, total_cost_usd, pinned_model) \
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                rusqlite::params![id, meta.title, meta.cwd, meta.mode, meta.pinned_model],
            )?;
            let journal = tx
                .query_row(
                    "SELECT enabled FROM anywhere_sync_state WHERE singleton = 1",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false);
            let mut sink = ImportSink {
                tx: &tx,
                session_id: &id,
                seq: 0,
                journal,
                first_ts: None,
                last_ts: None,
            };
            let summary = fill(&mut sink)?;
            let (first, last) = (sink.first_ts, sink.last_ts);
            if let (Some(first), Some(last)) = (first, last) {
                tx.execute(
                    "UPDATE session SET created_at = ?2, updated_at = ?3 WHERE id = ?1",
                    (&id, first, last),
                )?;
            }
            if let Some(summary) = summary.filter(|s| !s.trim().is_empty()) {
                tx.execute(
                    "INSERT INTO session_compaction (session_id, summary) VALUES (?1, ?2)",
                    (&id, &summary),
                )?;
                let payload = sync_json(serde_json::json!({
                    "session_id": id,
                    "summary": summary,
                    "keep_count": 0,
                }))?;
                append_sync_revision(
                    &tx,
                    "compaction",
                    &id,
                    SyncJournalOperation::Upsert,
                    &payload,
                )?;
            }
            append_session_snapshot(&tx, &id)?;
            tx.commit()?;
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: Role, content: &str, active: bool) -> ImportedRow {
        ImportedRow {
            role,
            content: content.into(),
            model: None,
            tool_calls: vec![],
            tool_call_id: None,
            active,
            created_at: Some(1_700_000_000),
        }
    }

    #[test]
    fn a_failed_fill_leaves_no_session_behind() {
        let store = Store::open_in_memory().unwrap();
        let meta = ImportedSessionMeta {
            cwd: "/w",
            mode: "default",
            title: None,
            pinned_model: None,
        };
        let err = store.import_session_with(&meta, |sink| {
            sink.push(row(Role::User, "hi", true))?;
            Err(StoreError::InvalidValue("boom".into()))
        });
        assert!(err.is_err());
        assert!(store.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn inactive_rows_are_compacted_history_and_the_summary_leads_the_context() {
        let store = Store::open_in_memory().unwrap();
        let meta = ImportedSessionMeta {
            cwd: "/w",
            mode: "default",
            title: Some("t"),
            pinned_model: Some("claude-cli::haiku"),
        };
        let id = store
            .import_session_with(&meta, |sink| {
                sink.push(row(Role::User, "old", false))?;
                sink.push(row(Role::User, "new", true))?;
                Ok(Some("recap".into()))
            })
            .unwrap();
        let live = store.load_messages(&id).unwrap();
        assert_eq!(live.len(), 2);
        assert!(live[0].content.ends_with("recap"));
        assert_eq!(live[1].content, "new");
        assert_eq!(store.load_all_messages(&id).unwrap().len(), 2);
        assert_eq!(
            store.session_pinned_model(&id).unwrap().as_deref(),
            Some("claude-cli::haiku")
        );
    }
}
