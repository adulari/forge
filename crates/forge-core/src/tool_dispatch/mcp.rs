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
                "tool": call.name, "args": effective_args, "cwd": self.workspace.display()
            })
            .to_string();
            let outcome = hooks::run_hooks(
                &self.config.hooks,
                forge_config::HookEvent::PreToolUse,
                &call.name,
                &payload,
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
                if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result) {
                    self.presenter
                        .emit(PresenterEvent::Warning(warning.clone()));
                    self.pending_hints.push(warning);
                }
                return Ok(result);
            }
            if let Some(new_args) = outcome.rewritten_args {
                args_json = serde_json::to_string(&new_args).unwrap_or_default();
                effective_args = new_args;
            }
        }

        let allowed = match permission::decide(
            self.mode,
            side_effect,
            &call.name,
            &effective_args,
            &self.rules,
        ) {
            PermissionDecision::Allow => true,
            PermissionDecision::Deny => false,
            PermissionDecision::Ask => match self.presenter.confirm(&call.name, side_effect) {
                forge_types::ConfirmOutcome::AlwaysAllow => {
                    self.rules.push(forge_types::PermissionRule {
                        tool: call.name.clone(),
                        patterns: vec![],
                        decision: forge_types::PermissionDecision::Allow,
                        source: forge_types::RuleSource::Configured,
                        reason: Some("user answered 'always' at runtime prompt".into()),
                    });
                    true
                }
                forge_types::ConfirmOutcome::Allow => true,
                forge_types::ConfirmOutcome::Deny => false,
            },
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
                    PermissionDecision::Ask => match self
                        .presenter
                        .confirm(inner_name, forge_types::SideEffect::External)
                    {
                        forge_types::ConfirmOutcome::AlwaysAllow => {
                            self.rules.push(forge_types::PermissionRule {
                                tool: inner_name.to_string(),
                                patterns: vec![],
                                decision: forge_types::PermissionDecision::Allow,
                                source: forge_types::RuleSource::Configured,
                                reason: Some("user answered 'always' at runtime prompt".into()),
                            });
                            true
                        }
                        forge_types::ConfirmOutcome::Allow => true,
                        forge_types::ConfirmOutcome::Deny => false,
                    },
                }
            }
        } else {
            allowed
        };
        let permission_label = if allowed { "allowed" } else { "denied" };

        let (result, ok) = if allowed {
            let out = mcp.call(&call.name, &effective_args).await;
            (out.text, out.ok)
        } else {
            ("permission denied by policy".to_string(), false)
        };

        self.presenter.emit(PresenterEvent::ToolResult {
            name: call.name.clone(),
            ok,
            summary: summarize(&result),
            detail: crate::tool_detail(&result),
        });
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            permission_label,
            if ok { "ok" } else { "error" },
        )?;

        // PostToolUse hooks: observe only — notes surfaced, result unchanged.
        if !self.config.hooks.is_empty() {
            let payload = serde_json::json!({
                "tool": call.name, "args": effective_args, "result": result, "ok": ok, "cwd": self.workspace.display()
            })
            .to_string();
            let outcome = hooks::run_hooks(
                &self.config.hooks,
                forge_config::HookEvent::PostToolUse,
                &call.name,
                &payload,
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
        }

        Ok(result)
    }
}
