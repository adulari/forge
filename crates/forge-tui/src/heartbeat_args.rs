//! `/heartbeat` argument parsing (docs/features/session-heartbeats.md), plus the shared raw-flag
//! helpers. Split from `commands.rs` to keep the command registry within its architecture-size
//! budget.

use crate::commands::CommandAction;

/// `/heartbeat` sub-actions (docs/features/session-heartbeats.md): the user's own recurring
/// re-entry prompt for this session — at most one, replaced (not stacked) by a new `every`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeartbeatAction {
    /// `/heartbeat every <interval> <prompt>` — e.g. `/heartbeat every 5m check the CI status`.
    /// The binary validates `interval` (30s/5m/1h forms, minimum 30s) and reports a parse error.
    Every { interval: String, prompt: String },
    /// `/heartbeat` / `/heartbeat status` — show whether one is set, and its next-due countdown.
    Status,
    /// `/heartbeat pause` — stop firing without losing the prompt/interval.
    Pause,
    /// `/heartbeat resume` — start firing again, rescheduled from now.
    Resume,
    /// `/heartbeat clear` — delete it entirely.
    Clear,
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

/// `/heartbeat every <interval> <prompt> | status | pause | resume | clear`
pub(crate) fn heartbeat_action(arg: &str) -> CommandAction {
    let trimmed = arg.trim();
    let action = if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("status") {
        HeartbeatAction::Status
    } else if trimmed.eq_ignore_ascii_case("pause") {
        HeartbeatAction::Pause
    } else if trimmed.eq_ignore_ascii_case("resume") {
        HeartbeatAction::Resume
    } else if trimmed.eq_ignore_ascii_case("clear") {
        HeartbeatAction::Clear
    } else {
        let rest = trimmed
            .strip_prefix("every ")
            .or_else(|| trimmed.strip_prefix("every"))
            .unwrap_or(trimmed);
        let mut parts = rest.trim().splitn(2, char::is_whitespace);
        let interval = parts.next().unwrap_or("").trim().to_string();
        let prompt = parts.next().unwrap_or("").trim().to_string();
        HeartbeatAction::Every { interval, prompt }
    };
    CommandAction::Heartbeat(action)
}
