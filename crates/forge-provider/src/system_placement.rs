//! Where mid-transcript system messages go on the wire. Split out of `genai_provider.rs` to keep
//! that file under the architecture size guard.

use forge_types::{Message, Role};

/// Keep the leading system block at the front and turn every later system message into an
/// in-place `<system-reminder>` user message.
///
/// Forge injects system content mid-transcript — lattice symbols, command guidance, and above all
/// the continuation nudges ("tasks are NOT yet Done", "call a tool or finish"). A strict chat
/// template does not allow that: Qwen3.8's raises
///
///   Jinja Exception: System message must be at the beginning.
///
/// which llama.cpp surfaces as `Unable to generate parser for this template`, failing EVERY
/// tool-bearing turn on an otherwise healthy local model. So a system message cannot stay where it
/// was put. This used to hoist them ALL to the front, which fixed the template and broke two other
/// things, measured on the live store (60 days, requests over 20k input tokens):
///
///  - the provider's prefix cache. Inserting text ahead of the whole conversation invalidates
///    everything after it: the request after an injection read 9% of its input from cache on
///    Bedrock Kimi (98% otherwise), 46% on Kimi Code (96%) and 40% on Muse (97%).
///  - the instruction itself. A nudge hoisted to the top leaves the request ending on the model's
///    own previous reply, so the thing that was supposed to re-drive it is buried and the model
///    answers an empty message. Most empty replies in the store follow one of these nudges.
///
/// A user message in place fixes both. A reminder that would land between an assistant's tool
/// calls and their results waits until the last result is in, because chat APIs require the
/// results to follow the calls directly.
pub(crate) fn place_system_messages(messages: &[Message]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    let mut deferred: Vec<Message> = Vec::new();
    let mut pending_calls: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut leading = true;
    for m in messages {
        if matches!(m.role, Role::System) {
            if leading {
                out.push(m.clone());
                continue;
            }
            let reminder = Message::user(format!(
                "<system-reminder>\n{}\n</system-reminder>",
                m.content
            ));
            if pending_calls.is_empty() {
                out.push(reminder);
            } else {
                deferred.push(reminder);
            }
            continue;
        }
        leading = false;
        match m.role {
            Role::Assistant => pending_calls.extend(m.tool_calls.iter().map(|c| c.id.as_str())),
            Role::Tool => {
                if let Some(id) = m.tool_call_id.as_deref() {
                    pending_calls.remove(id);
                }
            }
            _ => {}
        }
        out.push(m.clone());
        if pending_calls.is_empty() {
            out.append(&mut deferred);
        }
    }
    out.append(&mut deferred);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(messages: &[Message]) -> Vec<(String, String)> {
        messages
            .iter()
            .map(|m| (format!("{:?}", m.role), m.content.clone()))
            .collect()
    }

    /// Qwen3.8's chat template raises "System message must be at the beginning", which llama.cpp
    /// reports as `Unable to generate parser for this template` — failing EVERY tool-bearing turn.
    /// Only the leading system block may stay a system message.
    #[test]
    fn a_system_message_after_the_conversation_starts_stops_being_one() {
        let msgs = vec![
            Message::system("first system"),
            Message::system("env"),
            Message::user("hello"),
            Message::system("injected later"),
            Message::user("second turn"),
        ];
        let placed = place_system_messages(&msgs);
        let roles: Vec<_> = placed.iter().map(|m| format!("{:?}", m.role)).collect();
        assert_eq!(roles, ["System", "System", "User", "User", "User"]);
        assert_eq!(placed[2].content, "hello", "order is preserved");
        assert!(placed[3].content.contains("injected later"));
        assert!(placed[3].content.starts_with("<system-reminder>"));
        assert_eq!(placed[4].content, "second turn");
    }

    /// The measured cause of the cache collapse: a nudge hoisted to the front changed every byte
    /// after it. In place, the transcript up to the nudge is byte-identical to the last request's.
    #[test]
    fn a_nudge_leaves_the_prefix_before_it_untouched_and_ends_the_request() {
        let before = vec![
            Message::system("sys"),
            Message::user("task"),
            Message::assistant("done"),
        ];
        let mut after = before.clone();
        after.push(Message::system(
            "You ended your reply, but tasks are NOT yet Done.",
        ));
        let (a, b) = (
            place_system_messages(&before),
            place_system_messages(&after),
        );
        assert_eq!(texts(&a), texts(&b[..a.len()]), "prefix must not move");
        assert_eq!(b.len(), a.len() + 1);
        let last = b.last().unwrap();
        assert!(
            matches!(last.role, Role::User),
            "the request must end on the nudge"
        );
        assert!(last.content.contains("NOT yet Done"));
    }

    /// Chat APIs need a tool call's results directly after it, so a reminder injected while calls
    /// are outstanding waits until the last result is in.
    #[test]
    fn a_reminder_never_splits_a_tool_call_from_its_results() {
        let call = |id: &str| forge_types::ToolCall {
            id: id.to_string(),
            name: "shell".to_string(),
            args: serde_json::json!({}),
        };
        let msgs = vec![
            Message::system("sys"),
            Message::user("go"),
            Message::assistant_tool_calls("", vec![call("a"), call("b")]),
            Message::tool_result("a", "out a"),
            Message::system("hint"),
            Message::tool_result("b", "out b"),
        ];
        let roles: Vec<_> = place_system_messages(&msgs)
            .iter()
            .map(|m| format!("{:?}", m.role))
            .collect();
        assert_eq!(
            roles,
            ["System", "User", "Assistant", "Tool", "Tool", "User"],
            "got {roles:?}"
        );
    }

    /// A transcript that is already legal must come through untouched.
    #[test]
    fn a_transcript_without_stray_system_messages_is_unchanged() {
        let msgs = vec![
            Message::system("only system"),
            Message::user("a"),
            Message::user("b"),
        ];
        let got: Vec<String> = place_system_messages(&msgs)
            .into_iter()
            .map(|m| m.content)
            .collect();
        assert_eq!(got, ["only system", "a", "b"]);
    }
}
