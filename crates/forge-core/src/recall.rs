//! The `recall` virtual tool: search and read the part of this session's own history that a
//! compaction folded out of the model's context.
//!
//! The folded rows stay in the store, but until now nothing the model could call reached them, so
//! a stalled task whose detail had been summarized away could only be asked about or dropped.
//! `recall` is read-only, answers from this session's rows and the sessions that handed it their
//! goal (a fresh-worker relay, docs/features/goal-contract.md), and caps what it returns. It is
//! advertised once the session has at least one compaction, on the direct path and, through
//! `forge mcp-serve`, on the CLI bridge. Both paths call [`run`].

use forge_provider::ToolSpec;
use forge_store::Store;
use serde_json::Value;

use crate::{CoreError, Session};

/// The tool name.
pub const RECALL_TOOL: &str = "recall";

/// The most characters one `recall` result returns.
pub const MAX_OUTPUT_CHARS: usize = 8_000;
/// Hits one search returns.
const SEARCH_LIMIT: usize = 12;
/// Rows a read covers when `to_seq` is omitted.
const DEFAULT_READ_ROWS: i64 = 40;
/// Characters kept for the rows of a read; the rest of the cap is the header and footer.
const READ_BUDGET_CHARS: usize = MAX_OUTPUT_CHARS - 600;

/// The one line added to the system prompt while `recall` is advertised.
pub(crate) const SYSTEM_LINE: &str =
    "\n- Earlier parts of this session were folded into a summary. When you need something from \
them that the summary does not state exactly (a quoted line, a number, a file's contents, the \
reason behind a decision), call the `recall` tool (search a few distinctive words, then read the \
seq range) instead of reconstructing it from memory or guessing from what is still visible, and \
before you ask the user or drop work whose detail you cannot place.";

/// The `ToolSpec` advertised to the model for [`RECALL_TOOL`].
pub fn recall_spec() -> ToolSpec {
    ToolSpec {
        name: RECALL_TOOL.to_string(),
        description: "Search or read this session's folded history: the messages a compaction \
            replaced with a summary. They are exact and complete in the store. Use `search` with \
            a few distinctive words (a file name, an error, a decision) to find where something \
            was discussed, then `read` the seq range around a hit. Use it whenever the summary lacks the \
            exact text, value or reason you need; do not guess it. Read-only; it reaches only this \
            session and the earlier sessions that handed their goal to it. The summary's \"Earlier \
            episodes\" list says which seq ranges exist. \
            Output is capped at 8,000 characters; tool results are clipped, so re-run the \
            command or read the file if you need the full output."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "read"],
                    "description": "search: find folded messages by words. read: show a seq range."
                },
                "query": {
                    "type": "string",
                    "description": "for search: the words to look for (any order)"
                },
                "from_seq": {
                    "type": "integer",
                    "description": "for read: first message seq"
                },
                "to_seq": {
                    "type": "integer",
                    "description": "for read: last message seq (default: from_seq + 40)"
                },
                "session": {
                    "type": "string",
                    "description": "for read: the predecessor session id (or its first 8 characters) a search hit named; omit for this session"
                }
            },
            "required": ["action"]
        }),
    }
}

fn cap(mut text: String) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text;
    }
    let cut = text
        .char_indices()
        .nth(MAX_OUTPUT_CHARS - 40)
        .map_or(text.len(), |(i, _)| i);
    text.truncate(cut);
    text.push_str("\n[output truncated at 8000 characters]");
    text
}

/// Run one `recall` call against `session_id`'s own rows. Returns the text for the model and
/// whether the call succeeded. Never panics on bad arguments.
pub fn run(store: &Store, session_id: &str, args: &Value) -> (String, bool) {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let outcome = match action {
        "search" => search(store, session_id, args),
        "read" => read(store, session_id, args),
        other => Err(format!(
            "recall: `action` must be \"search\" or \"read\" (got {other:?})"
        )),
    };
    match outcome {
        Ok(text) => (cap(text), true),
        Err(error) => (format!("error: {error}"), false),
    }
}

fn predecessors(store: &Store, session_id: &str) -> Vec<String> {
    store.relay_predecessors(session_id).unwrap_or_default()
}

