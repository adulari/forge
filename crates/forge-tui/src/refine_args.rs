//! `/refine` argument parsing (docs/features/continual-harness.md). Split from `commands.rs` to
//! keep the command registry file within its architecture-size budget.

use crate::commands::CommandAction;

/// `/refine` sub-actions (docs/features/continual-harness.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineAction {
    /// `/refine [--global] [instructions]` — propose and apply one refinement batch now. Targets
    /// the session scope unless `global` is set.
    Run {
        instructions: Option<String>,
        global: bool,
    },
    /// `/refine status` — list harness entries in scope plus recent refinement history.
    Status,
    /// `/refine rollback <id>` — invert a past refinement batch (id or a unique id prefix).
    Rollback(String),
}

/// Extract a comma-separated lens list from `--flag <value>` in a raw arg string.
/// `/assay --only dead-weight,unsafe` → `extract_flag(arg, "--only")` → `["dead-weight", "unsafe"]`
pub(crate) fn extract_flag(arg: &str, flag: &str) -> Vec<String> {
    let tokens: Vec<&str> = arg.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        if *tok == flag {
            if let Some(val) = tokens.get(i + 1) {
                if !val.starts_with('-') {
                    return val
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

/// Check whether a boolean flag (no value) is present in `arg`.
pub(crate) fn has_flag(arg: &str, flag: &str) -> bool {
    arg.split_whitespace().any(|t| t == flag)
}

/// `/refine [--global] [instructions]` | `/refine rollback <id>` | `/refine status`
pub(crate) fn refine_action(arg: &str) -> CommandAction {
    let trimmed = arg.trim();
    let action = if trimmed.eq_ignore_ascii_case("status") {
        RefineAction::Status
    } else if let Some(rest) = trimmed
        .strip_prefix("rollback ")
        .or_else(|| trimmed.strip_prefix("rollback"))
    {
        RefineAction::Rollback(rest.trim().to_string())
    } else {
        let global = trimmed.split_whitespace().any(|t| t == "--global");
        let instructions: String = trimmed
            .split_whitespace()
            .filter(|t| !t.eq_ignore_ascii_case("--global"))
            .collect::<Vec<_>>()
            .join(" ");
        RefineAction::Run {
            instructions: (!instructions.is_empty()).then_some(instructions),
            global,
        }
    };
    CommandAction::Refine(action)
}
