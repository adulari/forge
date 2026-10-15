//! What the summarizer reads: the folded range at full fidelity, and which of its user messages
//! are the user's own words.

use forge_types::{Message, Role};

use crate::compaction_summary::{
    head_tail, split_verbatim, VerbatimEntry, INPUT_MESSAGE_MAX_CHARS, INPUT_TOOL_RESULT_MAX_CHARS,
    SUMMARY_PREFIX,
};
use crate::empty_reply::EMPTY_REPLY_NUDGE;

/// The folded range as the summarizer and the verbatim section need it.
#[derive(Debug, Default)]
pub(crate) struct FoldedInput {
    /// Model-visible messages in order. Tool results carry the stored content instead of the
    /// pruned in-memory head, and an earlier summary comes without its verbatim section (the
    /// harness re-appends that itself).
    pub messages: Vec<Message>,
    /// The user's own messages, oldest first: no harness nudges, no UI-only rows. Steers are
    /// typed by the user and stay.
    pub user_messages: Vec<String>,
    /// Entries of the verbatim section the previous summary carried.
    pub previous_verbatim: Vec<VerbatimEntry>,
}

/// User-role text the harness writes itself, recognized by its wording. Messages carry a
/// [`forge_types::MessageKind`] that says so exactly; this catches rows written before the kind
/// existed.
fn is_harness_text(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with(EMPTY_REPLY_NUDGE)
        || text.starts_with(crate::nudge_policy::CONTINUE_NUDGE)
        || text.starts_with("Auto-fix: the following checks failed")
        || text.starts_with("[stop hook] ")
        || text.starts_with("[background job ")
}

/// Pair each in-memory message of `older` with what the store holds for it.
///
/// `tool_results` is the store's active tool rows, oldest first. They are matched to in-memory
/// results by call id walking forward, so a provider that reuses an id across turns still pairs
/// each result with its own row, and a result the store never saw keeps its in-memory text.
pub(crate) fn fold_inputs(older: &[Message], tool_results: &[(String, String)]) -> FoldedInput {
    let mut out = FoldedInput::default();
    let mut cursor = 0usize;
    for message in older.iter().filter(|m| m.visibility.is_llm()) {
        let mut message = message.clone();
        match message.role {
            Role::Tool => {
                let paired = message.tool_call_id.as_deref().and_then(|id| {
                    tool_results[cursor..]
                        .iter()
                        .position(|(row_id, _)| row_id == id)
                        .map(|offset| cursor + offset)
                });
                if let Some(index) = paired {
                    cursor = index + 1;
                    if tool_results[index].1.len() > message.content.len() {
                        message.content = tool_results[index].1.clone();
                    }
                }
            }
            Role::System if message.content.starts_with(SUMMARY_PREFIX) => {
                let (body, entries) = split_verbatim(&message.content);
                if !entries.is_empty() {
                    out.previous_verbatim = entries;
                }
                message.content = body.to_string();
            }
            Role::User
                if message.is_real_user()
                    && !message.content.trim().is_empty()
                    && !is_harness_text(&message.content) =>
            {
                out.user_messages.push(message.content.clone());
            }
            _ => {}
        }
        out.messages.push(message);
    }
    out
}

/// One message as the summarizer reads it: `role: content`, with the assistant's tool calls (the
/// only record of what a turn did) and a cap so one giant result cannot dominate the input.
pub(crate) fn render_entry(message: &Message) -> String {
    let content = match message.role {
        Role::Tool | Role::System if !message.content.starts_with(SUMMARY_PREFIX) => {
            head_tail(&message.content, INPUT_TOOL_RESULT_MAX_CHARS)
        }
        _ if message.content.starts_with(SUMMARY_PREFIX) => message.content.clone(),
        _ => head_tail(&message.content, INPUT_MESSAGE_MAX_CHARS),
    };
    let mut line = format!("{}: {content}", message.role.as_str());
    for call in &message.tool_calls {
        line.push_str(&format!("\n  [call {} {}]", call.name, call.args));
    }
    line
}

