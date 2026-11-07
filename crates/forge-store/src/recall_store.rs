//! Search and read access to a session's folded history: the rows a compaction soft-deleted
//! (`active = 0, compacted = 1`) stay in the store, and this is the only path the model has to
//! them (the `recall` tool), beside the owner's history search.
//!
//! The index (`message_fts`, migration 57) is built lazily per session on the first search and
//! topped up on every later one. It never has to be told about a fold, an unfold or a sync apply:
//! message content is immutable, so the index only says which words a row contains, and whether
//! the row currently counts as folded is read from `message.compacted` at query time.

use super::*;

/// SQL condition, over a `message` row aliased `m` (pass `"m."`), that drops the
/// `ui` display copy of a final reply: the turn loop stores each answer twice, once for the model
/// (`llm_only`) and once for the screen (`ui`), so a search would list every answer twice.
fn not_ui_copy(m: &str) -> String {
    format!(
        "NOT ({m}visibility = 'ui' AND EXISTS (SELECT 1 FROM message p
           WHERE p.session_id = {m}session_id AND p.seq = {m}seq - 1
             AND p.visibility = 'llm_only' AND p.content = {m}content))"
    )
}

/// Characters of one row's content that reach the index. Bounds the index of a session full of
/// 32k-char tool results; a word past the cut is still found by reading the range around it.
const INDEX_CONTENT_CHARS: usize = 16_000;
/// Characters of a carrier row's tool-call JSON that reach the index (file paths, commands).
const INDEX_CALLS_CHARS: usize = 2_000;
/// Longest search query term list (extra words are ignored).
const MAX_QUERY_TERMS: usize = 12;
/// Characters of one tool result a read returns before clipping it.
const READ_TOOL_ROW_CHARS: usize = 1_500;
/// Characters of any other row a read returns before clipping it.
const READ_ROW_CHARS: usize = 6_000;
/// Characters of a tool call's arguments shown on a read.
const READ_CALL_ARG_CHARS: usize = 300;

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedHit {
    pub seq: i64,
    pub role: String,
    /// The matching words in context, matches wrapped in `[` `]`.
    pub snippet: String,
    /// Whether a compaction has folded this row out of the model's context. Always true for a
    /// folded-only search; a full-history search also returns live rows.
    pub folded: bool,
}

/// One row returned by [`Store::read_folded`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedRow {
    pub seq: i64,
    pub role: String,
    /// The content, clipped to the per-row limit (`clipped` says whether it was).
    pub content: String,
    pub clipped: bool,
}

/// What [`Store::read_folded`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldedRead {
    pub rows: Vec<FoldedRow>,
    /// The seq of the first folded row in range that did not fit the character budget, when the
    /// read stopped early. A follow-up read starts there.
    pub next_seq: Option<i64>,
}

/// A row a compaction is about to fold, as the episode index needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldPreviewRow {
    pub seq: i64,
    pub role: String,
    /// Harness-injected nudge rather than something the person typed.
    pub nudge: bool,
    /// The start of the content (user rows only; empty for other roles).
    pub head: String,
}

fn clip_chars(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((cut, _)) => (text[..cut].to_string(), true),
        None => (text.to_string(), false),
    }
}

/// An FTS5 `MATCH` expression for free text: every alphanumeric run becomes a quoted term, the
/// longer ones as prefixes. `None` when the text has no searchable word.
fn match_expression(query: &str, join: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(MAX_QUERY_TERMS)
        .map(|t| {
            if t.chars().count() >= 3 {
                format!("\"{t}\"*")
            } else {
                format!("\"{t}\"")
            }
        })
        .collect();
    (!terms.is_empty()).then(|| terms.join(join))
}

