//! The offline mock's answer to the `/goal` progress evaluator: a verdict read straight off the
//! task plan in the evaluator's input, so `--mock` goals end when their plan is done.

use forge_types::{Message, Role};

/// Must match `forge_core::goal_evaluator::GOAL_EVALUATOR_MARKER`.
const MARKER: &str = "You are the goal progress evaluator";

pub(crate) fn is_goal_evaluation(messages: &[Message]) -> bool {
    messages
        .iter()
        .any(|m| m.role == Role::System && m.content.starts_with(MARKER))
}

/// Must match `forge_core::goal_contract::GOAL_CONTRACT_MARKER`.
const CONTRACT_MARKER: &str = "You are the goal contract drafter";

pub(crate) fn is_contract_draft(messages: &[Message]) -> bool {
    messages
        .iter()
        .any(|m| m.role == Role::System && m.content.starts_with(CONTRACT_MARKER))
}

/// Must match `forge_core::goal_mission::GOAL_MISSION_MARKER`.
const MISSION_MARKER: &str = "You are the goal mission planner";

pub(crate) fn is_mission_draft(messages: &[Message]) -> bool {
    messages
        .iter()
        .any(|m| m.role == Role::System && m.content.starts_with(MISSION_MARKER))
}

/// A two-milestone mission every workspace satisfies, so an offline `--mission` goal can be
/// approved and run.
pub(crate) fn mission() -> String {
    serde_json::json!({"milestones": [
        {"title": "the workspace is in place", "assertions": [
            {"assertion": "the workspace directory exists", "check": "test -d .", "expect": "exit 0"},
            {"assertion": "the workspace can be listed", "check": "ls", "expect": "exit 0"},
        ]},
        {"title": "the workspace is usable", "assertions": [
            {"assertion": "the working directory resolves", "check": "pwd", "expect": "prints a path"},
            {"assertion": "the workspace is readable", "check": "test -r .", "expect": "exit 0"},
        ]},
    ]})
    .to_string()
}

/// A contract every workspace satisfies, so an offline `--contract` goal can be approved and run.
pub(crate) fn contract() -> String {
    serde_json::json!({"assertions": [
        {"assertion": "the workspace directory exists", "check": "test -d .", "expect": "exit 0"},
        {"assertion": "the workspace can be listed", "check": "ls", "expect": "exit 0"},
        {"assertion": "the working directory resolves", "check": "pwd", "expect": "prints a path"},
    ]})
    .to_string()
}

/// Achieved when the plan has tasks and all are done; otherwise the open tasks are what remains.
pub(crate) fn verdict(messages: &[Message]) -> String {
    let input = messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .unwrap_or("");
    let plan = input
        .split("Task plan:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .unwrap_or("");
    let tasks: Vec<&str> = plan.lines().filter(|l| l.starts_with("- [")).collect();
    let open: Vec<String> = tasks
        .iter()
        .filter(|l| !l.starts_with("- [done]"))
        .filter_map(|l| l.split_once("] ").map(|(_, title)| title.to_string()))
        .collect();
    let achieved = !tasks.is_empty() && open.is_empty();
    let remaining = if achieved || !open.is_empty() {
        open
    } else {
        vec!["record a task plan with update_tasks".to_string()]
    };
    serde_json::json!({
        "achieved": achieved,
        "next_step": remaining.first().cloned().unwrap_or_default(),
        "remaining": remaining,
        "confidence": 0.9,
        "reason": if achieved { "every planned task is done" } else { "planned tasks remain" },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(plan: &str) -> Vec<Message> {
        vec![
            Message::system(format!("{MARKER} for an autonomous coding agent.")),
            Message::user(format!(
                "GOAL:\nx\n\nTask plan:\n{plan}\n\nAgent's final message"
            )),
        ]
    }

    #[test]
    fn the_offline_contract_parses_as_three_assertions() {
        let drafter = vec![Message::system(format!("{CONTRACT_MARKER} for an agent."))];
        assert!(is_contract_draft(&drafter));
        assert!(!is_contract_draft(&input("")));
        assert_eq!(contract().matches("\"check\"").count(), 3);
    }

    #[test]
    fn open_tasks_remain_and_a_done_plan_is_achieved() {
        let open = verdict(&input("- [done] a\n- [pending] b"));
        assert!(
            open.contains("\"achieved\":false") && open.contains("\"b\""),
            "{open}"
        );
        let done = verdict(&input("- [done] a\n- [done] b"));
        assert!(done.contains("\"achieved\":true"), "{done}");
        assert!(is_goal_evaluation(&input("")));
    }
}
