//! One row per prompt-started turn and the reason it ended (docs/features/sessions.md, "Turn
//! endings"). The reason strings are chosen by the session core; the store keeps them opaque and
//! only derives `last_role_shape` from the transcript tail when a turn closes.

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::interrupted_turn_store::{classify_tail, InterruptedTail, TailRow, TailState};
use crate::{Result, Store};

/// What the transcript ended with when the turn closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnShape {
    Finished,
    UnansweredPrompt,
    PendingToolCalls,
    UnansweredToolResults,
}

impl TurnShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::UnansweredPrompt => "unanswered_prompt",
            Self::PendingToolCalls => "pending_tool_calls",
            Self::UnansweredToolResults => "unanswered_tool_results",
        }
    }

    pub fn from_tail(state: TailState) -> Self {
        match state {
            TailState::Finished | TailState::UserInterrupted => Self::Finished,
            TailState::Interrupted(InterruptedTail::UnansweredPrompt) => Self::UnansweredPrompt,
            TailState::Interrupted(InterruptedTail::PendingToolCalls) => Self::PendingToolCalls,
            TailState::Interrupted(InterruptedTail::UnansweredToolResults) => {
                Self::UnansweredToolResults
            }
        }
    }
}

/// A stored turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnRow {
    pub id: i64,
    pub session_id: String,
    pub prompt_seq: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub end_reason: Option<String>,
    pub detail_json: Option<String>,
    pub last_role_shape: Option<String>,
}

const COLS: &str =
    "id, session_id, prompt_seq, started_at, ended_at, end_reason, detail_json, last_role_shape";

