//! Session built-in and MCP tool dispatch through permissions, hooks, and audit.
//!
//! This deep owner keeps effect ordering, permission checks, hook processing, and tool-call persistence together.

use super::*;

impl Session {
    /// Run a single tool call, applying the permission policy, and return its result text.
    /// Whether `name` is a side-effect-free registry tool that's safe to run concurrently in a
    /// batch: not a core-owned virtual tool (those mutate session state / prompt the user), not an
    /// external MCP tool, present in the registry, and ReadOnly.
    pub(crate) fn is_concurrent_readonly(&self, name: &str) -> bool {
        if name == subagent::SPAWN_AGENTS_TOOL
            || name == workflow::RUN_WORKFLOW_TOOL
            || name == ASK_USER_TOOL
            || name == UPDATE_TASKS_TOOL
            || name == PRESENT_PLAN_TOOL
            || name == USE_SKILL_TOOL
            || name == REMEMBER_TOOL
            || name == fleet::MESSAGE_SESSION_TOOL
            || name == heartbeat::MANAGE_HEARTBEATS_TOOL
        {
            return false;
        }
        if self.mcp.as_ref().is_some_and(|m| m.knows_tool(name)) {
            return false;
        }
        self.tools
            .get(name)
            .map(|t| t.side_effect() == forge_types::SideEffect::ReadOnly)
            .unwrap_or(false)
    }

    /// Execute a batch of side-effect-free tool calls CONCURRENTLY, then append their results in the
    /// original order. When the model requests several independent reads/searches in one step,
    /// running them together (instead of serially) is a direct latency win — and safe because
    /// ReadOnly tools have no side effects, never prompt (permission resolves to Allow/Deny without
    /// asking), don't snapshot, and queue no hints. Only used when all calls qualify and no hooks
    /// are configured (PreToolUse/PostToolUse run on every call and must stay serial); otherwise the
    /// caller falls back to the serial [`invoke_tool`] path.
    /// Returns each call's `(name, failure_kind)` in original order so the caller can feed the
    /// failure-loop guard exactly as the serial path does — a concurrent batch that keeps failing the
    /// same way (e.g. two reads of ever-changing missing paths every step) must still be caught.
    pub(crate) async fn run_readonly_batch(
        &mut self,
        msg_id: &str,
        calls: &[forge_types::ToolCall],
    ) -> Result<Vec<(String, Option<ErrorCategory>)>, CoreError> {
        struct Pending {
            id: String,
            name: String,
            args: serde_json::Value,
            args_json: String,
            allowed: bool,
        }
        // Phase 1 (serial): announce each call + resolve permission (pure; no prompt for ReadOnly).
        let mut pend = Vec::with_capacity(calls.len());
        for call in calls {
            let args_json = serde_json::to_string(&call.args)?;
            self.presenter.emit(PresenterEvent::ToolStart {
                name: call.name.clone(),
                args: args_json.clone(),
            });
            let allowed = matches!(
                permission::decide(
                    self.mode,
                    forge_types::SideEffect::ReadOnly,
                    &call.name,
                    &call.args,
                    &self.rules,
                ),
                PermissionDecision::Allow
            );
            pend.push(Pending {
                id: call.id.clone(),
                name: call.name.clone(),
                args: call.args.clone(),
                args_json,
                allowed,
            });
        }
        // Phase 2 (concurrent): run every allowed tool's `run()` together. Borrows `self.tools`
        // immutably for the duration of the join; no `&mut self` is touched until it completes.
        let results: Vec<(String, bool)> = {
            let tools = &self.tools;
            let futs = pend.iter().map(|p| async move {
                if !p.allowed {
                    return ("permission denied by policy".to_string(), false);
                }
                match tools.get(&p.name) {
                    Some(tool) => match tool.run(&p.args).await {
                        Ok(out) => (out, true),
                        Err(e) => (format!("error: {e}"), false),
                    },
                    None => (format!("error: unknown tool '{}'", p.name), false),
                }
            });
            futures::future::join_all(futs).await
        };
        // Phase 3 (serial): surface + persist + append each result in the ORIGINAL order, so every
        // tool_call_id is answered in sequence. Also classify each result for the failure-loop guard.
        let mut classified = Vec::with_capacity(pend.len());
        for (p, (result, ok)) in pend.iter().zip(results) {
            self.presenter.emit(PresenterEvent::ToolResult {
                name: p.name.clone(),
                ok,
                summary: summarize(&result),
            });
            self.store.record_tool_call(
                msg_id,
                &p.name,
                &p.args_json,
                &result,
                if p.allowed { "allowed" } else { "denied" },
                if ok { "ok" } else { "error" },
            )?;
            classified.push((p.name.clone(), classify_tool_failure(&result)));
            let seq = self.next_seq();
            self.store.add_message_full(
                &self.id,
                seq,
                Role::Tool,
                &result,
                None,
                &[],
                Some(&p.id),
            )?;
            self.transcript.push(Message::tool_result(&p.id, result));
        }
        Ok(classified)
    }

