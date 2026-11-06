//! Recognising a turn that a daemon restart cut off, from the transcript alone.

use super::*;

/// Prefix of the UI-only note a user interrupt leaves behind. Without it an Esc looks exactly
/// like a crash (a user message with no reply), and a restart would re-drive work the user
/// stopped on purpose.
pub const USER_INTERRUPT_NOTE: &str = "⏹ Interrupted by you.";
/// Prefix of the UI-only note written when a restart found a turn cut off.
pub const RESTART_INTERRUPT_NOTE: &str = "⚠ Interrupted by a daemon restart.";

/// How far back the transcript tail is read; one assistant tool-call batch plus its results is
/// far shorter than this.
const TAIL_ROWS: i64 = 256;

/// What a transcript ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailState {
    /// Nothing is owed: the last turn answered, ended on a note, or the session is empty.
    Finished,
    /// The user stopped the turn on purpose.
    UserInterrupted,
    /// The transcript stops mid-turn.
    Interrupted(InterruptedTail),
}

/// The shape of an unfinished tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptedTail {
    /// A user prompt with no assistant reply after it.
    UnansweredPrompt,
    /// An assistant tool call with at least one result missing.
    PendingToolCalls,
    /// Every tool result is in, but the model never replied to them.
    UnansweredToolResults,
}

/// One persisted row, reduced to what tail classification reads.
#[derive(Debug, Clone)]
pub struct TailRow {
    pub role: String,
    pub content: String,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
    pub visibility: String,
}

/// Classify a transcript tail, newest row first. `LlmOnly` rows must already be filtered out.
pub fn classify_tail(newest_first: &[TailRow]) -> TailState {
    let Some(last) = newest_first.first() else {
        return TailState::Finished;
    };
    if last.visibility == "ui" {
        if last.role == "system" && last.content.starts_with(USER_INTERRUPT_NOTE) {
            return TailState::UserInterrupted;
        }
        if !(last.role == "system" && last.content.starts_with(RESTART_INTERRUPT_NOTE)) {
            // A published answer, a budget note, a command echo: the turn ended on it.
            return TailState::Finished;
        }
    }
    // Restart notes are transparent: a restart that failed to bring the session back must not
    // hide the cut-off turn from the next one. So are model-facing system rows (context a resume
    // injects, summaries): they are never the end of a turn.
    let mut rows = newest_first.iter().filter(|r| {
        !(r.role == "system"
            && (r.visibility != "ui" || r.content.starts_with(RESTART_INTERRUPT_NOTE)))
    });
    let Some(tail) = rows.next() else {
        return TailState::Finished;
    };
    match tail.role.as_str() {
        "user" => TailState::Interrupted(InterruptedTail::UnansweredPrompt),
        "assistant" => {
            if tail_calls(tail).is_empty() {
                TailState::Finished
            } else {
                TailState::Interrupted(InterruptedTail::PendingToolCalls)
            }
        }
        "tool" => {
            let mut answered: Vec<&str> = Vec::new();
            for row in std::iter::once(tail).chain(rows) {
                match row.role.as_str() {
                    "tool" => answered.extend(row.tool_call_id.as_deref()),
                    "assistant" => {
                        let calls = tail_calls(row);
                        if calls.is_empty() {
                            return TailState::Finished;
                        }
                        return if calls.iter().all(|id| answered.contains(&id.as_str())) {
                            TailState::Interrupted(InterruptedTail::UnansweredToolResults)
                        } else {
                            TailState::Interrupted(InterruptedTail::PendingToolCalls)
                        };
                    }
                    _ => return TailState::Finished,
                }
            }
            TailState::Finished
        }
        _ => TailState::Finished,
    }
}

fn tail_calls(row: &TailRow) -> Vec<String> {
    row.tool_calls_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Vec<forge_types::ToolCall>>(json).ok())
        .map(|calls| calls.into_iter().map(|c| c.id).collect())
        .unwrap_or_default()
}

