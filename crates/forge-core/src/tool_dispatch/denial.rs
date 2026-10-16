//! The tool result a model reads when a call was not permitted.

use forge_types::PermissionMode;

/// Why a call was refused, worded so the model neither hunts for a rule to bypass (a person said
/// no) nor retries a prompt nobody can answer (a non-interactive run).
fn message(tool: &str, mode: PermissionMode, declined: bool, attended: bool) -> String {
    match (declined, attended) {
        (true, false) => format!(
            "permission denied: `{tool}` needs approval and no one is attached to give it \
             (non-interactive run in {} mode). Re-run with --mode accept-edits or bypass, or add \
             an allow rule.",
            mode.label()
        ),
        (true, true) => "permission denied by the user (declined at the prompt)".to_string(),
        (false, _) => format!(
            "permission denied by policy: `{tool}` is not allowed in {} mode or is blocked by a \
             deny rule",
            mode.label()
        ),
    }
}

/// [`message`] as a failed tool outcome `(model text, full output, ok)`.
pub(super) fn result(
    tool: &str,
    mode: PermissionMode,
    declined: bool,
    attended: bool,
) -> (String, Option<String>, bool) {
    (message(tool, mode, declined, attended), None, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_refusal_says_why_and_what_to_do() {
        let policy = message("shell", PermissionMode::Default, false, true);
        assert!(policy.starts_with("permission denied by policy") && policy.contains("shell"));
        let headless = message("write_file", PermissionMode::Default, true, false);
        assert!(headless.contains("no one is attached") && headless.contains("--mode"));
        assert!(message("shell", PermissionMode::Default, true, true).contains("declined"));
    }
}