/// What a compaction that no model could summarize says in place of one: how much was dropped.
/// The user's own words follow it in the verbatim section.
pub(crate) fn trim_notice(messages: &[Message]) -> String {
    let tool_results = messages.iter().filter(|m| m.role == Role::Tool).count();
    let steps = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    format!(
        "No model could summarize this stretch, so it was trimmed instead: {steps} assistant \
         steps and {tool_results} tool results were dropped. What the user asked for is in the \
         verbatim section below."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction_summary::render_verbatim_block;
    use forge_types::ToolCall;

    fn call(id: &str) -> Message {
        Message::assistant_tool_calls(
            "",
            vec![ToolCall {
                id: id.into(),
                name: "read_file".into(),
                args: serde_json::json!({"path": id}),
            }],
        )
    }

    #[test]
    fn the_stored_tool_result_replaces_the_pruned_in_memory_head() {
        let full = "line\n".repeat(5_000);
        let older = vec![
            Message::user("task"),
            call("c1"),
            Message::tool_result("c1", "line\n…[output trimmed]"),
        ];
        let rows = vec![("c1".to_string(), full.clone())];
        let folded = fold_inputs(&older, &rows);
        assert_eq!(folded.messages[2].content, full);
    }

    #[test]
    fn a_reused_call_id_pairs_each_result_with_its_own_row_in_order() {
        let older = vec![
            call("c"),
            Message::tool_result("c", "a"),
            call("c"),
            Message::tool_result("c", "b"),
        ];
        let rows = vec![
            ("c".to_string(), "first full".to_string()),
            ("c".to_string(), "second full".to_string()),
        ];
        let folded = fold_inputs(&older, &rows);
        assert_eq!(folded.messages[1].content, "first full");
        assert_eq!(folded.messages[3].content, "second full");
    }

    #[test]
    fn a_result_the_store_never_saw_keeps_its_in_memory_text() {
        let older = vec![call("x"), Message::tool_result("x", "only in memory")];
        let folded = fold_inputs(&older, &[]);
        assert_eq!(folded.messages[1].content, "only in memory");
    }

    #[test]
    fn harness_nudges_are_not_the_users_words_but_steers_are() {
        let older = vec![
            Message::user("fix the login bug"),
            Message::user(EMPTY_REPLY_NUDGE),
            Message::user("some custom gate message").with_kind(forge_types::MessageKind::Nudge),
            Message::user("Auto-fix: the following checks failed, fix them:\n\n[lint]"),
            Message::user(crate::nudge_policy::CONTINUE_NUDGE),
            Message::user("[stop hook] tests must pass"),
            Message::user("[background job 3 `cargo test` exited 0]"),
            Message::user("actually use the v2 endpoint"),
            Message::system("note").ui_only(),
            Message::user("hidden").ui_only(),
        ];
        let folded = fold_inputs(&older, &[]);
        assert_eq!(
            folded.user_messages,
            ["fix the login bug", "actually use the v2 endpoint"]
        );
        assert!(
            folded.messages.iter().all(|m| m.visibility.is_llm()),
            "UI-only rows never reach the summarizer"
        );
        assert_eq!(
            folded.messages.len(),
            8,
            "nudges still reach the summarizer"
        );
    }

    #[test]
    fn an_earlier_summary_is_fed_without_its_verbatim_section_and_hands_it_forward() {
        let block = render_verbatim_block(&[], &["build the parser".into()]);
        let summary = format!("{SUMMARY_PREFIX}\n1. Request\nparser\n\n{block}");
        let older = vec![Message::system(summary), Message::user("add tests")];
        let folded = fold_inputs(&older, &[]);
        assert!(folded.messages[0].content.ends_with("parser"));
        assert!(!folded.messages[0].content.contains("verbatim"));
        assert_eq!(folded.previous_verbatim.len(), 1);
        assert_eq!(folded.previous_verbatim[0].text, "build the parser");
        assert_eq!(folded.user_messages, ["add tests"]);
    }

    #[test]
    fn a_giant_tool_result_is_capped_in_the_summarizer_input() {
        let message = Message::tool_result("c", "q".repeat(200_000));
        let entry = render_entry(&message);
        assert!(entry.chars().count() < INPUT_TOOL_RESULT_MAX_CHARS + 200);
        assert!(entry.contains("omitted from the middle"));
    }

    #[test]
    fn an_earlier_summary_is_never_capped() {
        let summary = format!("{SUMMARY_PREFIX}\n{}", "detail ".repeat(20_000));
        let entry = render_entry(&Message::system(&summary));
        assert!(entry.contains(&"detail ".repeat(20_000)));
    }

    #[test]
    fn rendered_entries_carry_the_assistants_tool_calls() {
        let entry = render_entry(&call("src/lib.rs"));
        assert!(entry.contains("[call read_file") && entry.contains("src/lib.rs"));
    }

    #[test]
    fn the_trim_notice_counts_what_was_dropped() {
        let older = vec![
            Message::user("x"),
            call("c"),
            Message::tool_result("c", "r"),
        ];
        let notice = trim_notice(&older);
        assert!(notice.contains("1 assistant steps") && notice.contains("1 tool results"));
    }
}
