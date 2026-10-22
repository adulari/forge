//! The mock provider's tool-heavy scenario, for exercising how the TUI folds tool activity.
//! `mock:tools` runs a short agentic turn: explore (list, find, search, read), run checks (one of
//! which fails), write and edit a file, then verify it, with prose between some steps and none
//! between others, and a reasoning delta before the steps that have no prose.
//!
//! `FORGE_MOCK_TOOLS_SLEEP` (seconds) makes one of the checks take that long, so a capture can
//! catch a group while it is still running.

use forge_types::{new_id, Message, Role, ToolCall};
use serde_json::{json, Value};

const TRIGGER: &str = "mock:tools";

pub(crate) fn in_scenario(lowercased_prompt: &str) -> bool {
    lowercased_prompt.contains(TRIGGER)
}

/// One step of the scenario: what the model says, what it thinks first, and the calls it makes.
pub(crate) struct Step {
    pub(crate) content: &'static str,
    pub(crate) reasoning: Option<&'static str>,
    pub(crate) calls: Vec<ToolCall>,
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: new_id(),
        name: name.to_string(),
        args,
    }
}

fn check_command() -> String {
    let secs = std::env::var("FORGE_MOCK_TOOLS_SLEEP")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0);
    match secs {
        Some(s) => format!("sleep {s}; echo '3 checks passed'"),
        None => "echo '3 checks passed'".to_string(),
    }
}

const SUMMARY: &str = "# Mock summary\n\nThe project has one source file.\n\n## Checks\n\n- build: ok\n- probe: failed (expected)\n- checks: 3 passed\n\n## Next\n\nNothing else to do.\n";

/// The step to take next, or `None` once every step has run and the turn should answer.
pub(crate) fn next_step(messages: &[Message]) -> Option<Step> {
    let start = messages
        .iter()
        .rposition(|m| m.role == Role::User && in_scenario(&m.content.to_lowercase()))?;
    let done = messages[start + 1..]
        .iter()
        .filter(|m| m.role == Role::Assistant && !m.tool_calls.is_empty())
        .count();
    let step = match done {
        0 => Step {
            content: "I'll get my bearings in the project first.",
            reasoning: None,
            calls: vec![
                call("list_dir", json!({ "path": "." })),
                call("glob", json!({ "pattern": "**/*.rs" })),
                call("search", json!({ "query": "fn main" })),
                call("read_file", json!({ "path": "sample.rs" })),
            ],
        },
        1 => Step {
            content: "",
            reasoning: Some("The layout is small. Run the checks next."),
            calls: vec![
                call("shell", json!({ "command": "echo 'build ok'" })),
                call("shell", json!({ "command": check_command() })),
                call(
                    "shell",
                    json!({ "command": "echo 'probe: missing fixture' >&2; exit 3" }),
                ),
            ],
        },
        2 => Step {
            content: "The checks pass apart from one probe that was expected to fail. Writing the summary and fixing the constant now.",
            reasoning: None,
            calls: vec![
                call(
                    "write_file",
                    json!({ "path": "mock-summary.md", "content": SUMMARY }),
                ),
                call(
                    "edit_file",
                    json!({ "path": "sample.rs", "old": "let x = 1;", "new": "let x = 2;" }),
                ),
            ],
        },
        3 => Step {
            content: "",
            reasoning: Some("Verify the summary landed."),
            calls: vec![
                call("read_file", json!({ "path": "mock-summary.md" })),
                call("shell", json!({ "command": "wc -l mock-summary.md" })),
            ],
        },
        _ => return None,
    };
    Some(step)
}

/// The closing answer once every step has run.
pub(crate) const ANSWER: &str = "Done. I explored the layout, ran the checks (one probe failed as expected), wrote `mock-summary.md` and bumped the constant in `sample.rs`.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_walks_four_steps_then_answers() {
        let mut messages = vec![Message::user("mock:tools tour the project")];
        let mut names = Vec::new();
        while let Some(step) = next_step(&messages) {
            names.push(
                step.calls
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>(),
            );
            messages.push(Message::assistant_tool_calls(step.content, step.calls));
            messages.push(Message::new(Role::Tool, "ok"));
        }
        assert_eq!(names.len(), 4);
        assert_eq!(names[1], ["shell", "shell", "shell"]);
        assert_eq!(names[2], ["write_file", "edit_file"]);
    }

    #[test]
    fn a_later_prompt_starts_the_scenario_over() {
        let messages = vec![
            Message::user("mock:tools first"),
            Message::assistant_tool_calls("x", vec![call("read_file", json!({}))]),
            Message::new(Role::Tool, "ok"),
            Message::user("mock:tools again"),
        ];
        let step = next_step(&messages).expect("a fresh turn starts at step one");
        assert_eq!(step.calls[0].name, "list_dir");
    }
}
