//! `shell_job {action:"expose"}` publishes a local port on a public, unauthenticated URL. That is
//! a different order of risk from running a command, so it is never approved silently: no allow
//! rule, `auto` verdict, accept-edits or bypass skips the prompt. Plan mode denies it earlier, like
//! every other side effect, and a deny rule still wins.

use serde_json::Value;

pub(super) fn always_asks(tool_name: &str, args: &Value) -> bool {
    tool_name == "shell_job" && args.get("action").and_then(Value::as_str) == Some("expose")
}

#[cfg(test)]
mod tests {
    use super::super::decide;
    use forge_types::{PermissionDecision, PermissionMode, PermissionRule, RuleSource, SideEffect};
    use serde_json::json;

    fn decide_expose(mode: PermissionMode, rules: &[PermissionRule]) -> PermissionDecision {
        decide(
            mode,
            SideEffect::Shell,
            "shell_job",
            &json!({"action": "expose", "id": 4242, "port": 5173}),
            rules,
        )
    }

    fn rule(decision: PermissionDecision) -> PermissionRule {
        PermissionRule {
            tool: "shell_job".into(),
            patterns: vec!["*".into()],
            decision,
            source: RuleSource::Configured,
            reason: None,
        }
    }

    #[test]
    fn expose_asks_in_every_mode_that_allows_side_effects() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ] {
            assert_eq!(
                decide_expose(mode, &[]),
                PermissionDecision::Ask,
                "{mode:?} must still ask before publishing a port"
            );
        }
    }

    #[test]
    fn expose_is_denied_in_plan_mode() {
        assert_eq!(
            decide_expose(PermissionMode::Plan, &[]),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn an_allow_rule_cannot_skip_the_prompt_but_a_deny_rule_wins() {
        assert_eq!(
            decide_expose(PermissionMode::Bypass, &[rule(PermissionDecision::Allow)]),
            PermissionDecision::Ask
        );
        assert_eq!(
            decide_expose(PermissionMode::Default, &[rule(PermissionDecision::Deny)]),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn the_other_shell_job_actions_keep_the_normal_shell_posture() {
        let list = |mode| {
            decide(
                mode,
                SideEffect::Shell,
                "shell_job",
                &json!({"action": "list"}),
                &[],
            )
        };
        assert_eq!(list(PermissionMode::Bypass), PermissionDecision::Allow);
        assert_eq!(list(PermissionMode::AcceptEdits), PermissionDecision::Allow);
    }
}