fn nothing_folded(store: &Store, session_id: &str) -> Result<(), String> {
    match store.has_folded_history(session_id) {
        Ok(true) => Ok(()),
        Ok(false) if !predecessors(store, session_id).is_empty() => Ok(()),
        Ok(false) => Err(
            "nothing in this session has been folded by a compaction yet; the \
            whole conversation is already in your context"
                .to_string(),
        ),
        Err(error) => Err(format!("could not read the session history: {error}")),
    }
}

fn search(store: &Store, session_id: &str, args: &Value) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if query.is_empty() {
        return Err("recall search needs a `query`".to_string());
    }
    nothing_folded(store, session_id)?;
    let hits = if store.has_folded_history(session_id).unwrap_or(false) {
        store
            .search_folded(session_id, query, SEARCH_LIMIT)
            .map_err(|error| format!("search failed: {error}"))?
    } else {
        Vec::new()
    };
    let earlier = store
        .search_predecessors(session_id, query, SEARCH_LIMIT)
        .map_err(|error| format!("search failed: {error}"))?;
    if hits.is_empty() && earlier.is_empty() {
        return Ok(format!(
            "no folded message matches {query:?}. Try fewer or different words (a file name, a \
             function, an error text), or read a seq range from the summary's episode list."
        ));
    }
    let mut out = format!(
        "{} {}message(s) match {query:?}, best first:\n",
        hits.len() + earlier.len(),
        if earlier.is_empty() { "folded " } else { "" }
    );
    for hit in &hits {
        out.push_str(&format!(
            "seq {} [{}]: {}\n",
            hit.seq,
            hit.role,
            hit.snippet.trim()
        ));
    }
    for (session, hit) in &earlier {
        out.push_str(&format!(
            "earlier session {} seq {} [{}]: {}\n",
            &session[..session.len().min(8)],
            hit.seq,
            hit.role,
            hit.snippet.trim()
        ));
    }
    out.push_str("\nUse recall read {from_seq, to_seq} to read the messages around a hit");
    out.push_str(if earlier.is_empty() {
        "."
    } else {
        " (add `session` with the earlier session's id for a hit from one)."
    });
    Ok(out)
}

fn read(store: &Store, session_id: &str, args: &Value) -> Result<String, String> {
    let seq = |key: &str| args.get(key).and_then(Value::as_i64);
    let Some(from) = seq("from_seq") else {
        return Err("recall read needs `from_seq` (an integer)".to_string());
    };
    let to = seq("to_seq").unwrap_or(from.saturating_add(DEFAULT_READ_ROWS));
    if to < from {
        return Err("`to_seq` is before `from_seq`".to_string());
    }
    nothing_folded(store, session_id)?;
    let asked = args
        .get("session")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let read = match asked {
        None => store.read_folded(session_id, from, to, READ_BUDGET_CHARS),
        Some(prefix) => {
            let Some(earlier) = predecessors(store, session_id)
                .into_iter()
                .find(|id| id.starts_with(prefix))
            else {
                return Err(format!(
                    "{prefix:?} is not an earlier session of this goal; recall only reaches the \
                     sessions that handed their goal to this one"
                ));
            };
            store.read_history(&earlier, from, to, READ_BUDGET_CHARS)
        }
    }
    .map_err(|error| format!("read failed: {error}"))?;
    if read.rows.is_empty() {
        return Ok(format!(
            "no folded message has a seq in {from}-{to}. Messages after the last compaction are \
             still in your context."
        ));
    }
    let mut out = String::new();
    for row in &read.rows {
        out.push_str(&format!(
            "--- seq {} [{}] ---\n{}",
            row.seq, row.role, row.content
        ));
        if row.clipped {
            out.push_str("\n[clipped]");
        }
        out.push('\n');
    }
    if let Some(next) = read.next_seq {
        out.push_str(&format!(
            "\n[output limit reached; continue with recall read from_seq={next}]"
        ));
    }
    Ok(out)
}

impl Session {
    /// Whether `recall` is advertised now: the session has folded history.
    pub(crate) fn recall_on(&self) -> bool {
        self.store.has_folded_history(&self.id).unwrap_or(false)
            || !predecessors(&self.store, &self.id).is_empty()
    }

    pub(crate) fn recall(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let (result, ok) = run(&self.store, &self.id, &call.args);
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::Role;

    fn compacted() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        for (seq, (role, text)) in [
            (Role::User, "pick the retry policy for uploads"),
            (Role::Assistant, "decision: exponential backoff, cap 30s"),
            (Role::User, "next"),
            (Role::Assistant, "ok"),
            (Role::User, "live user"),
            (Role::Assistant, "live reply"),
        ]
        .into_iter()
        .enumerate()
        {
            store
                .add_message(&sid, seq as i64, role, text, None)
                .unwrap();
        }
        (store, sid)
    }

