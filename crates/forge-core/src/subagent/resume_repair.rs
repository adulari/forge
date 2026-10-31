//! Closing a child's transcript before it is resumed.
//!
//! A child stopped mid-tool (a steer, a cancel, a crash) has saved its assistant message with
//! `tool_calls` but never the matching tool results. Providers reject such a transcript, so a
//! resume must first answer each dangling call.

use forge_store::Store;
use forge_types::{Message, Role};

use crate::CoreError;

/// What a closed-off call's result says.
const INTERRUPTED: &str = "[interrupted before this tool finished; no result was recorded]";

/// Append a synthetic tool result for every call in the last assistant message of `child_id`
/// that has none. Returns how many were added.
pub(super) fn close_dangling_tool_calls(store: &Store, child_id: &str) -> Result<usize, CoreError> {
    let stored = store.load_messages(child_id)?;
    let mut pending: Vec<String> = Vec::new();
    for m in &stored {
        match m.role {
            Role::Assistant => pending = m.tool_calls.iter().map(|c| c.id.clone()).collect(),
            Role::Tool => {
                if let Some(id) = m.tool_call_id.as_deref() {
                    pending.retain(|p| p != id);
                }
            }
            _ => {}
        }
    }
    let first = store.next_message_seq(child_id)?;
    for (seq, id) in (first..).zip(&pending) {
        store.add_message_full(child_id, seq, Role::Tool, INTERRUPTED, None, &[], Some(id))?;
    }
    Ok(pending.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::ToolCall;

    #[test]
    fn unanswered_calls_get_a_result_and_answered_ones_are_left_alone() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "default").unwrap();
        let call = |n: &str| ToolCall {
            id: n.into(),
            name: "shell".into(),
            args: serde_json::json!({}),
        };
        store.add_message(&id, 0, Role::User, "go", None).unwrap();
        store
            .add_message_full(
                &id,
                1,
                Role::Assistant,
                "",
                None,
                &[call("a"), call("b")],
                None,
            )
            .unwrap();
        store
            .add_message_full(&id, 2, Role::Tool, "done", None, &[], Some("a"))
            .unwrap();
        assert_eq!(close_dangling_tool_calls(&store, &id).unwrap(), 1);
        let after = store.load_messages(&id).unwrap();
        let last = after.last().unwrap();
        assert_eq!(last.role, Role::Tool);
        assert_eq!(last.tool_call_id.as_deref(), Some("b"));
        assert!(last.content.contains("interrupted"));
        assert_eq!(
            close_dangling_tool_calls(&store, &id).unwrap(),
            0,
            "idempotent"
        );
    }
}

/// A saved message as the provider-facing message it was.
pub(super) fn replay_message(m: forge_store::StoredMessage) -> Message {
    let visibility = m.visibility;
    let mut msg = match m.role {
        Role::System => Message::system(&m.content),
        Role::User => Message::user(&m.content),
        Role::Assistant => Message::assistant_tool_calls(&m.content, m.tool_calls),
        Role::Tool => Message::tool_result(m.tool_call_id.as_deref().unwrap_or(""), m.content),
    };
    msg.visibility = visibility;
    msg
}
