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
