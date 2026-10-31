//! The MCP half of tool dispatch: an MCP (meta-)tool call routed through the permission broker
//! and the manager, with hooks and audit exactly as for a built-in tool.
//!
//! Split out of `tool_dispatch.rs` so that file stays under the CI implementation-size limit
//! (`scripts/ci/architecture_size.py`); the two paths share `Session` and nothing else.

use super::*;

impl Session {
    /// Run an MCP (meta-)tool call through the permission broker and the manager. Every MCP call
    /// is `SideEffect::External` (the local catalog meta-tools are `ReadOnly`); the broker decides
    /// allow/ask/deny exactly as for built-in tools, and the call is recorded for audit.
    pub(crate) async fn invoke_mcp(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let Some(mcp) = self.mcp.clone() else {
            return Err(CoreError::Internal(
                "invoke_mcp called without an MCP manager".into(),
            ));
        };
        let mut args_json = serde_json::to_string(&call.args)?;
        let mut effective_args = call.args.clone();
        let side_effect = mcp.side_effect_of(&call.name);
        self.presenter.emit(PresenterEvent::ToolStart {
            name: call.name.clone(),
            args: args_json.clone(),
        });

        // PreToolUse hooks: same semantics as native tools — block, observe, or rewrite args.
        if !self.config.hooks.is_empty() {
            let payload = serde_json::json!({
                "tool": call.name, "args": effective_args, "cwd": self.workspace.display(), "session_id": self.id
            })
            .to_string();
            let runner = self.llm_hook_runner().await;

            let outcome = crate::hooks_llm::with_runner(
                runner,
                hooks::run_hooks(
                    &self.config.hooks,
                    forge_config::HookEvent::PreToolUse,
                    &call.name,
                    &payload,
                ),
            )
            .await;
            for n in outcome.notes {
                self.presenter.emit(PresenterEvent::Warning(n));
            }
            // Queue any hook-injected context as a model-visible system hint (drained into the
            // transcript after the tool result), so a hook can feed the model extra context.
            for ctx in outcome.injected_context {
                self.pending_hints.push(ctx);
            }
            if let Some(reason) = outcome.blocked {
                let result = format!("blocked by hook: {reason}");
                self.presenter.emit(PresenterEvent::ToolResult {
                    name: call.name.clone(),
                    ok: false,
                    summary: "blocked by hook".to_string(),
                    detail: crate::tool_detail(&result),
                });
                self.store.record_tool_call(
                    msg_id, &call.name, &args_json, &result, "blocked", "error",
                )?;
                return Ok(result);
            }
            if let Some(new_args) = outcome.rewritten_args {
                args_json = serde_json::to_string(&new_args).unwrap_or_default();
                effective_args = new_args;
            }
        }

        // `mcp_call` is only a wrapper: the real prompt is the inner tool's, asked below. Asking
        // for the wrapper too made every call prompt twice.
        let defer_to_inner_gate = call.name == forge_mcp::MCP_CALL
            && ["name", "qualified_name", "tool"]
                .iter()
                .filter_map(|key| effective_args.get(*key).and_then(|v| v.as_str()))
                .any(|name| !name.is_empty());
        let mut prompt: Option<forge_types::ConfirmReply> = None;
        let allowed = match permission::decide(
            self.mode,
            side_effect,
            &call.name,
            &effective_args,
            &self.rules,
        ) {
            PermissionDecision::Allow => true,
            PermissionDecision::Deny => false,
            PermissionDecision::Ask if defer_to_inner_gate => true,
            PermissionDecision::Ask => self.ask_mcp(&call.name, side_effect, &mut prompt),
        };
        // When the model routes an MCP server tool via the mcp_call meta-wrapper, also gate the
        // inner (real) tool name against the permission broker. Without this, a per-tool
        // allow/ask/deny rule targeting e.g. "myserver__dangerous" is bypassed on the direct
        // path because the outer broker only sees "mcp_call".
        let allowed = if allowed && call.name == forge_mcp::MCP_CALL {
            let inner_name = effective_args
                .get("name")
                .or_else(|| effective_args.get("qualified_name"))
                .or_else(|| effective_args.get("tool"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let inner_args = effective_args
                .get("arguments")
                .or_else(|| effective_args.get("args"))
                .cloned()
                .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
            if inner_name.is_empty() {
                true
            } else {
                match permission::decide(
                    self.mode,
                    forge_types::SideEffect::External,
                    inner_name,
                    &inner_args,
                    &self.rules,
                ) {
                    PermissionDecision::Allow => true,
                    PermissionDecision::Deny => false,
                    PermissionDecision::Ask => {
                        self.ask_mcp(inner_name, forge_types::SideEffect::External, &mut prompt)
                    }
                }
            }
        } else {
            allowed
        };
        let permission_label = if allowed { "allowed" } else { "denied" };

        let note = prompt.as_ref().and_then(|p| p.note.as_deref());
        let (result, ok) = if allowed {
            let out = self
                .call_mcp_serving_elicitation(&mcp, &call.name, &effective_args)
                .await;
            match note {
                Some(note) => (
                    format!(
                        "{}\n\n{}",
                        out.text,
                        super::runtime_allow::note_for_model(true, note)
                    ),
                    out.ok,
                ),
                None => (out.text, out.ok),
            }
        } else if prompt.as_ref().is_some_and(|p| !p.outcome.allows()) {
            let reason =
                note.map(|n| format!(". {}", super::runtime_allow::note_for_model(false, n)));
            (
                format!(
                    "permission denied by the user (declined at the prompt){}",
                    reason.unwrap_or_default()
                ),
                false,
            )
        } else {
            ("permission denied by policy".to_string(), false)
        };

        self.presenter.emit(PresenterEvent::ToolResult {
            name: call.name.clone(),
            ok,
            summary: summarize(&result),
            detail: crate::tool_detail(&result),
        });
        let kept = self.surface_full_output(&call.name, &call.id, None, &result);
        let result = crate::tool_output::fit_for_model(result, kept.as_deref());
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            permission_label,
            if ok { "ok" } else { "error" },
        )?;

        // PostToolUse / PostToolUseFailure hooks: observe only — notes surfaced, result unchanged.
        self.run_post_tool_hooks(call, &effective_args, &result, allowed, ok)
            .await;

        Ok(result)
    }
}

impl Session {
    /// Prompt for an MCP tool. Both standing answers add an in-memory rule only: MCP rules are
    /// never written to the project config.
    pub(crate) fn ask_mcp(
        &mut self,
        tool: &str,
        side_effect: forge_types::SideEffect,
        prompt: &mut Option<forge_types::ConfirmReply>,
    ) -> bool {
        let reply = self.presenter.confirm_reply(tool, side_effect);
        let outcome = reply.outcome;
        if matches!(
            outcome,
            forge_types::ConfirmOutcome::AlwaysAllow | forge_types::ConfirmOutcome::AllowSession
        ) {
            self.rules.push(forge_types::PermissionRule {
                tool: tool.to_string(),
                patterns: vec![],
                decision: forge_types::PermissionDecision::Allow,
                source: forge_types::RuleSource::Configured,
                reason: Some("user allowed it for this session at runtime prompt".into()),
            });
        }
        *prompt = Some(reply);
        outcome.allows()
    }
}

#[cfg(test)]
mod tests;
