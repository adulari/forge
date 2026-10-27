//! Surface-independent session transcript replay.
//!
//! This module owns the single mapping from persisted messages to presenter
//! replay items, preserving event ordering and compaction visibility.

use forge_types::{Message, ReplayItem, Role};

/// Render a sequence of messages into surface-independent [`ReplayItem`](forge_types::ReplayItem)s — user prompts,
/// assistant text, tool calls (with args), tool results (matched to their call's name via
/// `tool_call_id`), and the compaction marker. Shared by the model-facing replay
/// ([`Session::replay_items`]) and the full-history replay ([`Session::replay_items_full`]).
pub(crate) fn messages_to_replay_items(msgs: &[Message]) -> Vec<forge_types::ReplayItem> {
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for m in msgs {
        if !m.visibility.is_user_visible() {
            continue;
        }
        match m.role {
            Role::User => {
                if m.kind != forge_types::MessageKind::Nudge && !m.content.trim().is_empty() {
                    out.push(ReplayItem::User(m.content.clone()));
                }
            }
            Role::Assistant => {
                if !m.content.trim().is_empty() {
                    out.push(ReplayItem::Assistant(m.content.clone()));
                }
                for tc in &m.tool_calls {
                    names.insert(tc.id.clone(), tc.name.clone());
                    let args = serde_json::to_string(&tc.args).unwrap_or_default();
                    out.push(ReplayItem::Tool {
                        name: tc.name.clone(),
                        args,
                    });
                }
            }
            Role::Tool => {
                let name = m
                    .tool_call_id
                    .as_ref()
                    .and_then(|id| names.get(id).cloned())
                    .unwrap_or_else(|| "tool".to_string());
                let summary = m.content.lines().next().unwrap_or("").to_string();
                // The success flag isn't persisted; an error result conventionally starts with
                // "error". Good enough to color the replayed line.
                let ok = !summary.trim_start().to_lowercase().starts_with("error");
                out.push(ReplayItem::ToolResult { name, ok, summary });
            }
            Role::System => {
                // Only the compaction marker represents real prior conversation; other System
                // messages (per-turn guidance/project prompt) are machinery — skip them.
                if m.content.starts_with("[Earlier conversation summarized") {
                    let first = m.content.lines().next().unwrap_or("").to_string();
                    out.push(ReplayItem::Note(first.trim_matches(['[', ']']).to_string()));
                }
            }
        }
    }
    out
}

/// A session's full persisted conversation as replay items, read straight from the store. This is
/// how any surface shows a session it does not own — a subagent's child session while it runs or
/// long after — with the same mapping a resumed session uses.
pub fn stored_replay_items(
    store: &forge_store::Store,
    session_id: &str,
) -> Result<Vec<ReplayItem>, forge_store::StoreError> {
    let msgs: Vec<Message> = store
        .load_all_messages(session_id)?
        .into_iter()
        .map(|m| Message {
            reasoning: None,
            provider_items: None,
            role: m.role,
            content: m.content,
            tool_calls: m.tool_calls,
            tool_call_id: m.tool_call_id,
            images: Vec::new(),
            visibility: m.visibility,
            kind: m.kind,
        })
        .collect();
    Ok(messages_to_replay_items(&msgs))
}

#[cfg(test)]
mod nudge_tests {
    use super::*;

    #[test]
    fn harness_nudges_are_not_replayed_as_user_prompts() {
        let mut nudge = Message::new(Role::User, "Implement the fix now.");
        nudge.kind = forge_types::MessageKind::Nudge;
        let real = Message::new(Role::User, "hello");
        let items = messages_to_replay_items(&[nudge, real]);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn a_child_session_replays_from_the_store_and_grows_as_it_runs() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let parent = store.create_session(".", "default").unwrap();
        let child = store.create_child_session(".", "default", &parent).unwrap();
        store
            .add_message(&child, 0, Role::User, "scan the repo", None)
            .unwrap();
        let items = stored_replay_items(&store, &child).unwrap();
        assert!(matches!(&items[..], [ReplayItem::User(t)] if t == "scan the repo"));

        store
            .add_message(&child, 1, Role::Assistant, "found it", None)
            .unwrap();
        let items = stored_replay_items(&store, &child).unwrap();
        assert!(matches!(items.last(), Some(ReplayItem::Assistant(t)) if t == "found it"));
        assert!(stored_replay_items(&store, &parent).unwrap().is_empty());
    }
}
