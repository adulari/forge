//! The mock provider's tool-heavy scenario, for soaking a daemon's memory over many long turns.
//! `mock:soak` makes `FORGE_MOCK_SOAK_CALLS` (default 20) distinct `read_file` calls per user turn,
//! each response carrying an opaque reasoning item like a Responses-API model's, then answers.

use forge_types::{new_id, Message, Role, ToolCall};
use serde_json::{json, Value};

const DEFAULT_CALLS: usize = 20;
const DEFAULT_REASONING_BYTES: usize = 6_000;
const FILE_LINES: usize = 600;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn wants(lowercased_prompt: &str) -> bool {
    lowercased_prompt.contains("mock:soak")
}

pub(crate) fn in_scenario(messages: &[Message]) -> bool {
    messages
        .iter()
        .any(|m| m.role == Role::User && wants(&m.content.to_lowercase()))
}

/// The call to make next, or `None` once this turn has made its quota. Each soak turn reads its
/// own file (`soak_<turn>.txt`, wrapping at `FORGE_MOCK_SOAK_FILES`) in disjoint slices, so no
/// progress guard sees repeated content. A harness may append its own user-role messages
/// mid-turn, so the turn is the one that began at the last prompt asking for the scenario.
pub(crate) fn next_call(messages: &[Message]) -> Option<(ToolCall, Vec<Value>)> {
    let turn_start = messages
        .iter()
        .rposition(|m| m.role == Role::User && wants(&m.content.to_lowercase()))?;
    let turn = messages
        .iter()
        .filter(|m| m.role == Role::User && wants(&m.content.to_lowercase()))
        .count()
        - 1;
    let made = messages[turn_start + 1..]
        .iter()
        .filter(|m| m.role == Role::Tool)
        .count();
    if made >= env_usize("FORGE_MOCK_SOAK_CALLS", DEFAULT_CALLS) {
        return None;
    }
    let call = ToolCall {
        id: new_id(),
        name: "read_file".to_string(),
        args: json!({
            "path": format!("soak_{}.txt", turn % env_usize("FORGE_MOCK_SOAK_FILES", 64)),
            "offset": 1 + made * FILE_LINES,
            "limit": FILE_LINES,
        }),
    };
    let blob = "A".repeat(env_usize(
        "FORGE_MOCK_SOAK_REASONING_BYTES",
        DEFAULT_REASONING_BYTES,
    ));
    let item = json!({
        "type": "reasoning",
        "summary": [],
        "encrypted_content": format!("{made}:{blob}"),
    });
    Some((call, vec![item]))
}

/// Repeat the allocation pattern of a Responses WebSocket request over the whole transcript: build
/// the body, stamp the frame, compare against the previous request, serialize. The mock has no
/// wire, so without this the soak never exercises the large short-lived buffers that dominate a
/// real provider's heap churn. Enabled by `FORGE_MOCK_SOAK_WIRE=1`.
pub(crate) fn emulate_wire(model: &str, messages: &[Message]) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<Value>> = Mutex::new(None);
    if std::env::var_os("FORGE_MOCK_SOAK_WIRE").is_none() {
        return;
    }
    let body = crate::oauth_responses::build_responses_request(
        model,
        messages,
        &[],
        &crate::CompletionOptions::default(),
        0,
    );
    let mut frame = body.clone();
    frame["type"] = json!("response.create");
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(previous) = last.as_ref() {
        let _ = previous.clone() == body.clone();
    }
    let _wire = serde_json::to_string(&frame).unwrap_or_default();
    *last = Some(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_distinct_and_stop_at_the_quota() {
        let mut msgs = vec![Message::user("mock:soak go")];
        let mut seen = std::collections::HashSet::new();
        while let Some((call, items)) = next_call(&msgs) {
            assert_eq!(items.len(), 1);
            assert!(seen.insert(call.args.to_string()), "args repeat");
            msgs.push(Message::assistant_tool_calls("", vec![call]));
            msgs.push(Message::new(Role::Tool, "out"));
        }
        assert_eq!(seen.len(), DEFAULT_CALLS);
        msgs.push(Message::user("mock:soak again"));
        assert!(next_call(&msgs).is_some(), "a new user turn starts afresh");
    }
}
