//! Answers at the runtime permission prompt: allow once, allow for this session, always allow
//! (saved to the project config), deny, each with an optional note for the model.

use super::*;

/// What the user said at the prompt, as the dispatch path needs it.
pub(super) struct PromptAnswer {
    pub allowed: bool,
    /// A person said no (as opposed to a rule or the mode).
    pub declined: bool,
    pub note: Option<String>,
}

/// Patterns for a runtime "always allow", scoped to the command the user was shown. An empty list
/// means tool-wide (the previous behaviour), kept for cases we cannot narrow confidently.
fn always_allow_patterns(tool: &str, args: &serde_json::Value) -> Vec<String> {
    if tool != "shell" {
        return Vec::new();
    }
    let Some(cmd_text) = forge_types::shell_command_text(args) else {
        return Vec::new();
    };
    let cmd = cmd_text.as_str();
    // The leading word only: `git status --short` grants `git *`, never `--short`.
    match cmd.split_whitespace().next() {
        Some(verb) if !verb.is_empty() && !verb.starts_with('-') => {
            vec![format!("{verb} *"), verb.to_string()]
        }
        _ => Vec::new(),
    }
}

/// The note the model reads with the tool result. On a deny it is the reason; on an allow it
/// follows the output.
pub(super) fn note_for_model(allowed: bool, note: &str) -> String {
    if allowed {
        format!("[the user approved this call and added: {note}]")
    } else {
        format!("The user said: {note}")
    }
}

impl Session {
    /// Ask the user about `call` and apply a standing answer: "always" persists a rule scoped to
    /// what they were shown and adds it to this session, "this session" only adds it here.
    pub(super) fn ask_at_prompt(
        &mut self,
        call: &forge_types::ToolCall,
        effective_args: &serde_json::Value,
        side_effect: forge_types::SideEffect,
    ) -> PromptAnswer {
        let learn = self.learnable_shape(call, effective_args, side_effect);
        let offered = learn
            .as_ref()
            .and_then(|shape| self.rule_offer(shape))
            .is_some_and(|offer| self.presenter.offer_rule(&offer));
        let mut reply = permission::confirm_call(
            self.presenter.as_mut(),
            &call.name,
            effective_args,
            side_effect,
        );
        let accepted = offered && reply.accept_rule && reply.outcome.allows();
        if accepted {
            if let Some(shape) = &learn {
                self.accept_learned_rule(shape);
            }
            reply.outcome = forge_types::ConfirmOutcome::Allow;
        }
        let outcome = reply.outcome;
        if matches!(
            outcome,
            forge_types::ConfirmOutcome::AlwaysAllow | forge_types::ConfirmOutcome::AllowSession
        ) {
            // Scope to what the user SAW: an empty list matches every segment (see
            // decide_shell_segments), so approving `git status` used to auto-approve every later
            // shell command in the session.
            let patterns = always_allow_patterns(&call.name, &call.args);
            let persist = outcome == forge_types::ConfirmOutcome::AlwaysAllow;
            if persist && call.name != forge_tools::PYTHON_TOOL {
                if let Err(e) =
                    forge_config::append_allow_rule_in(self.rules_root(), &call.name, &patterns)
                {
                    self.presenter.emit(PresenterEvent::Warning(format!(
                        "allow rule not saved: {e}"
                    )));
                }
            }
            // An unscopable shell call is allowed once, whichever standing answer was picked.
            // Arbitrary code: one click must not approve every later cell, so `python` stays allow-once
            // and users who want a standing grant write a config rule.
            let unscopable = call.name == forge_tools::PYTHON_TOOL
                || (call.name == "shell" && patterns.is_empty());
            if !unscopable {
                let reason = if persist {
                    forge_config::RUNTIME_ALWAYS_REASON
                } else {
                    crate::permission::RUNTIME_SESSION_REASON
                };
                self.rules.push(forge_types::PermissionRule {
                    tool: call.name.clone(),
                    patterns,
                    decision: forge_types::PermissionDecision::Allow,
                    source: forge_types::RuleSource::Configured,
                    reason: Some(reason.into()),
                });
            }
        }
        self.record_decision(&call.name, learn.as_ref(), outcome, accepted, offered);
        PromptAnswer {
            allowed: outcome.allows(),
            declined: !outcome.allows(),
            note: reply.note,
        }
    }
    /// The shape this call would be learned under, when it has one.
    fn learnable_shape(
        &self,
        call: &forge_types::ToolCall,
        effective_args: &serde_json::Value,
        side_effect: forge_types::SideEffect,
    ) -> Option<permission::learn::Shape> {
        let workspace = self.workspace_binding.read().ok().map(|g| g.clone());
        let beyond = self.writes_beyond_workspace(effective_args);
        permission::learn::shape_for(
            &call.name,
            side_effect,
            effective_args,
            &self.rules,
            workspace.as_deref(),
            beyond,
        )
        .filter(|_| !permission::learn::covered(&self.rules, &call.name, effective_args))
    }

