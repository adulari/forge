//! Transcript reconstruction, history pagination, compaction, and replay.

use super::*;

/// A row's persisted `tool_calls_json`, or an empty list WITH a diagnostic when the JSON is
/// damaged. The surrounding message stays readable either way — losing one row's call list must
/// not make a whole transcript unloadable — but corruption is reported instead of silently
/// presenting the turn as call-free.
fn parse_tool_calls_with_diagnostic(
    session_id: &str,
    seq: Option<i64>,
    raw: Option<&str>,
) -> Vec<ToolCall> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    match serde_json::from_str(raw) {
        Ok(calls) => calls,
        Err(error) => {
            tracing::warn!(
                session_id,
                seq = ?seq,
                %error,
                "store: malformed persisted tool_calls_json; showing the message without its calls"
            );
            Vec::new()
        }
    }
}

fn parse_role_with_diagnostic(session_id: &str, seq: Option<i64>, raw: &str) -> Role {
    match Role::parse(raw) {
        Some(role) => role,
        None => {
            tracing::warn!(
                session_id,
                seq = ?seq,
                role = raw,
                "store: unknown persisted message role; replaying as user"
            );
            Role::User
        }
    }
}

impl Store {
    /// All *active* messages of a session, in turn order (by seq). Soft-deleted rows (those a
    /// `/undo` rewound past) are excluded — they remain in the table for audit/redo. If a
    /// compaction summary exists (written by [`compact_session_store`](Self::compact_session_store)),
    /// a synthetic System message is prepended so a resumed session sees the compacted view.
    pub fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>> {
        let conn = self.lock()?;
        // Read compaction summary before the message prepare (both are &self borrows; ordering
        // keeps the non-mut borrow from query_row from conflicting with the stmt lifetime).
        let summary: Option<String> = conn
            .query_row(
                "SELECT summary FROM session_compaction WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        let mut stmt = conn.prepare_cached(
            "SELECT role, content, model, tool_calls_json, tool_call_id, visibility
             FROM message WHERE session_id = ?1 AND active = 1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let role: String = row.get(0)?;
            let tool_calls_json: Option<String> = row.get(3)?;
            let tool_calls =
                parse_tool_calls_with_diagnostic(session_id, None, tool_calls_json.as_deref());
            let visibility: String = row.get(5)?;
            Ok(StoredMessage {
                role: parse_role_with_diagnostic(session_id, None, &role),
                content: row.get(1)?,
                model: row.get(2)?,
                tool_calls,
                tool_call_id: row.get(4)?,
                visibility: Visibility::parse(&visibility),
            })
        })?;
        let mut msgs = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        if let Some(s) = summary {
            msgs.insert(
                0,
                StoredMessage {
                    role: Role::System,
                    content: format!(
                        "[Earlier conversation summarized to save context]\n{}",
                        s.trim()
                    ),
                    model: None,
                    tool_calls: vec![],
                    tool_call_id: None,
                    visibility: Visibility::Llm,
                },
            );
        }
        Ok(msgs)
    }

    /// ALL messages of a session in turn order, INCLUDING soft-deleted rows (compacted-away or
    /// `/undo`-rewound) and WITHOUT prepending the summary marker — the genuine, untouched
    /// conversation. The model only ever sees the compacted view ([`load_messages`](Self::load_messages)),
    /// but this lets the USER still read the FULL original history in scrollback after a resume.
    pub fn load_all_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT role, content, model, tool_calls_json, tool_call_id, visibility
             FROM message WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let role: String = row.get(0)?;
            let tool_calls_json: Option<String> = row.get(3)?;
            let tool_calls =
                parse_tool_calls_with_diagnostic(session_id, None, tool_calls_json.as_deref());
            let visibility: String = row.get(5)?;
            Ok(StoredMessage {
                role: parse_role_with_diagnostic(session_id, None, &role),
                content: row.get(1)?,
                model: row.get(2)?,
                tool_calls,
                tool_call_id: row.get(4)?,
                visibility: Visibility::parse(&visibility),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// One page of a session's user-facing transcript, NEWEST first — the remote-control
    /// scrollback pagination seam (docs/features/remote-control.md). Returns user + assistant
    /// turns plus `visibility='ui'` notes (they are part of the visible conversation); tool
    /// results, tool-call carrier rows (empty content), and system prompts are harness plumbing
    /// and excluded. Soft-deleted (`active=0`) rows are INCLUDED, like
    /// [`load_all_messages`](Self::load_all_messages) — this is the user's history, not the
    /// model's context. `before_seq` restricts to rows with `seq < before_seq` (pass `None` for
    /// the newest page); `limit` caps the page size.
    pub fn load_history_page(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<HistoryRow>> {
        self.load_history_page_with(session_id, before_seq, limit, false)
    }

    /// [`load_history_page`](Self::load_history_page) with the tool rows opted in.
    ///
    /// `include_tools = false` is byte-for-byte the historical page (the filter below collapses to
    /// the original predicate), so every existing caller and the legacy PWA see no change.
    ///
    /// `include_tools = true` additionally returns the persisted `role='tool'` rows — the tool
    /// RESULT rows written by `Session::invoke_tool` — with [`HistoryRow::tool_name`] resolved.
    /// Note what that is and isn't: the tool row itself stores only the result text and its
    /// `tool_call_id`; the NAME lives in the assistant carrier's `tool_calls_json`, so it is
    /// recovered from the nearest preceding row that has one and left `None` when that carrier is
    /// gone or does not contain the id. `llm_only` tool rows stay excluded — like their
    /// user/assistant siblings they are provider-continuity plumbing, not conversation.
    ///
    /// It also SURFACES the tool CALLS, which are persisted but were previously unreachable: a
    /// carrier row's `content` is empty, so the `content != ''` filter dropped it and the
    /// `[{id, name, args}]` it carries went with it. With tools opted in, each carrier is expanded
    /// into one row per declared call ([`ToolPhase::Call`], content = a capped args summary), at
    /// the CARRIER's `seq`/`created_at` — so every call sorts before the result row that answers
    /// it (results always carry a later seq), and a call whose result never arrived (an
    /// interrupted turn) still shows up instead of vanishing. A carrier that also wrote prose
    /// keeps its own assistant row, ordered before its calls. Nothing here is reconstructed: the
    /// carrier row and its JSON are real persisted data.
    pub fn load_history_page_with(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: usize,
        include_tools: bool,
    ) -> Result<Vec<HistoryRow>> {
        let conn = self.lock()?;
        // The carrier lookup sits behind `CASE WHEN ?4 = 1 AND m.role = 'tool'` rather than in the
        // projection unconditionally: SQLite short-circuits the CASE, so the default page runs the
        // same work it always did and pays nothing for a subquery it never needs. Same for the
        // row's OWN `tool_calls_json` (the carrier expansion below) and for the widened content
        // filter, which collapses back to `m.content != ''` when tools aren't asked for.
        let mut stmt = conn.prepare(
            "SELECT m.seq, m.role, m.content, m.model, m.created_at, m.visibility,
                    m.tool_call_id,
                    CASE WHEN ?4 = 1 AND m.role = 'tool' THEN (
                        SELECT c.tool_calls_json FROM message c
                         WHERE c.session_id = m.session_id
                           AND c.seq < m.seq
                           AND c.tool_calls_json IS NOT NULL
                           AND EXISTS (
                               SELECT 1 FROM json_each(c.tool_calls_json) AS call
                               WHERE json_extract(call.value, '$.id') = m.tool_call_id
                           )
                         ORDER BY c.seq DESC LIMIT 1
                    ) END,
                    CASE WHEN ?4 = 1 THEN m.tool_calls_json END
             FROM message m
             WHERE m.session_id = ?1
               AND (?2 IS NULL OR m.seq < ?2)
               AND (((m.role IN ('user', 'assistant') AND m.visibility != 'llm_only') OR m.visibility = 'ui')
                    OR (?4 = 1 AND m.role = 'tool' AND m.visibility != 'llm_only'))
               AND (m.content != ''
                    OR (?4 = 1 AND m.role = 'assistant' AND m.visibility != 'llm_only'
                        AND m.tool_calls_json IS NOT NULL))
             ORDER BY m.seq DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![
                session_id,
                before_seq,
                limit as i64,
                i64::from(include_tools)
            ],
            |row| {
                let seq: i64 = row.get(0)?;
                let role: String = row.get(1)?;
                let visibility: String = row.get(5)?;
                let tool_call_id: Option<String> = row.get(6)?;
                let carrier_json: Option<String> = row.get(7)?;
                let own_calls_json: Option<String> = row.get(8)?;
                let role = parse_role_with_diagnostic(session_id, Some(seq), &role);
                Ok((
                    HistoryRow {
                        seq,
                        role,
                        content: row.get(2)?,
                        model: row.get(3)?,
                        created_at: row.get(4)?,
                        visibility: Visibility::parse(&visibility),
                        tool_name: carrier_json.as_deref().and_then(|carrier| {
                            tool_name_from_carrier(carrier, tool_call_id.as_deref())
                        }),
                        tool_phase: (role == Role::Tool).then_some(ToolPhase::Result),
                    },
                    own_calls_json,
                ))
            },
        )?;
        // `limit` bounds the DB rows read, not the rows returned: one carrier expands into as many
        // call rows as it declares. Pagination is unaffected — every returned row carries a REAL
        // `seq`, so `before_seq` from the oldest of them opens the next window with no gap and no
        // repeat.
        let mut out = Vec::new();
        for row in rows {
            let (row, own_calls_json) = row?;
            // Newest-first order, so a carrier's calls are pushed in reverse declaration order and
            // its own prose last: reversed by the client, that reads prose → call 1 → call 2.
            if let Some(json) = own_calls_json.as_deref() {
                for call in parse_tool_calls(json).into_iter().rev() {
                    out.push(HistoryRow {
                        seq: row.seq,
                        // A call is tool activity, like the result it precedes — the carrier's
                        // `assistant` role belongs to its prose row, not to the calls.
                        role: Role::Tool,
                        content: tool_call_args_summary(&call.args),
                        // Only a provider round-trip has a model; the persisted result rows this
                        // sits next to carry none either.
                        model: None,
                        created_at: row.created_at,
                        visibility: row.visibility,
                        tool_name: Some(call.name),
                        tool_phase: Some(ToolPhase::Call),
                    });
                }
            }
            if !row.content.is_empty() {
                out.push(row);
            }
        }
        Ok(out)
    }

    /// When the session's FIRST user-facing row was written — the zero point a transcript replay
    /// measures its offsets from (raw `created_at` epochs give a scrubber nothing to anchor on).
    /// Same row filter as [`load_history_page`](Self::load_history_page), so the epoch is the
    /// first row a client can actually page to. `None` for a session with no visible rows yet.
    pub fn history_epoch(&self, session_id: &str) -> Result<Option<i64>> {
        self.history_epoch_with(session_id, false)
    }

    /// [`history_epoch`](Self::history_epoch) over the row set
    /// [`load_history_page_with`](Self::load_history_page_with) returns for the same `include_tools`.
    ///
    /// The two MUST be asked the same question: a session whose first row is a tool result has one
    /// epoch with tools included and a later one without, and mixing them would slide every
    /// `elapsed_ms` on the wire — the scrubber's zero point would jump the moment a client toggled
    /// tool rows on.
    pub fn history_epoch_with(&self, session_id: &str, include_tools: bool) -> Result<Option<i64>> {
        let conn = self.lock()?;
        // Must mirror `load_history_page_with`'s filter exactly, INCLUDING the carrier rows it
        // expands into call rows: a carrier can be the oldest row of an include_tools page (a turn
        // that opened with a tool call), and measuring against a later row would slide every
        // `elapsed_ms` on the wire.
        let epoch = conn.query_row(
            "SELECT MIN(created_at)
             FROM message
             WHERE session_id = ?1
               AND (((role IN ('user', 'assistant') AND visibility != 'llm_only') OR visibility = 'ui')
                    OR (?2 = 1 AND role = 'tool' AND visibility != 'llm_only'))
               AND (content != ''
                    OR (?2 = 1 AND role = 'assistant' AND visibility != 'llm_only'
                        AND tool_calls_json IS NOT NULL))",
            rusqlite::params![session_id, i64::from(include_tools)],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        Ok(epoch)
    }

    /// Whether this session has a stored compaction summary (was compacted at least once) — the
    /// signal for offering "compact first vs continue uncompacted" when resuming it.
    pub fn session_has_compaction(&self, session_id: &str) -> Result<bool> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM session_compaction WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// Persist the compacted view of a session: soft-delete the oldest active messages (keeping
    /// the last `keep_count`) and upsert `summary` into `session_compaction`. On the next resume,
    /// [`load_messages`](Self::load_messages) prepends a System message with the summary so the
    /// session rehydrates the compacted state instead of the full transcript.
    pub fn compact_session_store(
        &self,
        session_id: &str,
        summary: &str,
        keep_count: usize,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if keep_count == 0 {
            tx.execute(
                "UPDATE message SET active = 0, compacted = 1 WHERE session_id = ?1 AND active = 1",
                [session_id],
            )?;
        } else {
            // Soft-delete every active message whose seq is below the (keep_count)-th newest.
            // LIMIT 1 OFFSET (keep_count-1) on DESC order gives the oldest row to KEEP.
            tx.execute(
                "UPDATE message SET active = 0, compacted = 1
                 WHERE session_id = ?1 AND active = 1
                 AND seq < (
                     SELECT seq FROM message
                     WHERE session_id = ?1 AND active = 1
                     ORDER BY seq DESC
                     LIMIT 1 OFFSET ?2
                 )",
                (session_id, keep_count as i64 - 1),
            )?;
        }
        tx.execute(
            "INSERT INTO session_compaction (session_id, summary) VALUES (?1, ?2)
             ON CONFLICT(session_id) DO UPDATE SET
               summary = excluded.summary,
               created_at = strftime('%s','now')",
            (session_id, summary),
        )?;
        let payload = sync_json(serde_json::json!({
            "session_id": session_id,
            "summary": summary,
            "keep_count": keep_count,
        }))?;
        append_sync_revision(
            &tx,
            "compaction",
            session_id,
            SyncJournalOperation::Upsert,
            &payload,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Undo a compaction: reactivate the messages THIS compaction soft-deleted (`compacted = 1`)
    /// and drop the stored summary row. Rows `/undo` soft-deleted (`active = 0`, `compacted = 0`)
    /// stay removed — resurrecting them was a bug. Returns `false` (no-op) if the session was never
    /// compacted (no `session_compaction` row).
    pub fn uncompact_session_store(&self, session_id: &str) -> Result<bool> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let had_compaction: bool = tx.query_row(
            "SELECT COUNT(*) FROM session_compaction WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if !had_compaction {
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "UPDATE message SET active = 1, compacted = 0 WHERE session_id = ?1 AND compacted = 1",
            [session_id],
        )?;
        tx.execute(
            "DELETE FROM session_compaction WHERE session_id = ?1",
            [session_id],
        )?;
        append_sync_revision(
            &tx,
            "compaction",
            session_id,
            SyncJournalOperation::Tombstone,
            &[],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Every active message of a session in turn order, each joined to its usage row so a
    /// replay can show the model, token counts, cost, and wall-clock time of each turn
    /// (docs/features/session-replay.md). Unlike [`load_messages`](Self::load_messages) this
    /// is for auditing a finished session, not rebuilding live state.
    pub fn load_replay(&self, session_id: &str) -> Result<Vec<ReplayEntry>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT m.seq, m.role, m.content, m.model, m.created_at, m.tool_calls_json,
                    u.input_tokens, u.output_tokens, u.cost_usd
             FROM message m LEFT JOIN usage u ON u.message_id = m.id
             WHERE m.session_id = ?1 AND m.active = 1 ORDER BY m.seq",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let role: String = row.get(1)?;
            let tool_calls_json: Option<String> = row.get(5)?;
            let seq: i64 = row.get(0)?;
            let tool_calls =
                parse_tool_calls_with_diagnostic(session_id, Some(seq), tool_calls_json.as_deref());
            Ok(ReplayEntry {
                seq,
                role: parse_role_with_diagnostic(session_id, Some(seq), &role),
                content: row.get(2)?,
                model: row.get(3)?,
                created_at: row.get(4)?,
                tool_calls,
                input_tokens: row.get(6)?,
                output_tokens: row.get(7)?,
                cost_usd: row.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_role_with_diagnostic, parse_tool_calls_with_diagnostic, Role, Store};

    #[test]
    fn unknown_role_keeps_the_transcript_row_readable_as_user() {
        assert_eq!(
            parse_role_with_diagnostic("session-1", Some(8), "future-role"),
            forge_types::Role::User
        );
    }

    #[test]
    fn malformed_tool_calls_json_keeps_the_row_readable_with_no_calls() {
        assert!(
            parse_tool_calls_with_diagnostic("session-1", Some(3), Some("{not json")).is_empty()
        );
        assert!(parse_tool_calls_with_diagnostic("session-1", None, None).is_empty());
    }

    /// A damaged `tool_calls_json` row must not make the transcript unloadable — resume and
    /// replay still return every message, with the broken row's call list empty (and a warning
    /// emitted, which this can't observe).
    #[test]
    fn corrupt_tool_calls_row_survives_resume_and_replay() {
        let store = Store::open_in_memory().unwrap();
        let session = store.create_session("/tmp", "default").unwrap();
        store
            .add_message(&session, 0, Role::Assistant, "did a thing", None)
            .unwrap();
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE message SET tool_calls_json = '{not json' WHERE session_id = ?1",
                [&session],
            )
            .unwrap();

        let msgs = store.load_messages(&session).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "did a thing");
        assert!(msgs[0].tool_calls.is_empty());

        let all = store.load_all_messages(&session).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].tool_calls.is_empty());

        let replay = store.load_replay(&session).unwrap();
        assert_eq!(replay.len(), 1);
        assert!(replay[0].tool_calls.is_empty());
    }
}
