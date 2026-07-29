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
                if !m.content.trim().is_empty() {
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
