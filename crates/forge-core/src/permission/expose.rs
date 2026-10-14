//! `shell_job {action:"expose"}` publishes a local port on a public, unauthenticated URL. That is
//! a different order of risk from running a command, so it is never approved silently: no allow
//! rule, `auto` verdict, accept-edits or bypass skips the prompt. Plan mode denies it earlier, like
//! every other side effect, and a deny rule still wins.

use forge_types::{ConfirmOutcome, Presenter, SideEffect};
use serde_json::Value;

/// Every prompt label for an expose starts with this, so a presenter with no human behind it
/// (the MCP agent server) can refuse it instead of auto-approving by mode.
pub const EXPOSE_PROMPT_PREFIX: &str = "shell_job expose";

pub fn always_asks(tool_name: &str, args: &Value) -> bool {
    tool_name == "shell_job" && args.get("action").and_then(Value::as_str) == Some("expose")
}

/// What the permission prompt names instead of the bare tool, so the user sees they are
/// publishing a port rather than approving an anonymous `shell_job` call.
pub fn confirm_label(tool_name: &str, args: &Value) -> Option<String> {
    if !always_asks(tool_name, args) {
        return None;
    }
    let port = args
        .get("port")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".to_string(), |p| p.to_string());
    let job = args
        .get("id")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".to_string(), |id| id.to_string());
    Some(format!(
        "{EXPOSE_PROMPT_PREFIX} — PUBLIC unauthenticated URL for port {port} (job {job})"
    ))
}

/// Ask the presenter about a call, naming an expose for what it is. "Always" on an expose counts
/// as a one-time yes: it asks every time, so a saved allow rule could never apply anyway.
pub fn confirm_call(
    presenter: &mut dyn Presenter,
    tool_name: &str,
    args: &Value,
    side_effect: SideEffect,
) -> ConfirmOutcome {
    let Some(label) = confirm_label(tool_name, args) else {
        return presenter.confirm(tool_name, side_effect);
    };
    match presenter.confirm(&label, side_effect) {
        ConfirmOutcome::AlwaysAllow => ConfirmOutcome::Allow,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::super::decide;
    use forge_types::{
        ConfirmOutcome, PermissionDecision, PermissionMode, PermissionRule, RuleSource, SideEffect,
    };
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
    fn the_prompt_names_the_public_port_only_for_expose() {
        let label = super::confirm_label(
            "shell_job",
            &json!({"action": "expose", "id": 4242, "port": 5173}),
        )
        .unwrap();
        assert!(label.starts_with(super::EXPOSE_PROMPT_PREFIX), "{label}");
        assert!(label.contains("PUBLIC") && label.contains("5173") && label.contains("4242"));
        assert_eq!(
            super::confirm_label("shell_job", &json!({"action": "stop", "id": 1})),
            None
        );
        assert_eq!(
            super::confirm_label("shell", &json!({"command": "expose"})),
            None
        );
    }

    struct Answers {
        seen: Vec<String>,
    }

    impl forge_types::Presenter for Answers {
        fn emit(&mut self, _event: forge_types::PresenterEvent) {}
        fn confirm(&mut self, tool: &str, _side_effect: SideEffect) -> ConfirmOutcome {
            self.seen.push(tool.to_string());
            ConfirmOutcome::AlwaysAllow
        }
        fn ask(&mut self, _q: &str, _o: &[forge_types::QChoice], _other: bool) -> String {
            String::new()
        }
        fn read_line(&mut self) -> Option<String> {
            None
        }
    }

    #[test]
    fn always_on_an_expose_is_a_one_time_yes_under_its_own_label() {
        let mut presenter = Answers { seen: Vec::new() };
        let expose = json!({"action": "expose", "id": 1, "port": 3000});
        assert_eq!(
            super::confirm_call(&mut presenter, "shell_job", &expose, SideEffect::Shell),
            ConfirmOutcome::Allow
        );
        let stop = json!({"action": "stop", "id": 1});
        assert_eq!(
            super::confirm_call(&mut presenter, "shell_job", &stop, SideEffect::Shell),
            ConfirmOutcome::AlwaysAllow
        );
        assert!(presenter.seen[0].starts_with(super::EXPOSE_PROMPT_PREFIX));
        assert_eq!(presenter.seen[1], "shell_job");
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