impl Store {
    /// Whether the session's transcript ends mid-turn, and how.
    pub fn session_tail_state(&self, session_id: &str) -> Result<TailState> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT role, content, tool_calls_json, tool_call_id, visibility FROM message \
             WHERE session_id = ?1 AND active = 1 AND visibility != 'llm_only' \
             ORDER BY seq DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map((session_id, TAIL_ROWS), |row| {
                Ok(TailRow {
                    role: row.get(0)?,
                    content: row.get(1)?,
                    tool_calls_json: row.get(2)?,
                    tool_call_id: row.get(3)?,
                    visibility: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(classify_tail(&rows))
    }

    /// Append the UI-only note that marks the turn as stopped on purpose.
    pub fn note_user_interrupt(&self, session_id: &str) -> Result<()> {
        let seq = self.next_message_seq(session_id)?;
        self.add_ui_note(session_id, seq, Role::System, USER_INTERRUPT_NOTE)
            .map(|_| ())
    }

    /// Append the UI-only note that records why a turn failed. Without it the failure lived only
    /// in the live snapshot: history read later showed an unanswered prompt, and a restart took the
    /// failed turn for one it had cut off and re-drove it.
    pub fn note_turn_failure(&self, session_id: &str, error: &str) -> Result<()> {
        let seq = self.next_message_seq(session_id)?;
        let error: String = error.chars().take(500).collect();
        self.add_ui_note(
            session_id,
            seq,
            Role::System,
            &format!("⚠ Turn failed: {error}"),
        )
        .map(|_| ())
    }

    /// Append the UI-only note that tells the user a restart cut the turn off.
    pub fn note_restart_interrupt(&self, session_id: &str, will_continue: bool) -> Result<()> {
        let seq = self.next_message_seq(session_id)?;
        let tail = if will_continue {
            " Forge is continuing it."
        } else {
            ""
        };
        self.add_ui_note(
            session_id,
            seq,
            Role::System,
            &format!("{RESTART_INTERRUPT_NOTE}{tail}"),
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::ToolCall;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "shell".into(),
            args: serde_json::json!({}),
        }
    }

    fn store_with_session() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        (store, sid)
    }

    #[test]
    fn empty_session_and_answered_turn_are_finished() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        assert_eq!(store.session_tail_state(&sid).unwrap(), TailState::Finished);
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        store
            .add_message(&sid, 1, Role::Assistant, "done", None)
            .unwrap();
        assert_eq!(store.session_tail_state(&sid).unwrap(), TailState::Finished);
    }

    #[test]
    fn unanswered_prompt_is_interrupted() {
        let (store, sid) = store_with_session();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::UnansweredPrompt)
        );
    }

    #[test]
    fn a_failed_turn_is_finished_not_cut_off() {
        let (store, sid) = store_with_session();
        store
            .note_turn_failure(&sid, "provider busy (503 no_capacity)")
            .unwrap();
        assert_eq!(store.session_tail_state(&sid).unwrap(), TailState::Finished);
    }

    #[test]
    fn tool_call_missing_a_result_is_interrupted() {
        let (store, sid) = store_with_session();
        store
            .add_message_full(
                &sid,
                1,
                Role::Assistant,
                "",
                None,
                &[call("a"), call("b")],
                None,
            )
            .unwrap();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::PendingToolCalls)
        );
        store
            .add_message_full(&sid, 2, Role::Tool, "ok", None, &[], Some("a"))
            .unwrap();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::PendingToolCalls),
            "one of two results is still missing"
        );
    }

    #[test]
    fn all_results_in_but_no_reply_is_interrupted() {
        let (store, sid) = store_with_session();
        store
            .add_message_full(
                &sid,
                1,
                Role::Assistant,
                "",
                None,
                &[call("a"), call("b")],
                None,
            )
            .unwrap();
        for (seq, id) in [(2, "a"), (3, "b")] {
            store
                .add_message_full(&sid, seq, Role::Tool, "ok", None, &[], Some(id))
                .unwrap();
        }
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::UnansweredToolResults)
        );
        store
            .add_message(&sid, 4, Role::Assistant, "finished", None)
            .unwrap();
        assert_eq!(store.session_tail_state(&sid).unwrap(), TailState::Finished);
    }

    #[test]
    fn user_interrupt_note_is_not_re_driven() {
        let (store, sid) = store_with_session();
        store.note_user_interrupt(&sid).unwrap();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::UserInterrupted
        );
        store
            .add_message(&sid, 2, Role::User, "instead do this", None)
            .unwrap();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::UnansweredPrompt),
            "a later prompt is a new turn"
        );
    }

    #[test]
    fn published_answer_or_budget_note_ends_the_turn() {
        let (store, sid) = store_with_session();
        store
            .add_ui_note(&sid, 1, Role::System, "⚠ budget cap reached")
            .unwrap();
        assert_eq!(store.session_tail_state(&sid).unwrap(), TailState::Finished);
        let (store, sid) = store_with_session();
        store
            .add_ui_note(&sid, 1, Role::Assistant, "the answer")
            .unwrap();
        assert_eq!(store.session_tail_state(&sid).unwrap(), TailState::Finished);
    }

    #[test]
    fn restart_note_does_not_hide_the_cut_off_turn() {
        let (store, sid) = store_with_session();
        store.note_restart_interrupt(&sid, true).unwrap();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::UnansweredPrompt)
        );
    }

    #[test]
    fn model_facing_system_rows_do_not_end_a_turn() {
        let (store, sid) = store_with_session();
        store
            .add_message(&sid, 1, Role::System, "injected context", None)
            .unwrap();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::UnansweredPrompt)
        );
    }

    #[test]
    fn llm_only_rows_are_ignored() {
        let (store, sid) = store_with_session();
        store
            .add_llm_only_message_full(&sid, 1, Role::Assistant, "provisional", None, &[], None)
            .unwrap();
        assert_eq!(
            store.session_tail_state(&sid).unwrap(),
            TailState::Interrupted(InterruptedTail::UnansweredPrompt)
        );
    }
}