    #[test]
    fn the_prompt_and_the_tool_tell_the_model_not_to_reconstruct_folded_detail() {
        assert!(SYSTEM_LINE.contains("instead of reconstructing it"));
        assert!(recall_spec().description.contains("do not guess it"));
    }

    #[test]
    fn a_session_without_a_compaction_has_nothing_to_recall() {
        let (store, sid) = compacted();
        let (text, ok) = run(
            &store,
            &sid,
            &serde_json::json!({"action": "search", "query": "backoff"}),
        );
        assert!(!ok);
        assert!(text.contains("nothing in this session has been folded"));
    }

    #[test]
    fn search_returns_the_folded_decision_and_read_shows_it_in_context() {
        let (store, sid) = compacted();
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        let (found, ok) = run(
            &store,
            &sid,
            &serde_json::json!({"action": "search", "query": "retry policy"}),
        );
        assert!(ok, "{found}");
        assert!(found.contains("seq 0 [user]"), "{found}");
        let (read, ok) = run(
            &store,
            &sid,
            &serde_json::json!({"action": "read", "from_seq": 0, "to_seq": 3}),
        );
        assert!(ok);
        assert!(read.contains("exponential backoff, cap 30s"));
        assert!(!read.contains("live reply"), "live rows are not recall's");
    }

    #[test]
    fn output_is_capped_and_says_where_to_continue() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        for seq in 0..30 {
            store
                .add_message(&sid, seq, Role::Assistant, &"w".repeat(2_000), None)
                .unwrap();
        }
        store.compact_session_store(&sid, "S", 1).unwrap();
        let (read, ok) = run(
            &store,
            &sid,
            &serde_json::json!({"action": "read", "from_seq": 0, "to_seq": 28}),
        );
        assert!(ok);
        assert!(
            read.chars().count() <= MAX_OUTPUT_CHARS,
            "{}",
            read.chars().count()
        );
        assert!(read.contains("continue with recall read from_seq="));
    }

    #[test]
    fn bad_arguments_are_errors_not_panics() {
        let (store, sid) = compacted();
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        for args in [
            serde_json::json!({}),
            serde_json::json!({"action": "search"}),
            serde_json::json!({"action": "read"}),
            serde_json::json!({"action": "read", "from_seq": 5, "to_seq": 1}),
            serde_json::json!({"action": "delete"}),
        ] {
            assert!(!run(&store, &sid, &args).1, "{args}");
        }
    }

    #[test]
    fn a_successor_recalls_its_predecessors_and_no_other_session() {
        let store = Store::open_in_memory().unwrap();
        let first = store.create_session("/x", "default").unwrap();
        let second = store.create_session("/x", "default").unwrap();
        let stranger = store.create_session("/x", "default").unwrap();
        store
            .add_message(
                &first,
                4,
                Role::Assistant,
                "decision: ship the zebra parser",
                None,
            )
            .unwrap();
        store
            .add_message(
                &stranger,
                0,
                Role::Assistant,
                "zebra parser in another project",
                None,
            )
            .unwrap();
        store
            .record_relay(&first, &second, "wedged", "", "")
            .unwrap();

        let (found, ok) = run(
            &store,
            &second,
            &serde_json::json!({"action": "search", "query": "zebra parser"}),
        );
        assert!(ok, "{found}");
        assert!(
            found.contains(&format!("earlier session {} seq 4", &first[..8])),
            "{found}"
        );
        assert!(!found.contains("another project"), "{found}");

        let (read, ok) = run(
            &store,
            &second,
            &serde_json::json!({"action": "read", "from_seq": 4, "to_seq": 4, "session": &first[..8]}),
        );
        assert!(ok, "{read}");
        assert!(read.contains("ship the zebra parser"));
        let (refused, ok) = run(
            &store,
            &second,
            &serde_json::json!({"action": "read", "from_seq": 0, "to_seq": 0, "session": &stranger[..8]}),
        );
        assert!(!ok, "{refused}");
        let (_, ok) = run(
            &store,
            &stranger,
            &serde_json::json!({"action": "search", "query": "zebra"}),
        );
        assert!(
            !ok,
            "a session with no ancestors and no fold still has nothing to recall"
        );
    }
}