impl Store {
    /// Whether a compaction has folded any row of this session. The `recall` tool is offered only
    /// when it has, so a short session pays nothing for it.
    pub fn has_folded_history(&self, session_id: &str) -> Result<bool> {
        let conn = self.lock()?;
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM message WHERE session_id = ?1 AND compacted = 1)",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Index the rows of `session_id` the index has not seen yet. `folded_only` limits it to
    /// folded rows (the model's recall); otherwise live rows are indexed too (history search).
    fn index_session_rows(&self, session_id: &str, folded_only: bool) -> Result<()> {
        let mut conn = self.lock()?;
        let state = if folded_only {
            "m.compacted = 1"
        } else {
            "(m.active = 1 OR m.compacted = 1)"
        };
        let pending: Vec<(i64, String, String)> = {
            let mut stmt = conn.prepare(&format!(
                "SELECT m.rowid, m.id, m.content, m.tool_calls_json, m.seq
                 FROM message m
                 LEFT JOIN message_fts f ON f.rowid = m.rowid AND f.message_id = m.id
                 WHERE m.session_id = ?1 AND {state} AND f.rowid IS NULL
                   AND m.role != 'system'
                   AND (m.content != '' OR COALESCE(m.tool_calls_json, '') NOT IN ('', '[]'))"
            ))?;
            let rows = stmt.query_map([session_id], |row| {
                let content: String = row.get(2)?;
                let calls: Option<String> = row.get(3)?;
                let seq: i64 = row.get(4)?;
                let mut text = clip_chars(&content, INDEX_CONTENT_CHARS).0;
                if let Some(calls) = calls.filter(|c| c != "[]") {
                    // Name and arguments only: the call id is noise, and it is what a hit on a
                    // content-less carrier row would otherwise show as its snippet.
                    let calls: Vec<String> =
                        transcript_read_store::parse_tool_calls_with_diagnostic(
                            session_id,
                            Some(seq),
                            Some(&calls),
                        )
                        .into_iter()
                        .map(|call| format!("{} {}", call.name, call.args))
                        .collect();
                    if !calls.is_empty() {
                        text.push(' ');
                        text.push_str(&clip_chars(&calls.join(" "), INDEX_CALLS_CHARS).0);
                    }
                }
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, text))
            })?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        if pending.is_empty() {
            return Ok(());
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut clear = tx.prepare("DELETE FROM message_fts WHERE rowid = ?1")?;
            let mut insert = tx.prepare(
                "INSERT INTO message_fts (rowid, content, session_id, message_id)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (rowid, id, text) in &pending {
                // A rowid SQLite handed out again after a delete may still carry the old row.
                clear.execute([rowid])?;
                insert.execute(rusqlite::params![rowid, text, session_id, id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Search the folded history of a session: the rows a compaction removed from the model's
    /// context. Best matches first, at most `limit`. Never reaches another session.
    pub fn search_folded(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FoldedHit>> {
        self.search_history_rows(session_id, query, limit, true)
    }

    /// Search everything a session said that still counts as its conversation: live rows and
    /// folded ones, not the rows `/undo` rewound past. The owner's history search.
    pub fn search_session_history(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FoldedHit>> {
        self.search_history_rows(session_id, query, limit, false)
    }

    fn search_history_rows(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
        folded_only: bool,
    ) -> Result<Vec<FoldedHit>> {
        let (Some(all_terms), Some(any_terms)) = (
            match_expression(query, " "),
            match_expression(query, " OR "),
        ) else {
            return Ok(Vec::new());
        };
        self.index_session_rows(session_id, folded_only)?;
        let conn = self.lock()?;
        let state = if folded_only {
            "m.compacted = 1"
        } else {
            "(m.active = 1 OR m.compacted = 1)"
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT m.seq, m.role, m.compacted,
                    snippet(message_fts, 0, '[', ']', '…', 24)
             FROM message_fts
             JOIN message m ON m.rowid = message_fts.rowid AND m.id = message_fts.message_id
             WHERE message_fts MATCH ?1 AND message_fts.session_id = ?2 AND {state}
               AND m.role != 'system' AND {dup}
             ORDER BY message_fts.rank, m.seq
             LIMIT ?3",
            dup = not_ui_copy("m."),
        ))?;
        let mut run = |expression: &str| -> Result<Vec<FoldedHit>> {
            let rows = stmt.query_map(
                rusqlite::params![expression, session_id, limit as i64],
                |row| {
                    Ok(FoldedHit {
                        seq: row.get(0)?,
                        role: row.get(1)?,
                        folded: row.get::<_, i64>(2)? != 0,
                        snippet: row.get::<_, String>(3)?.replace('\n', " "),
                    })
                },
            )?;
            Ok(rows.collect::<std::result::Result<_, _>>()?)
        };
        let hits = run(&all_terms)?;
        if hits.is_empty() && all_terms != any_terms {
            return run(&any_terms);
        }
        Ok(hits)
    }

    /// The folded rows of `session_id` with `seq_from <= seq <= seq_to`, oldest first, each row
    /// clipped and the whole read held to about `max_chars`. At least one row is returned when any
    /// is in range, so a read always makes progress. Never reaches another session.
    pub fn read_folded(
        &self,
        session_id: &str,
        seq_from: i64,
        seq_to: i64,
        max_chars: usize,
    ) -> Result<FoldedRead> {
        self.read_rows(session_id, seq_from, seq_to, max_chars, "compacted = 1")
    }

    /// [`read_folded`](Self::read_folded) over everything that still counts as the conversation,
    /// live rows and folded ones alike: what the owner's history search reads around a hit.
    pub fn read_history(
        &self,
        session_id: &str,
        seq_from: i64,
        seq_to: i64,
        max_chars: usize,
    ) -> Result<FoldedRead> {
        self.read_rows(
            session_id,
            seq_from,
            seq_to,
            max_chars,
            "(active = 1 OR compacted = 1)",
        )
    }

    fn read_rows(
        &self,
        session_id: &str,
        seq_from: i64,
        seq_to: i64,
        max_chars: usize,
        state: &str,
    ) -> Result<FoldedRead> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT seq, role, content, tool_calls_json FROM message m
             WHERE m.session_id = ?1 AND {state} AND m.seq >= ?2 AND m.seq <= ?3 AND {dup}
             ORDER BY m.seq",
            dup = not_ui_copy("m."),
        ))?;
        let mut rows = stmt.query(rusqlite::params![session_id, seq_from, seq_to])?;
        let mut out = FoldedRead::default();
        let mut used = 0usize;
        while let Some(row) = rows.next()? {
            let seq: i64 = row.get(0)?;
            let role: String = row.get(1)?;
            let content: String = row.get(2)?;
            let calls_json: Option<String> = row.get(3)?;
            let limit = if role == "tool" {
                READ_TOOL_ROW_CHARS
            } else {
                READ_ROW_CHARS
            };
            let (mut text, mut clipped) = clip_chars(&content, limit);
            for call in transcript_read_store::parse_tool_calls_with_diagnostic(
                session_id,
                Some(seq),
                calls_json.as_deref(),
            ) {
                let (args, args_clipped) = clip_chars(&call.args.to_string(), READ_CALL_ARG_CHARS);
                clipped |= args_clipped;
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&format!(
                    "[call {} {args}{}]",
                    call.name,
                    if args_clipped { "…" } else { "" }
                ));
            }
            let cost = text.chars().count() + 24;
            if !out.rows.is_empty() && used + cost > max_chars {
                out.next_seq = Some(seq);
                break;
            }
            used += cost;
            out.rows.push(FoldedRow {
                seq,
                role,
                content: text,
                clipped,
            });
        }
        Ok(out)
    }

    /// The rows `compact_session_store(.., keep_count)` is about to fold, oldest first. Lets the
    /// caller build the episode index for the summary that replaces them.
    pub fn fold_preview(&self, session_id: &str, keep_count: usize) -> Result<Vec<FoldPreviewRow>> {
        let conn = self.lock()?;
        let cut: Option<i64> = if keep_count == 0 {
            None
        } else {
            conn.query_row(
                "SELECT seq FROM message WHERE session_id = ?1 AND active = 1
                 ORDER BY seq DESC LIMIT 1 OFFSET ?2",
                (session_id, keep_count as i64 - 1),
                |row| row.get(0),
            )
            .optional()?
        };
        if keep_count > 0 && cut.is_none() {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare(
            "SELECT seq, role, nudge, content FROM message
             WHERE session_id = ?1 AND active = 1 AND seq < ?2 ORDER BY seq",
        )?;
        let rows = stmt.query_map((session_id, cut.unwrap_or(i64::MAX)), |row| {
            let role: String = row.get(1)?;
            let content: String = row.get(3)?;
            let head = if role == "user" {
                clip_chars(&content, 400).0
            } else {
                String::new()
            };
            Ok(FoldPreviewRow {
                seq: row.get(0)?,
                nudge: row.get::<_, i64>(2)? != 0,
                role,
                head,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Drop a session's search index rows. Called when its messages go away. The option is set
    /// again here for stores that ran migration 57 before it set `secure-delete`.
    pub(crate) fn drop_recall_index(conn: &Connection, session_id: &str) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO message_fts(message_fts, rank) VALUES('secure-delete', 1)",
            [],
        )?;
        conn.execute(
            "DELETE FROM message_fts WHERE rowid IN (SELECT rowid FROM message WHERE session_id = ?1)",
            [session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        let lines = [
            (Role::User, "decide the retry policy for the uploader"),
            (
                Role::Assistant,
                "we settled on exponential backoff capped at 30s",
            ),
            (Role::User, "now fix the parser"),
            (Role::Assistant, "parser fixed"),
            (Role::User, "latest question"),
            (Role::Assistant, "latest answer"),
        ];
        for (seq, (role, text)) in lines.iter().enumerate() {
            store
                .add_message(&sid, seq as i64, *role, text, None)
                .unwrap();
        }
        (store, sid)
    }

    #[test]
    fn nothing_is_folded_before_a_compaction() {
        let (store, sid) = seeded();
        assert!(!store.has_folded_history(&sid).unwrap());
        assert!(store.search_folded(&sid, "backoff", 5).unwrap().is_empty());
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        assert!(store.has_folded_history(&sid).unwrap());
    }

    #[test]
    fn search_finds_folded_rows_only_and_follows_a_fold_and_an_unfold() {
        let (store, sid) = seeded();
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        let hits = store.search_folded(&sid, "exponential backoff", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].seq, hits[0].role.as_str()), (1, "assistant"));
        assert!(hits[0].snippet.contains("[backoff]") || hits[0].snippet.contains("[exponential]"));
        assert!(hits[0].folded);
        // A live row is not folded history, but the owner's search sees it.
        assert!(store.search_folded(&sid, "latest", 5).unwrap().is_empty());
        let all = store.search_session_history(&sid, "latest", 5).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|h| !h.folded));

        // Unfolding makes the rows live again: recall has nothing, history search still finds them.
        assert!(store.uncompact_session_store(&sid).unwrap());
        assert!(store.search_folded(&sid, "backoff", 5).unwrap().is_empty());
        let hit = store.search_session_history(&sid, "backoff", 5).unwrap();
        assert_eq!(hit.len(), 1);
        assert!(!hit[0].folded);

        // Folding again finds them without any further bookkeeping.
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        assert_eq!(store.search_folded(&sid, "backoff", 5).unwrap().len(), 1);
    }

    #[test]
    fn rewound_rows_are_never_found() {
        let (store, sid) = seeded();
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        store.deactivate_messages_from(&sid, 4).unwrap();
        assert!(store
            .search_session_history(&sid, "latest", 5)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_never_crosses_into_another_session() {
        let (store, sid) = seeded();
        let (_, other) = {
            let other = store.create_session("/y", "default").unwrap();
            for (seq, text) in ["secret backoff plan", "b", "c", "d", "e", "f"]
                .iter()
                .enumerate()
            {
                store
                    .add_message(&other, seq as i64, Role::User, text, None)
                    .unwrap();
            }
            store.compact_session_store(&other, "S", 2).unwrap();
            ((), other)
        };
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        let hits = store.search_folded(&sid, "secret", 5).unwrap();
        assert!(hits.is_empty());
        assert_eq!(store.search_folded(&other, "secret", 5).unwrap().len(), 1);
        assert!(store.read_folded(&sid, 0, 99, 10_000).unwrap().rows.len() == 4);
    }

    #[test]
    fn punctuation_and_empty_queries_are_harmless() {
        let (store, sid) = seeded();
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        assert!(store.search_folded(&sid, "   ", 5).unwrap().is_empty());
        assert!(store.search_folded(&sid, "\"*()-:", 5).unwrap().is_empty());
        assert_eq!(
            store
                .search_folded(&sid, "retry-policy: \"uploader\"", 5)
                .unwrap()
                .len(),
            1
        );
        // No row has both words: falls back to any of them.
        assert!(!store
            .search_folded(&sid, "uploader parser", 5)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn read_clips_rows_and_stops_at_the_budget_with_a_resume_point() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "start", None)
            .unwrap();
        store
            .add_message_full(
                &sid,
                1,
                Role::Tool,
                &"t".repeat(5_000),
                None,
                &[],
                Some("c1"),
            )
            .unwrap();
        for seq in 2..10 {
            store
                .add_message(&sid, seq, Role::Assistant, &"a".repeat(900), None)
                .unwrap();
        }
        store.compact_session_store(&sid, "S", 1).unwrap();

        let read = store.read_folded(&sid, 0, 9, 3_000).unwrap();
        assert_eq!(read.rows[0].seq, 0);
        let tool = read.rows.iter().find(|r| r.seq == 1).unwrap();
        assert!(tool.clipped && tool.content.chars().count() == READ_TOOL_ROW_CHARS);
        assert!(read.rows.len() < 8, "the budget cut the read short");
        let resume = read.next_seq.expect("a resume point");
        assert_eq!(resume, read.rows.last().unwrap().seq + 1);
        let rest = store.read_folded(&sid, resume, 9, 100_000).unwrap();
        assert_eq!(rest.next_seq, None);
        assert_eq!(rest.rows.first().unwrap().seq, resume);
        // The live tail (seq 9) is not folded history.
        assert_eq!(rest.rows.last().unwrap().seq, 8);
    }

    #[test]
    fn a_reply_stored_for_the_model_and_the_screen_is_found_and_read_once() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "pick the cache key", None)
            .unwrap();
        store
            .add_llm_only_message_full(
                &sid,
                1,
                Role::Assistant,
                "use v3 tenant keys",
                None,
                &[],
                None,
            )
            .unwrap();
        store
            .add_ui_note(&sid, 2, Role::Assistant, "use v3 tenant keys")
            .unwrap();
        for seq in 3..8 {
            store
                .add_message(&sid, seq, Role::User, "filler", None)
                .unwrap();
        }
        store.compact_session_store(&sid, "S", 2).unwrap();
        let hits = store.search_folded(&sid, "tenant keys", 5).unwrap();
        assert_eq!(hits.iter().map(|h| h.seq).collect::<Vec<_>>(), [1]);
        let rows = store.read_folded(&sid, 0, 4, 10_000).unwrap().rows;
        assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), [0, 1, 3, 4]);
    }

    #[test]
    fn harness_injected_system_rows_are_not_searchable() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "name the cache key", None)
            .unwrap();
        store
            .add_message(
                &sid,
                1,
                Role::System,
                "Relevant code (Lattice): cache.py:1 function cache_key",
                None,
            )
            .unwrap();
        for seq in 2..8 {
            store
                .add_message(&sid, seq, Role::User, "filler", None)
                .unwrap();
        }
        store.compact_session_store(&sid, "S", 2).unwrap();
        let hits = store.search_folded(&sid, "cache", 5).unwrap();
        assert_eq!(hits.iter().map(|h| h.seq).collect::<Vec<_>>(), [0]);
        let all = store.search_session_history(&sid, "cache", 5).unwrap();
        assert_eq!(all.iter().map(|h| h.seq).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn a_tool_call_row_is_found_by_its_arguments_and_shows_no_call_id() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "start", None)
            .unwrap();
        let call = forge_types::ToolCall {
            id: "call_00_zzzIDzzz".into(),
            name: "read_file".into(),
            args: serde_json::json!({"path": "billing/invoice.rs"}),
        };
        store
            .add_message_full(&sid, 1, Role::Assistant, "", None, &[call], None)
            .unwrap();
        for seq in 2..8 {
            store
                .add_message(&sid, seq, Role::User, "filler", None)
                .unwrap();
        }
        store.compact_session_store(&sid, "S", 2).unwrap();
        let hits = store.search_folded(&sid, "invoice", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].snippet.contains("read_file"),
            "{:?}",
            hits[0].snippet
        );
        assert!(!hits[0].snippet.contains("zzzID"), "{:?}", hits[0].snippet);
    }

    #[test]
    fn read_history_covers_live_and_folded_rows() {
        let (store, sid) = seeded();
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        let rows = store.read_history(&sid, 0, 99, 10_000).unwrap().rows;
        assert_eq!(
            rows.iter().map(|r| r.seq).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5]
        );
        assert_eq!(
            store.read_folded(&sid, 0, 99, 10_000).unwrap().rows.len(),
            4
        );
    }

    #[test]
    fn fold_preview_lists_exactly_the_rows_a_fold_removes() {
        let (store, sid) = seeded();
        let preview = store.fold_preview(&sid, 2).unwrap();
        assert_eq!(
            preview.iter().map(|r| r.seq).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(preview[0].head, "decide the retry policy for the uploader");
        assert!(preview[1].head.is_empty());
        assert!(store.fold_preview(&sid, 99).unwrap().is_empty());
    }
}