    pub(crate) async fn invoke_tool(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        // Snapshot args before hooks so the audit row preserves exactly what the model requested.
        let call_args_json = serde_json::to_string(&call.args)?;
        if let Some(scope) = self
            .task_scope
            .as_ref()
            .filter(|scope| !scope.permits_tool(&call.name))
        {
            let result = format!(
                "permission denied by task scope {}: `{}` is unavailable for {:?}",
                scope.audit_digest(),
                call.name,
                scope.contract.intent()
            );
            self.store.record_tool_call(
                msg_id,
                &call.name,
                &call_args_json,
                &result,
                "denied",
                "error",
            )?;
            return Ok(result);
        }
        if let Some(warning) = self
            .failure_tracker
            .record_call(&call.name, &call_args_json)
        {
            self.presenter
                .emit(PresenterEvent::Warning(warning.clone()));
            self.pending_hints.push(format!(
                "The `{}` call just repeated with identical arguments. Do not retry it unchanged; inspect the actual state or try a different tool/argument path.",
                call.name
            ));
            return Ok(warning);
        }

        // The subagent virtual tool is owned by core (it needs provider/router/store), not the
        // registry — intercept before the registry lookup (RFC subagent-orchestration).
        if call.name == subagent::SPAWN_AGENTS_TOOL {
            return self.spawn_agents(msg_id, call).await;
        }
        // Follow-ups to a persisted child (persistent subagents) — also core-owned.
        if call.name == subagent::SEND_TO_AGENT_TOOL {
            return self.send_to_agent(msg_id, call).await;
        }
        // Retained async subagents (RFC retained-async-subagents) — status + cancellation for
        // detached children, also core-owned (they need the store + the in-process abort registry).
        if call.name == subagent::LIST_SUBAGENTS_TOOL {
            return self.list_subagents(msg_id, call);
        }
        if call.name == subagent::CANCEL_SUBAGENT_TOOL {
            return self.cancel_subagent(msg_id, call);
        }
        // Workflow scripts are core-owned for the same reason (docs/rfcs/forge-workflow.md).
        if call.name == workflow::RUN_WORKFLOW_TOOL {
            return self.run_workflow(msg_id, call).await;
        }
        // The interactive question tool is core-owned too (it needs the presenter).
        if call.name == ASK_USER_TOOL {
            return self.ask_user(msg_id, call);
        }
        // Task tracking is core-owned (it mutates session state + persists + emits to the TUI).
        if call.name == UPDATE_TASKS_TOOL {
            return self.update_tasks(msg_id, call);
        }
        // Plan presentation is core-owned (seeds tasks, persists the plan, drives the approval flow).
        if call.name == PRESENT_PLAN_TOOL {
            return self.present_plan(msg_id, call);
        }
        // Skill loading is core-owned (it reads the attached catalog). Returns the skill's
        // methodology as the tool result so the model follows it; unknown name → a helpful error.
        if call.name == USE_SKILL_TOOL {
            return self.use_skill(msg_id, call);
        }
        // On-demand memory write — model calls this to persist a durable fact immediately,
        // without waiting for end-of-turn auto-capture.
        if call.name == REMEMBER_TOOL {
            return self.remember(msg_id, call).await;
        }
        // Fleet agent-to-agent messaging is core-owned but surface-agnostic: it only touches the
        // `FleetMessaging` callback the daemon wires in (ADR-0004) — never advertised, and so
        // never called, unless this session is forge-serve-hosted.
        if call.name == fleet::MESSAGE_SESSION_TOOL {
            return self.message_session(msg_id, call).await;
        }
        // Agent-created heartbeats are core-owned (they persist to the store and are scoped away
        // from the user's own `/heartbeat`, which never goes through tool dispatch at all).
        if call.name == heartbeat::MANAGE_HEARTBEATS_TOOL {
            return self.manage_heartbeats(msg_id, call);
        }
        // External MCP tools (meta-tools + exposed server tools) are owned by the manager, not the
        // built-in registry. Route them here, still through the permission broker (mcp-client.md).
        if self.mcp.as_ref().is_some_and(|m| m.knows_tool(&call.name)) {
            return self.invoke_mcp(msg_id, call).await;
        }

        let mut effective_args = call.args.clone();
        // Session workspace rooting is unconditional: all relative paths and omitted shell
        // cwd values resolve within this immutable session workspace.
        effective_args = subagent::rewrite_args_for_root(&effective_args, self.workspace.root());
        effective_args =
            add_workspace_default_path(&call.name, effective_args, self.workspace.root());
        let mut args_json = serde_json::to_string(&effective_args)?;
        if let Err(error) = validate_workspace_args(&effective_args, &self.workspace) {
            let result = format!("error: {error}");
            self.presenter.emit(PresenterEvent::ToolStart {
                name: call.name.clone(),
                args: args_json.clone(),
            });
            self.presenter.emit(PresenterEvent::ToolResult {
                name: call.name.clone(),
                ok: false,
                summary: "path outside session workspace".to_string(),
            });
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "denied", "error")?;
            if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result) {
                self.presenter
                    .emit(PresenterEvent::Warning(warning.clone()));
                self.pending_hints.push(warning);
            }
            return Ok(result);
        }

        if self.config.mesh.env_fight_nudge
            && self.env_fight.should_block()
            && call.name == "shell"
            && effective_args
                .get("command")
                .and_then(|command| command.as_str())
                .is_some_and(is_env_setup_command)
        {
            let result = ENV_FIGHT_BLOCKED_RESULT.to_string();
            self.presenter.emit(PresenterEvent::ToolStart {
                name: call.name.clone(),
                args: args_json.clone(),
            });
            self.presenter.emit(PresenterEvent::ToolResult {
                name: call.name.clone(),
                ok: false,
                summary: "environment setup/build spend cap".to_string(),
            });
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "blocked", "error")?;
            if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result) {
                self.presenter
                    .emit(PresenterEvent::Warning(warning.clone()));
                self.pending_hints.push(warning);
            }
            return Ok(result);
        }

        let Some(tool) = self.tools.get(&call.name) else {
            // Name the valid tools so the model can recover instead of guessing again.
            let mut available: Vec<String> =
                self.tool_specs().into_iter().map(|s| s.name).collect();
            available.sort();
            let result = format!(
                "error: unknown tool '{}'. Available tools: {}",
                call.name,
                available.join(", ")
            );
            self.presenter.emit(PresenterEvent::ToolResult {
                name: call.name.clone(),
                ok: false,
                summary: "unknown tool".to_string(),
            });
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "n/a", "error")?;
            if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result) {
                self.presenter
                    .emit(PresenterEvent::Warning(warning.clone()));
                self.pending_hints.push(warning);
            }
            return Ok(result);
        };

        let side_effect = tool.side_effect();
        self.presenter.emit(PresenterEvent::ToolStart {
            name: call.name.clone(),
            args: args_json.clone(),
        });

        // PreToolUse hooks (hooks.md): run user shell hooks before the tool. A non-zero exit
        // blocks the call (the hook's output is the reason the model sees). Exit 0 + JSON object
        // on stdout rewrites the args before the tool runs. Inert when no hooks configured.
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
                });
                self.store.record_tool_call(
                    msg_id, &call.name, &args_json, &result, "blocked", "error",
                )?;
                return Ok(result);
            }
            if let Some(new_args) = outcome.rewritten_args {
                effective_args = subagent::rewrite_args_for_root(&new_args, self.workspace.root());
                effective_args =
                    add_workspace_default_path(&call.name, effective_args, self.workspace.root());
                args_json = serde_json::to_string(&effective_args).unwrap_or_default();
                if let Err(error) = validate_workspace_args(&effective_args, &self.workspace) {
                    let result = format!("error: {error}");
                    self.presenter.emit(PresenterEvent::ToolResult {
                        name: call.name.clone(),
                        ok: false,
                        summary: "hook rewrote path outside session workspace".to_string(),
                    });
                    self.store.record_tool_call(
                        msg_id, &call.name, &args_json, &result, "denied", "error",
                    )?;
                    if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result)
                    {
                        self.presenter
                            .emit(PresenterEvent::Warning(warning.clone()));
                        self.pending_hints.push(warning);
                    }
                    return Ok(result);
                }
            }
        }

        // Validate the call's arguments against the tool's schema BEFORE running it. A malformed
        // call (missing a required field, or args that aren't an object) otherwise fails deep inside
        // the tool with an opaque message; instead return an actionable error naming what's missing
        // plus the required fields, so the model self-corrects on the next step instead of thrashing.
        if let Err(reason) = validate_tool_args(&tool.schema(), &effective_args) {
            let result = format!("error: invalid arguments for `{}` — {reason}", call.name);
            self.presenter.emit(PresenterEvent::ToolResult {
                name: call.name.clone(),
                ok: false,
                summary: "invalid arguments".to_string(),
            });
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "n/a", "error")?;
            if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result) {
                self.presenter
                    .emit(PresenterEvent::Warning(warning.clone()));
                self.pending_hints.push(warning);
            }
            return Ok(result);
        }

        // For a file-mutating tool, show the proposed change BEFORE the permission gate so
        // the user reviews a diff instead of approving a blind write.
        if side_effect == forge_types::SideEffect::Write {
            if let Some(diff) = tool.preview(&effective_args).await {
                self.presenter.emit(PresenterEvent::Diff(diff));
            }
        }

        let decision = permission::decide(
            self.mode,
            side_effect,
            &call.name,
            &effective_args,
            &self.rules,
        );
        // Notification lifecycle hook (Claude-Code parity): the agent needs the user's attention to
        // approve this tool. Fired just before the prompt is shown (inert when no hooks configured).
        // Inlined with field-level borrows because `tool` holds an immutable borrow of `self.tools`
        // here, so a whole-`self` method call wouldn't borrow-check.
        if matches!(decision, PermissionDecision::Ask) && !self.config.hooks.is_empty() {
            let outcome = hooks::run_lifecycle_hooks(
                &self.config.hooks,
                forge_config::HookEvent::Notification,
                &self.id,
                serde_json::json!({ "message": format!("permission needed: {}", call.name) }),
            )
            .await;
            for n in outcome.notes {
                self.presenter.emit(PresenterEvent::Warning(n));
            }
        }
        let allowed = match decision {
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
        let permission_label = if allowed { "allowed" } else { "denied" };

        // Snapshot the target's pre-edit bytes BEFORE a permitted write, so `/undo` can restore
        // it (PR3 shadow snapshots; first touch per path per turn wins). The target path is read via
        // the centralized `extract_path_arg`, so a write tool naming its arg `file_path`/`target`
        // still gets snapshotted (and is subject to the same secret-deny / permission path logic).
        let write_path = (allowed && side_effect == forge_types::SideEffect::Write)
            .then(|| forge_types::extract_path_arg(&effective_args))
            .flatten()
            .map(std::path::PathBuf::from);
        if let Some(path) = &write_path {
            // Surface a snapshot failure: the write below still proceeds, but `/undo` will NOT be
            // able to restore this file, so the user must be told rather than silently losing the
            // safety net.
            if let Err(e) = snapshot::snapshot_before_write(
                &self.checkpoint_root,
                &self.id,
                self.current_turn_seq,
                path,
            ) {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "could not snapshot {} before writing ({e}) — /undo will not be able to restore this change",
                    path.display()
                )));
            }
        }

        let (result, ok) = if allowed {
            match tool.run(&effective_args).await {
                Ok(out) => {
                    // Record what we wrote, so a later restore can warn on a manual edit.
                    if let Some(path) = &write_path {
                        let _ = snapshot::record_post_write(
                            &self.checkpoint_root,
                            &self.id,
                            self.current_turn_seq,
                            path,
                        );
                        // Count this successful write so the autofix stage knows edits happened.
                        self.edits_this_turn += 1;
                        // Reindex the touched file in-turn so later retrieval/queries this turn
                        // reflect the edit (code-intelligence.md — post-edit freshness).
                        if let Some(lat) = &self.lattice {
                            let _ = lat.reindex_path(path);
                        }
                        // LSP diagnostics: ask the language server for errors on the
                        // just-written file and queue them as a pending hint so the model
                        // self-corrects this turn. Best-effort: missing server → silent.
                        if self.config.lsp.enabled {
                            if let Some(lsp) = &self.lsp {
                                let abs =
                                    std::path::absolute(path).unwrap_or_else(|_| path.clone());
                                let timeout =
                                    std::time::Duration::from_millis(self.config.lsp.timeout_ms);
                                let lsp = Arc::clone(lsp);
                                let diags = lsp.diagnostics_for(&abs, timeout).await;
                                if !diags.is_empty() {
                                    let lines: Vec<String> = diags
                                        .iter()
                                        .map(|d| d.format_line(&path.display().to_string()))
                                        .collect();
                                    self.pending_hints
                                        .push(format!("[lsp diagnostics]\n{}", lines.join("\n")));
                                }
                            }
                        }
                    }
                    (out, true)
                }
                Err(e) => (format!("error: {e}"), false),
            }
        } else {
            ("permission denied by policy".to_string(), false)
        };

        self.presenter.emit(PresenterEvent::ToolResult {
            name: call.name.clone(),
            ok,
            summary: summarize(&result),
        });
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            permission_label,
            if ok { "ok" } else { "error" },
        )?;

        if ok {
            self.failure_tracker.record_success(&call.name);
        } else if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result) {
            self.presenter
                .emit(PresenterEvent::Warning(warning.clone()));
            self.pending_hints.push(warning);
        }

        // PostToolUse hooks (hooks.md): observe the completed call (e.g. re-index, notify). The
        // tool result is already final; post hooks only surface notes, they don't change it.
        if !self.config.hooks.is_empty() {
            let payload =
                serde_json::json!({ "tool": call.name, "args": call.args, "result": result, "ok": ok, "cwd": self.workspace.display() })
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

        // Shell error interceptor (shell-error-interceptor.md): on a failed shell command,
        // auto-explain the likely cause + a fix with one cheap model call. Best-effort, never
        // alters the result the model sees.
        if side_effect == forge_types::SideEffect::Shell
            && self.config.shell.explain_errors
            && shell_command_failed(&result)
        {
            if let Some(command) = call.args.get("command").and_then(|v| v.as_str()) {
                let command = command.to_string();
                self.diagnose_shell_error(&command, &result).await;
            }
        }

        Ok(result)
    }

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
