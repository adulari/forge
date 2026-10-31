//! The hooks that follow a tool call's permission decision and result: `PostToolUse`,
//! `PostToolUseFailure` and `PermissionDenied` (docs/features/hooks.md).
//!
//! Split out of `tool_dispatch.rs` so that file stays under the CI implementation-size limit
//! (`scripts/ci/architecture_size.py`).

use super::*;
use forge_config::{HookConfig, HookEvent};

impl Session {
    /// Hooks after a call settled. Native `PostToolUse` hooks see every call with its `ok` flag, as
    /// they always have. Claude-Code hooks follow Claude Code: `PostToolUse` after a call that ran
    /// and succeeded, `PostToolUseFailure` (with `error`) after one that ran and failed, which for
    /// `shell` includes a command that exited non-zero. A refused call fires neither.
    pub(crate) async fn run_post_tool_hooks(
        &mut self,
        call: &forge_types::ToolCall,
        args: &serde_json::Value,
        result: &str,
        allowed: bool,
        ok: bool,
    ) {
        if self.config.hooks.is_empty() {
            return;
        }
        let failed =
            !ok || (permission::is_shell_tool(&call.name) && shell_command_did_fail(result));
        let post: Vec<HookConfig> = self
            .config
            .hooks
            .iter()
            .filter(|h| h.event == HookEvent::PostToolUse && (!h.cc_compat || allowed && !failed))
            .cloned()
            .collect();
        let payload = serde_json::json!({
            "tool": call.name, "args": args, "result": result, "ok": ok,
            "cwd": self.workspace.display(), "session_id": self.id, "tool_use_id": call.id,
        });
        if !post.is_empty() {
            self.fire_tool_event(HookEvent::PostToolUse, &post, &call.name, &payload)
                .await;
        }
        let failure_hooks = allowed
            && failed
            && self
                .config
                .hooks
                .iter()
                .any(|h| h.event == HookEvent::PostToolUseFailure);
        if failure_hooks {
            let payload = serde_json::json!({
                "tool": call.name, "args": args, "error": result, "is_interrupt": false,
                "cwd": self.workspace.display(), "session_id": self.id, "tool_use_id": call.id,
            });
            let hooks = self.config.hooks.clone();
            self.fire_tool_event(HookEvent::PostToolUseFailure, &hooks, &call.name, &payload)
                .await;
        }
    }

    /// Run one tool event's hooks, surface their notes and queue their context for the model.
    async fn fire_tool_event(
        &mut self,
        event: HookEvent,
        hooks: &[HookConfig],
        tool: &str,
        payload: &serde_json::Value,
    ) -> hooks::HookOutcome {
        let runner = self.llm_hook_runner().await;
        let mut outcome = crate::hooks_llm::with_runner(
            runner,
            hooks::run_hooks(hooks, event, tool, &payload.to_string()),
        )
        .await;
        for n in std::mem::take(&mut outcome.notes) {
            self.presenter.emit(PresenterEvent::Warning(n));
        }
        // Drained into the transcript right after the tool result.
        self.pending_hints
            .extend(std::mem::take(&mut outcome.injected_context));
        outcome
    }

    /// The result a refused call returns, firing `PermissionDenied` when Forge refused it on its
    /// own: the permission mode forbids it, a prompt no one could answer (an unattended run), or
    /// the safety floor. A person saying
    /// no, a deny rule, a `PermissionRequest` hook's denial and a `PreToolUse` block do not fire
    /// it (Claude Code's rule: only automatic denials). A hook answering `retry: true` lets the
    /// model try again, except after a safety-floor refusal, which no retry can change.
    pub(super) async fn refusal(
        &mut self,
        call: &forge_types::ToolCall,
        args: &serde_json::Value,
        hook_denial: Option<String>,
        user_declined: bool,
    ) -> (String, Option<String>, bool) {
        let attended = self.presenter.is_attended();
        let (text, automatic) = if self.mode == forge_types::PermissionMode::Plan
            && permission::is_shell_tool(&call.name)
        {
            (permission::plan_shell_denial(), None)
        } else if let Some(reason) = hook_denial {
            (denial::hook(&reason).0, None)
        } else if let Some(note) = self.floor_denial(&call.name, args) {
            (note, Some(false))
        } else if let Some(rule) = permission::denying_rule(&call.name, args, &self.rules) {
            (permission::rule_denial(&call.name, rule, args), None)
        } else {
            let (text, _, _) = denial::result(&call.name, self.mode, user_declined, attended);
            // Only a person saying no is not automatic: a policy refusal or a prompt no one
            // was there to answer is Forge's own decision.
            (text, (!(user_declined && attended)).then_some(true))
        };
        if let Some(retry_honored) = automatic {
            self.fire_permission_denied(call, args, &text, retry_honored)
                .await;
        }
        (text, None, false)
    }

    async fn fire_permission_denied(
        &mut self,
        call: &forge_types::ToolCall,
        args: &serde_json::Value,
        reason: &str,
        retry_honored: bool,
    ) {
        if !self
            .config
            .hooks
            .iter()
            .any(|h| h.event == HookEvent::PermissionDenied)
        {
            return;
        }
        let payload = serde_json::json!({
            "tool": call.name, "args": args, "reason": reason, "cwd": self.workspace.display(),
            "session_id": self.id, "tool_use_id": call.id, "permission_mode": self.mode.key(),
        });
        let hooks = self.config.hooks.clone();
        let outcome = self
            .fire_tool_event(HookEvent::PermissionDenied, &hooks, &call.name, &payload)
            .await;
        if outcome.retry && retry_honored {
            self.pending_hints.push(format!(
                "A PermissionDenied hook says you may retry the refused `{}` call.",
                call.name
            ));
        }
    }
}
