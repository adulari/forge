//! `PermissionRequest` hooks and the safety-floor refusal on the tool-dispatch path.
//!
//! Split out of `tool_dispatch.rs` so that file stays under the CI implementation-size limit
//! (`scripts/ci/architecture_size.py`).

use super::*;

impl Session {
    /// Adopt arguments a `PermissionRequest` hook rewrote (`updatedInput`): normalized and checked
    /// exactly like a `PreToolUse` rewrite, and refused when the safety floor or a deny rule
    /// rejects the new form (an allow for one command must not smuggle in another).
    fn adopt_hook_args(
        &self,
        tool: &str,
        new_args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let args = subagent::rewrite_args_for_root(&new_args, self.workspace.root());
        let args = add_workspace_default_path(tool, args, self.workspace.root());
        let (args, readable_roots) = self.scope_to_workspace(tool, args);
        self.check_workspace(tool, &args, &readable_roots)
            .map_err(|e| format!("rewritten input is outside the session workspace: {e}"))?;
        let side_effect = self
            .tools
            .get(tool)
            .map_or(forge_types::SideEffect::Shell, |t| t.side_effect());
        if permission::decide(
            forge_types::PermissionMode::Bypass,
            side_effect,
            tool,
            &args,
            &self.rules,
        ) == PermissionDecision::Deny
        {
            return Err(permission::denial_note(tool, &args)
                .unwrap_or_else(|| "rewritten input is denied by a permission rule".to_string()));
        }
        Ok(args)
    }

    /// Run the `PermissionRequest` hooks for a call that would ask the user. Returns the settled
    /// decision and, when a hook refused, the reason the model reads. A hook's `updatedInput`
    /// replaces `effective_args` (and `args_json`) only once it passes [`Self::adopt_hook_args`].
    pub(super) async fn settle_permission_hooks(
        &mut self,
        decision: PermissionDecision,
        tool: &str,
        effective_args: &mut serde_json::Value,
        args_json: &mut String,
    ) -> (PermissionDecision, Option<String>) {
        let mut decision = decision;
        let mut hook_denial: Option<String> = None;
        if decision == PermissionDecision::Ask
            && self
                .config
                .hooks
                .iter()
                .any(|h| h.event == forge_config::HookEvent::PermissionRequest)
        {
            let payload = serde_json::json!({
                "tool": tool,
                "args": &*effective_args,
                "cwd": self.workspace.display(),
                "session_id": self.id,
                "permission_mode": self.mode.key(),
            })
            .to_string();
            let runner = self.llm_hook_runner().await;
            let outcome = crate::hooks_llm::with_runner(
                runner,
                hooks::run_permission_hooks(&self.config.hooks, tool, &payload),
            )
            .await;
            for n in outcome.notes {
                self.presenter.emit(PresenterEvent::Warning(n));
            }
            match outcome.decision {
                Some(hooks::PermissionHookDecision::Allow { updated_input }) => {
                    let updated_ok = match updated_input {
                        None => true,
                        Some(new_args) => match self.adopt_hook_args(tool, new_args) {
                            Ok(args) => {
                                *effective_args = args;
                                *args_json =
                                    serde_json::to_string(&*effective_args).unwrap_or_default();
                                true
                            }
                            Err(why) => {
                                hook_denial = Some(why);
                                false
                            }
                        },
                    };
                    decision = if updated_ok {
                        PermissionDecision::Allow
                    } else {
                        PermissionDecision::Deny
                    };
                }
                Some(hooks::PermissionHookDecision::Deny { message, interrupt }) => {
                    if interrupt {
                        self.pending_hints.push(format!(
                            "A PermissionRequest hook denied `{}` and interrupted the turn: \
                             {message}. Stop now: do not call more tools; tell the user what \
                             you were trying to do.",
                            tool
                        ));
                    }
                    hook_denial = Some(message);
                    decision = PermissionDecision::Deny;
                }
                None => {}
            }
        }
        (decision, hook_denial)
    }

    /// The safety floor's model-facing refusal (privileged command or catastrophic delete), with a
    /// warning so the user sees why. `None` for any other denial.
    pub(super) fn floor_denial(&mut self, tool: &str, args: &serde_json::Value) -> Option<String> {
        let note = permission::denial_note(tool, args)?;
        let command = forge_types::shell_command_text(args).unwrap_or_default();
        let why = if permission::privilege_escalation(&command).is_some() {
            "it needs root and Forge never runs it — run it yourself in a terminal if you want it"
        } else {
            "Forge's safety floor never runs a recursive delete of home or root"
        };
        self.presenter.emit(PresenterEvent::Warning(format!(
            "refused `{command}`: {why}"
        )));
        Some(note)
    }
}