    /// The rule to offer with this prompt: the shape has been allowed enough times here since the
    /// last time it was refused or its offer turned down.
    fn rule_offer(&self, shape: &permission::learn::Shape) -> Option<forge_types::RuleSuggestion> {
        let allows = self
            .store
            .permission_allow_streak(
                &self.rules_root().to_string_lossy(),
                &shape.tool,
                &shape.key,
            )
            .ok()?;
        (allows >= permission::learn::OFFER_AFTER_ALLOWS).then(|| forge_types::RuleSuggestion {
            rule: shape.key.clone(),
            allows,
        })
    }

    /// Write the offered rule to the project and keep it for the rest of this session, through the
    /// same plumbing as an "always allow" answer.
    fn accept_learned_rule(&mut self, shape: &permission::learn::Shape) {
        if let Err(e) =
            forge_config::append_allow_rule_in(self.rules_root(), &shape.tool, &shape.patterns)
        {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "allow rule not saved: {e}"
            )));
        }
        self.rules.push(forge_types::PermissionRule {
            tool: shape.tool.clone(),
            patterns: shape.patterns.clone(),
            decision: forge_types::PermissionDecision::Allow,
            source: forge_types::RuleSource::Configured,
            reason: Some(forge_config::RUNTIME_ALWAYS_REASON.into()),
        });
    }

    /// Remember how the prompt was answered, so a shape the person keeps allowing can be offered
    /// as a rule. A write failure only costs the learning, never the answer.
    fn record_decision(
        &self,
        tool: &str,
        shape: Option<&permission::learn::Shape>,
        outcome: forge_types::ConfirmOutcome,
        accepted: bool,
        offered: bool,
    ) {
        use forge_types::ConfirmOutcome as O;
        let project = self.rules_root().to_string_lossy().into_owned();
        let key = shape.map_or("", |s| s.key.as_str());
        let label = match outcome {
            _ if accepted => "suggestion_accepted",
            O::Allow => "allow",
            O::AllowSession => "session",
            O::AlwaysAllow => "always",
            O::Deny => "deny",
        };
        let _ = self
            .store
            .record_permission_decision(&project, tool, key, label);
        if offered && !accepted && outcome != O::AlwaysAllow && outcome != O::Deny {
            let _ =
                self.store
                    .record_permission_decision(&project, tool, key, "suggestion_declined");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{always_allow_patterns, note_for_model};

    /// "Always allow" must grant what the user was SHOWN, not the whole tool. An empty pattern
    /// list matches every shell segment (see `decide_shell_segments`), so approving `git status`
    /// once used to authorise every later shell command in the session.
    #[test]
    fn always_allow_scopes_the_grant_to_the_approved_verb() {
        let args = serde_json::json!({ "command": "git status --short" });
        let pats = always_allow_patterns("shell", &args);
        assert!(
            pats.iter().any(|p| p == "git *"),
            "the approved verb must be granted: {pats:?}"
        );
        assert!(
            !pats.iter().any(|p| p == "--short"),
            "a flag must never become the granted pattern: {pats:?}"
        );

        // Cases we cannot narrow confidently stay tool-wide rather than silently granting less
        // than the user expects, which would make "always" unreliable in the other direction.
        assert!(always_allow_patterns("shell", &serde_json::json!({ "command": "" })).is_empty());
        assert!(always_allow_patterns("read_file", &serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_note_reads_as_a_reason_on_deny_and_as_an_addendum_on_allow() {
        assert_eq!(
            note_for_model(false, "use rg instead"),
            "The user said: use rg instead"
        );
        assert!(note_for_model(true, "only this once").contains("approved this call"));
    }
}
