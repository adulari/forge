//! Arg parsing for `/workflow` (alias `/wf`), split out of `commands.rs` (at its CI size ratchet).

use super::WorkflowAction;

/// `/workflow` lists, `/workflow run <name> [args]` runs a saved one, anything else is an inline
/// workflow description.
pub(crate) fn workflow_action(arg: &str) -> WorkflowAction {
    let trimmed = arg.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("list") {
        WorkflowAction::List
    } else if let Some(rest) = trimmed
        .strip_prefix("run ")
        .or_else(|| trimmed.strip_prefix("run"))
    {
        let mut parts = rest.trim().splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").trim().to_string();
        let args = parts.next().unwrap_or("").trim().to_string();
        WorkflowAction::RunSaved { name, args }
    } else {
        WorkflowAction::Run(trimmed.to_string())
    }
}