fn row_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRow> {
    Ok(TurnRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        prompt_seq: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
        end_reason: row.get(5)?,
        detail_json: row.get(6)?,
        last_role_shape: row.get(7)?,
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn tail_shape(conn: &rusqlite::Connection, session_id: &str) -> rusqlite::Result<TurnShape> {
    let mut stmt = conn.prepare_cached(
        "SELECT role, content, tool_calls_json, tool_call_id, visibility FROM message \
         WHERE session_id = ?1 AND active = 1 AND visibility != 'llm_only' \
         ORDER BY seq DESC LIMIT 256",
    )?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok(TailRow {
                role: row.get(0)?,
                content: row.get(1)?,
                tool_calls_json: row.get(2)?,
                tool_call_id: row.get(3)?,
                visibility: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(TurnShape::from_tail(classify_tail(&rows)))
}

impl Store {
    /// Open a turn for the prompt persisted at `prompt_seq`; returns its id.
    pub fn open_turn(&self, session_id: &str, prompt_seq: i64) -> Result<i64> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO turn (session_id, prompt_seq, started_at) VALUES (?1, ?2, ?3)",
            params![session_id, prompt_seq, now_secs()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Close an open turn with its end reason and detail. A turn that is already closed keeps
    /// its first answer (a guard and the normal path may both try). Returns whether it closed.
    pub fn close_turn(&self, id: i64, reason: &str, detail_json: Option<&str>) -> Result<bool> {
        let conn = self.lock()?;
        let Some(session_id) = conn
            .query_row(
                "SELECT session_id FROM turn WHERE id = ?1 AND ended_at IS NULL",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let shape = tail_shape(&conn, &session_id)?;
        let changed = conn.execute(
            "UPDATE turn SET ended_at = ?2, end_reason = ?3, detail_json = ?4, \
             last_role_shape = ?5 WHERE id = ?1 AND ended_at IS NULL",
            params![id, now_secs(), reason, detail_json, shape.as_str()],
        )?;
        Ok(changed > 0)
    }

    /// Crash/restart sweep: close every turn still open as `reason`. Returns how many.
    pub fn close_open_turns_as(&self, reason: &str) -> Result<usize> {
        self.close_open_turns(None, reason)
    }

    /// The same sweep for one session: its open turns cannot still be running once the process
    /// that ran them is gone, or once a new prompt is about to start.
    pub fn close_open_session_turns_as(&self, session_id: &str, reason: &str) -> Result<usize> {
        self.close_open_turns(Some(session_id), reason)
    }

    fn close_open_turns(&self, session_id: Option<&str>, reason: &str) -> Result<usize> {
        let conn = self.lock()?;
        let open: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, session_id FROM turn WHERE ended_at IS NULL \
                 AND (?1 IS NULL OR session_id = ?1)",
            )?;
            let rows = stmt
                .query_map([session_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        let now = now_secs();
        for (id, session_id) in &open {
            let shape = tail_shape(&conn, session_id)?;
            conn.execute(
                "UPDATE turn SET ended_at = ?2, end_reason = ?3, last_role_shape = ?4 \
                 WHERE id = ?1 AND ended_at IS NULL",
                params![id, now, reason, shape.as_str()],
            )?;
        }
        Ok(open.len())
    }

    /// The session's newest turns, newest first.
    pub fn recent_turns(&self, session_id: &str, limit: usize) -> Result<Vec<TurnRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM turn WHERE session_id = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![session_id, limit as i64], row_of)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The session's newest closed turn.
    pub fn last_closed_turn(&self, session_id: &str) -> Result<Option<TurnRow>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM turn WHERE session_id = ?1 AND ended_at IS NOT NULL \
                     ORDER BY started_at DESC, id DESC LIMIT 1"
                ),
                [session_id],
                row_of,
            )
            .optional()?)
    }

    /// Closed turns since `since` (unix secs) that did not end in a normal answer, newest first.
    pub fn recent_unfinished_turns(
        &self,
        since: i64,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TurnRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM turn WHERE started_at >= ?1 AND (?2 IS NULL OR session_id = ?2) \
             AND ended_at IS NOT NULL AND end_reason NOT IN ('final_answer', 'plan_presented') \
             ORDER BY started_at DESC, id DESC LIMIT ?3"
        ))?;
        let rows = stmt
            .query_map(params![since, session_id, limit as i64], row_of)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Turn counts by end reason since `since` (unix secs), most common first. Turns still open
    /// count under `running`.
    pub fn turn_end_counts(
        &self,
        since: i64,
        session_id: Option<&str>,
    ) -> Result<Vec<(String, i64)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(end_reason, 'running'), COUNT(*) AS n FROM turn \
             WHERE started_at >= ?1 AND (?2 IS NULL OR session_id = ?2) \
             GROUP BY 1 ORDER BY n DESC, 1",
        )?;
        let rows = stmt
            .query_map(params![since, session_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Reconstruct turns from the transcript of sessions that have no `turn` row for a prompt
    /// yet: shape only, reason `unknown_<shape>`. A prompt is a user row that is not a harness
    /// nudge; steers cannot be told apart from prompts in old data, so they count as turns.
    /// Read-only; [`Store::write_backfilled_turns`] stores the result.
    pub fn backfill_turns(&self, session_id: Option<&str>, since: i64) -> Result<Vec<TurnRow>> {
        let conn = self.lock()?;
        let mut sessions = conn.prepare(
            "SELECT DISTINCT session_id FROM message \
             WHERE role = 'user' AND nudge = 0 AND active = 1 AND visibility = 'llm' \
             AND created_at >= ?1 AND (?2 IS NULL OR session_id = ?2)",
        )?;
        let ids = sessions
            .query_map(params![since, session_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::new();
        for sid in ids {
            let mut stmt = conn.prepare_cached(
                "SELECT seq, role, content, tool_calls_json, tool_call_id, visibility, nudge, \
                 created_at FROM message WHERE session_id = ?1 AND active = 1 \
                 AND visibility != 'llm_only' ORDER BY seq",
            )?;
            let rows = stmt
                .query_map([&sid], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        TailRow {
                            role: row.get(1)?,
                            content: row.get(2)?,
                            tool_calls_json: row.get(3)?,
                            tool_call_id: row.get(4)?,
                            visibility: row.get(5)?,
                        },
                        row.get::<_, i64>(6)? != 0,
                        row.get::<_, i64>(7)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let known: std::collections::HashSet<i64> = {
                let mut k =
                    conn.prepare_cached("SELECT prompt_seq FROM turn WHERE session_id = ?1")?;
                let v = k
                    .query_map([&sid], |row| row.get::<_, i64>(0))?
                    .collect::<std::result::Result<_, _>>()?;
                v
            };
            let is_prompt = |r: &(i64, TailRow, bool, i64)| {
                r.1.role == "user" && !r.2 && r.1.visibility == "llm"
            };
            let starts: Vec<usize> = (0..rows.len()).filter(|&i| is_prompt(&rows[i])).collect();
            for (n, &start) in starts.iter().enumerate() {
                let end = starts.get(n + 1).copied().unwrap_or(rows.len());
                let (seq, _, _, created) = &rows[start];
                if *created < since || known.contains(seq) {
                    continue;
                }
                let newest_first: Vec<TailRow> =
                    rows[start..end].iter().rev().map(|r| r.1.clone()).collect();
                let shape = TurnShape::from_tail(classify_tail(&newest_first));
                out.push(TurnRow {
                    id: 0,
                    session_id: sid.clone(),
                    prompt_seq: *seq,
                    started_at: *created,
                    ended_at: Some(rows[end - 1].3),
                    end_reason: Some(format!("unknown_{}", shape.as_str())),
                    detail_json: None,
                    last_role_shape: Some(shape.as_str().to_string()),
                });
            }
        }
        out.sort_by_key(|t| std::cmp::Reverse(t.started_at));
        Ok(out)
    }

    /// Store the turns [`Store::backfill_turns`] returned. Returns how many were written.
    pub fn write_backfilled_turns(&self, turns: &[TurnRow]) -> Result<usize> {
        let conn = self.lock()?;
        let mut written = 0;
        for t in turns {
            written += conn.execute(
                "INSERT INTO turn (session_id, prompt_seq, started_at, ended_at, end_reason, \
                 detail_json, last_role_shape) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE NOT EXISTS \
                 (SELECT 1 FROM turn WHERE session_id = ?1 AND prompt_seq = ?2)",
                params![
                    t.session_id,
                    t.prompt_seq,
                    t.started_at,
                    t.ended_at,
                    t.end_reason,
                    t.detail_json,
                    t.last_role_shape
                ],
            )?;
        }
        Ok(written)
    }

    /// Append the UI note for a turn that stopped without a normal answer, unless the newest
    /// visible row is already a UI system note (a budget refusal, a loop-guard halt): callers run
    /// this for a turn whose prompt is persisted, so that row belongs to this turn.
    /// Returns whether a note was written.
    pub fn note_turn_stopped(&self, session_id: &str, text: &str) -> Result<bool> {
        let newest: Option<(String, String, String)> = {
            let conn = self.lock()?;
            conn.query_row(
                "SELECT role, visibility, content FROM message WHERE session_id = ?1 \
                 AND active = 1 AND visibility != 'llm_only' ORDER BY seq DESC LIMIT 1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
        };
        if let Some((role, visibility, content)) = newest {
            let _ = content;
            if role == "system" && visibility == "ui" {
                return Ok(false);
            }
        }
        let seq = self.next_message_seq(session_id)?;
        self.add_ui_note(session_id, seq, forge_types::Role::System, text)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::{Role, ToolCall};

    fn store_with_prompt() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        (store, sid)
    }

    #[test]
    fn open_close_records_reason_detail_and_shape() {
        let (store, sid) = store_with_prompt();
        let id = store.open_turn(&sid, 0).unwrap();
        assert_eq!(
            store.turn_end_counts(0, None).unwrap(),
            [("running".into(), 1)]
        );
        assert!(store
            .close_turn(id, "provider_error", Some("{\"kind\":\"x\"}"))
            .unwrap());
        assert!(
            !store.close_turn(id, "final_answer", None).unwrap(),
            "first close wins"
        );
        let t = &store.recent_turns(&sid, 5).unwrap()[0];
        assert_eq!(t.end_reason.as_deref(), Some("provider_error"));
        assert_eq!(t.detail_json.as_deref(), Some("{\"kind\":\"x\"}"));
        assert_eq!(t.last_role_shape.as_deref(), Some("unanswered_prompt"));
        assert!(t.ended_at.is_some());
    }

    #[test]
    fn shape_follows_the_tail() {
        let (store, sid) = store_with_prompt();
        store
            .add_message_full(
                &sid,
                1,
                Role::Assistant,
                "",
                None,
                &[ToolCall {
                    id: "a".into(),
                    name: "shell".into(),
                    args: serde_json::json!({}),
                }],
                None,
            )
            .unwrap();
        let id = store.open_turn(&sid, 0).unwrap();
        store.close_turn(id, "interrupted", None).unwrap();
        assert_eq!(
            store
                .last_closed_turn(&sid)
                .unwrap()
                .unwrap()
                .last_role_shape
                .as_deref(),
            Some("pending_tool_calls")
        );
        store
            .add_message(&sid, 2, Role::Assistant, "ok", None)
            .unwrap();
        let id = store.open_turn(&sid, 0).unwrap();
        store.close_turn(id, "final_answer", None).unwrap();
        assert_eq!(
            store
                .last_closed_turn(&sid)
                .unwrap()
                .unwrap()
                .last_role_shape
                .as_deref(),
            Some("finished")
        );
    }

    #[test]
    fn crash_sweep_closes_only_open_turns() {
        let (store, sid) = store_with_prompt();
        let done = store.open_turn(&sid, 0).unwrap();
        store.close_turn(done, "final_answer", None).unwrap();
        store.open_turn(&sid, 1).unwrap();
        assert_eq!(
            store
                .close_open_turns_as("daemon_restart_or_crash")
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .close_open_turns_as("daemon_restart_or_crash")
                .unwrap(),
            0
        );
        let counts = store.turn_end_counts(0, Some(&sid)).unwrap();
        assert!(counts.contains(&("daemon_restart_or_crash".to_string(), 1)));
        assert!(counts.contains(&("final_answer".to_string(), 1)));
    }

    #[test]
    fn a_session_sweep_leaves_other_sessions_alone() {
        let (store, a) = store_with_prompt();
        let b = store.create_session("/repo", "default").unwrap();
        store.open_turn(&a, 0).unwrap();
        store.open_turn(&b, 0).unwrap();
        assert_eq!(
            store
                .close_open_session_turns_as(&a, "daemon_restart_or_crash")
                .unwrap(),
            1
        );
        assert_eq!(
            store.turn_end_counts(0, Some(&b)).unwrap(),
            [("running".into(), 1)]
        );
    }

    #[test]
    fn unfinished_filter_and_window() {
        let (store, sid) = store_with_prompt();
        for (seq, reason) in [(0, "final_answer"), (1, "stalled"), (2, "provider_error")] {
            let id = store.open_turn(&sid, seq).unwrap();
            store.close_turn(id, reason, None).unwrap();
        }
        let bad = store.recent_unfinished_turns(0, None, 10).unwrap();
        let reasons: Vec<_> = bad.iter().filter_map(|t| t.end_reason.as_deref()).collect();
        assert_eq!(reasons.len(), 2);
        assert!(!reasons.contains(&"final_answer"));
        assert!(store
            .recent_unfinished_turns(i64::MAX, None, 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn backfill_classifies_history_and_is_idempotent() {
        let (store, sid) = store_with_prompt();
        store
            .add_message(&sid, 1, Role::Assistant, "answer", None)
            .unwrap();
        store
            .add_message(&sid, 2, Role::User, "again", None)
            .unwrap();
        let turns = store.backfill_turns(None, 0).unwrap();
        assert_eq!(turns.len(), 2);
        let reasons: Vec<_> = turns.iter().filter_map(|t| t.end_reason.clone()).collect();
        assert!(reasons.contains(&"unknown_finished".to_string()));
        assert!(reasons.contains(&"unknown_unanswered_prompt".to_string()));
        assert_eq!(
            store.turn_end_counts(0, None).unwrap().len(),
            0,
            "read-only by default"
        );
        assert_eq!(store.write_backfilled_turns(&turns).unwrap(), 2);
        assert!(store.backfill_turns(None, 0).unwrap().is_empty());
        assert_eq!(store.write_backfilled_turns(&turns).unwrap(), 0);
    }

    #[test]
    fn stop_note_is_written_once() {
        let (store, sid) = store_with_prompt();
        assert!(store
            .note_turn_stopped(&sid, "⚠ Turn stopped (stalled): send continue")
            .unwrap());
        assert!(!store
            .note_turn_stopped(&sid, "⚠ Turn stopped (stalled): again")
            .unwrap());
        assert_eq!(store.session_tail_state(&sid).unwrap(), TailState::Finished);
    }
}
