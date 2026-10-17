//! Session built-in and MCP tool dispatch through permissions, hooks, and audit.
//!
//! This deep owner keeps effect ordering, permission checks, hook processing, and tool-call persistence together.

use super::*;

mod deferred;
mod denial;
mod mcp;
mod permission_hooks;

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

impl Session {
    /// Steer main-checkout paths into a linked worktree session and compute the roots this call
    /// may touch: the configured extras, plus the main checkout and temp dir for read-only tools.
    fn scope_to_workspace(
        &self,
        tool: &str,
        args: serde_json::Value,
    ) -> (serde_json::Value, Vec<std::path::PathBuf>) {
        let read_only = self
            .tools
            .get(tool)
            .is_some_and(|t| t.side_effect() == forge_types::SideEffect::ReadOnly);
        let root = self.workspace.root();
        let args = match forge_tools::worktree_origin(root) {
            Some(origin) => forge_tools::remap_origin_paths(&args, &origin, root, read_only),
            None => args,
        };
        let mut roots = self.extra_tool_roots.clone();
        if read_only {
            roots.extend(forge_tools::read_only_roots(root));
        }
        (args, roots)
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
    ) -> Result<Vec<(String, Option<ErrorCategory>, String)>, CoreError> {
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
        let results: Vec<(String, Option<String>, bool)> = {
            let tools = &self.tools;
            let futs = pend.iter().map(|p| async move {
                if !p.allowed {
                    return ("permission denied by policy".to_string(), None, false);
                }
                match tools.get(&p.name) {
                    Some(tool) => match tool.run_full(&p.args).await {
                        Ok(out) => (out.model, out.full, true),
                        Err(e) => (format!("error: {e}"), None, false),
                    },
                    None => (format!("error: unknown tool '{}'", p.name), None, false),
                }
            });
            futures::future::join_all(futs).await
        };
        // Phase 3 (serial): surface + persist + append each result in the ORIGINAL order, so every
        // tool_call_id is answered in sequence. Also classify each result for the failure-loop guard.
        let mut classified = Vec::with_capacity(pend.len());
        for (p, (result, full, ok)) in pend.iter().zip(results) {
            let shown_ok = ok && !(p.name == "shell" && shell_command_failed(&result));
            self.emit_tool_card(&p.name, shown_ok, &result);
            self.record_test_proof(&p.name, &p.args, &result, full.as_deref());
            if ok {
                self.note_read_files(&p.name, &p.args);
            }
            let kept = self.surface_full_output(&p.name, &p.id, full.as_deref(), &result);
            let result = crate::tool_output::fit_for_model(result, kept.as_deref());
            self.store.record_tool_call(
                msg_id,
                &p.name,
                &p.args_json,
                &result,
                if p.allowed { "allowed" } else { "denied" },
                if ok { "ok" } else { "error" },
            )?;
            classified.push((
                p.name.clone(),
                classify_tool_failure(&result),
                result.clone(),
            ));
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

    /// Run a single tool call, applying the permission policy, and return its result text.
    pub(crate) async fn invoke_tool(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        // Idempotent: the model loop already resolved `tool_call`, so its counters saw the same call.
        let call = &self.resolve_deferred_call(call);
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
            self.emit_tool_card(&call.name, false, &result);
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
        if call.name == fleet::LIST_SESSIONS_TOOL {
            return self.list_sessions(msg_id, call).await;
        }
        // Plan and dispatch: recorded through the host the daemon wires into coordinator sessions.
        if call.name == crate::dispatch::DISPATCH_SESSIONS_TOOL {
            return self.dispatch_sessions(msg_id, call).await;
        }
        // Agent-created heartbeats are core-owned (they persist to the store and are scoped away
        // from the user's own `/heartbeat`, which never goes through tool dispatch at all).
        if call.name == heartbeat::MANAGE_HEARTBEATS_TOOL {
            return self.manage_heartbeats(msg_id, call);
        }
        if call.name == push_notify::PUSH_NOTIFICATION_TOOL {
            return self.push_notification(msg_id, call).await;
        }
        if call.name == wakeup::SCHEDULE_WAKEUP_TOOL {
            return self.schedule_wakeup(msg_id, call);
        }
        if call.name == WORKTREE_TOOL {
            return self.worktree_tool(msg_id, call);
        }
        if call.name == crate::deferred_tools::TOOL_SEARCH
            || call.name == crate::deferred_tools::TOOL_CALL
        {
            return self.invoke_deferred_meta(msg_id, call).await;
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
        let (scoped_args, readable_roots) = self.scope_to_workspace(&call.name, effective_args);
        effective_args = scoped_args;
        let mut args_json = serde_json::to_string(&effective_args)?;
        if let Err(error) = self.check_workspace(&call.name, &effective_args, &readable_roots) {
            let result = format!("error: {error}");
            self.presenter.emit(PresenterEvent::ToolStart {
                name: call.name.clone(),
                args: args_json.clone(),
            });
            self.presenter.emit(PresenterEvent::ToolResult {
                name: call.name.clone(),
                ok: false,
                summary: match error {
                    CoreError::StaleHostPath(_) => "stale path from the previous host",
                    _ => "path outside session workspace",
                }
                .to_string(),
                detail: crate::tool_detail(&result),
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
            && forge_types::shell_command_text(&effective_args)
                .as_deref()
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
                detail: crate::tool_detail(&result),
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

        // Resolved before the registry borrow below: building it may route a side-call model.
        let pre_hook_runner = if self.config.hooks.is_empty() {
            None
        } else {
            self.llm_hook_runner().await
        };
        let Some(tool) = self.tools.get(&call.name) else {
            // Name the valid tools so the model can recover instead of guessing again.
            let mut available: Vec<String> =
                self.full_tool_specs().into_iter().map(|s| s.name).collect();
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
                detail: crate::tool_detail(&result),
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
        let announced = serde_json::to_string(&call.args).unwrap_or_else(|_| args_json.clone());
        self.presenter.emit(PresenterEvent::ToolStart {
            name: call.name.clone(),
            args: announced,
        });

        // PreToolUse hooks (hooks.md): run user shell hooks before the tool. A non-zero exit
        // blocks the call (the hook's output is the reason the model sees). Exit 0 + JSON object
        // on stdout rewrites the args before the tool runs. Inert when no hooks configured.
        if !self.config.hooks.is_empty() {
            let payload = serde_json::json!({
                "tool": call.name, "args": effective_args, "cwd": self.workspace.display(), "session_id": self.id
            })
            .to_string();
            let outcome = crate::hooks_llm::with_runner(
                pre_hook_runner,
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
                effective_args = subagent::rewrite_args_for_root(&new_args, self.workspace.root());
                effective_args =
                    add_workspace_default_path(&call.name, effective_args, self.workspace.root());
                let (scoped_args, readable_roots) =
                    self.scope_to_workspace(&call.name, effective_args);
                effective_args = scoped_args;
                args_json = serde_json::to_string(&effective_args).unwrap_or_default();
                if let Err(error) =
                    self.check_workspace(&call.name, &effective_args, &readable_roots)
                {
                    let result = format!("error: {error}");
                    self.presenter.emit(PresenterEvent::ToolResult {
                        name: call.name.clone(),
                        ok: false,
                        summary: "hook rewrote path outside session workspace".to_string(),
                        detail: crate::tool_detail(&result),
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
        let validation = validate_tool_args(&tool.schema(), &effective_args).and_then(|()| {
            // `command` and `commands` are alternatives, which `required` cannot express without
            // a schema combinator some providers reject; hold the line here instead.
            if call.name == "shell" && forge_types::shell_command_text(&effective_args).is_none() {
                Err("missing required field: command (or commands)".to_string())
            } else {
                Ok(())
            }
        });
        if let Err(reason) = validation {
            // Observed 2026-09-08: a session routed through a Headroom proxy (`[mesh] headroom`)
            // degraded into `{}` tool calls turn after turn — the proxy rewrites prior turns and
            // tool schemas, and the model lost the arguments. Name the likely cause once so the
            // operator can switch the proxy off instead of watching the model retry.
            if forge_config::headroom_route().is_some() && !self.headroom_args_warned {
                self.headroom_args_warned = true;
                self.presenter.emit(PresenterEvent::Warning(
                    "tool call arrived with invalid arguments while requests are routed through \
                     the Headroom proxy — if this repeats, set `[mesh] headroom = \"off\"` \
                     (docs/features/token-savings.md)"
                        .to_string(),
                ));
            }
            let result = format!("error: invalid arguments for `{}` — {reason}", call.name);
            self.presenter.emit(PresenterEvent::ToolResult {
                name: call.name.clone(),
                ok: false,
                summary: "invalid arguments".to_string(),
                detail: crate::tool_detail(&result),
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

        let workspace = self.workspace_binding.read().ok().map(|g| g.clone());
        let (decision, auto_verdict) = permission::decide_in_auto(
            self.mode,
            side_effect,
            &call.name,
            &effective_args,
            &self.rules,
            workspace.as_deref(),
        );
        let decision = self
            .settle_auto(decision, auto_verdict, &effective_args)
            .await;
        let decision = crate::accept_edits_write_gate(decision, self.mode, side_effect, || {
            self.writes_beyond_workspace(&effective_args)
        });
        // PermissionRequest hooks (Claude-Code parity): a hook may answer the prompt for the user.
        let (decision, hook_denial) = self
            .settle_permission_hooks(decision, &call.name, &mut effective_args, &mut args_json)
            .await;
        // Notification lifecycle hook (Claude-Code parity): the agent needs the user's attention to
        // approve this tool. Fired just before the prompt is shown (inert when no hooks configured).
        // Inlined with field-level borrows (the hooks call needs only `config`, `id`, `presenter`).
        if matches!(decision, PermissionDecision::Ask) && !self.config.hooks.is_empty() {
            let runner = self.llm_hook_runner().await;
            let outcome = crate::hooks_llm::with_runner(
                runner,
                hooks::run_lifecycle_hooks(
                    &self.config.hooks,
                    forge_config::HookEvent::Notification,
                    &self.id,
                    serde_json::json!({ "message": format!("permission needed: {}", call.name) }),
                ),
            )
            .await;
            for n in outcome.notes {
                self.presenter.emit(PresenterEvent::Warning(n));
            }
        }
        let mut user_declined = false;
        let allowed = match decision {
            PermissionDecision::Allow => true,
            PermissionDecision::Deny => false,
            PermissionDecision::Ask => match permission::confirm_call(
                self.presenter.as_mut(),
                &call.name,
                &effective_args,
                side_effect,
            ) {
                forge_types::ConfirmOutcome::AlwaysAllow => {
                    // Scope to what the user SAW: an empty list matches every segment (see
                    // decide_shell_segments), so approving `git status` used to auto-approve
                    // every later shell command in the session.
                    let patterns = always_allow_patterns(&call.name, &call.args);
                    // Persisted with the same scope; an unscopable shell call is allowed once.
                    if let Err(e) =
                        forge_config::append_allow_rule_in(self.rules_root(), &call.name, &patterns)
                    {
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "allow rule not saved: {e}"
                        )));
                    }
                    if call.name != "shell" || !patterns.is_empty() {
                        self.rules.push(forge_types::PermissionRule {
                            tool: call.name.clone(),
                            patterns,
                            decision: forge_types::PermissionDecision::Allow,
                            source: forge_types::RuleSource::Configured,
                            reason: Some("user answered 'always' at runtime prompt".into()),
                        });
                    }
                    true
                }
                forge_types::ConfirmOutcome::Allow => true,
                forge_types::ConfirmOutcome::Deny => {
                    user_declined = true;
                    false
                }
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

        let lsp_pre_edit = write_path.as_deref().and_then(|p| self.lsp_pre_edit(p));

        let (result, full, ok) = if allowed {
            // Re-borrowed here: the permission gate above needs `&mut self` (classifier side
            // call), which cannot overlap a registry borrow held since the top of the call.
            let Some(tool) = self.tools.get(&call.name) else {
                unreachable!("tool `{}` resolved above", call.name)
            };
            let run = self.evidence_scope(tool.run_full(&effective_args));
            let outcome = if call.name == "shell"
                && effective_args
                    .get("background")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
            {
                // Only a job THIS call starts reports its exit to this session (see job_wake.rs).
                let wake = self.job_wake.clone();
                forge_tools::scope_job_exit_sink(
                    std::sync::Arc::new(move |exit| wake.push(&exit)),
                    run,
                )
                .await
            } else {
                run.await
            };
            match outcome {
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
                        if let Some(shown) = path.to_str() {
                            self.note_file_use(shown);
                        }
                        // Commit discipline: every N successful edits since the last commit or
                        // reminder, remind the model (right after this result) that its work is
                        // uncommitted. `git status` runs only when the reminder is due.
                        if self.commit_nudges_on()
                            && self.git_hygiene.record_edit(
                                self.workspace.root(),
                                path,
                                self.config.git.commit_nudge_edits,
                            )
                        {
                            if let Some(nudge) =
                                self.git_hygiene.mid_turn_nudge(self.workspace.root())
                            {
                                self.pending_hints.push(nudge.render());
                            }
                        }
                        // Reindex the touched file in-turn so later retrieval/queries this turn
                        // reflect the edit (code-intelligence.md — post-edit freshness).
                        if let Some(lat) = &self.lattice {
                            let _ = lat.reindex_path(path);
                        }
                        // LSP diagnostics for the just-written file (lsp_hints.rs), queued as a
                        // hint so the model self-corrects this turn.
                        self.queue_lsp_hint(path, lsp_pre_edit).await;
                    }
                    (out.model, out.full, true)
                }
                Err(e) => (format!("error: {e}"), None, false),
            }
        } else if self.mode == forge_types::PermissionMode::Plan
            && permission::is_shell_tool(&call.name)
        {
            (permission::plan_shell_denial(), None, false)
        } else if let Some(reason) = hook_denial {
            (
                format!("denied by PermissionRequest hook: {reason}"),
                None,
                false,
            )
        } else if let Some(note) = self.floor_denial(&call.name, &effective_args) {
            (note, None, false)
        } else {
            denial::result(
                &call.name,
                self.mode,
                user_declined,
                self.presenter.is_attended(),
            )
        };
        self.record_test_proof(&call.name, &effective_args, &result, full.as_deref());

        // The call ran, so `ok` stays true for the failure tracker and the commit bookkeeping, but
        // a shell command that exited non-zero must not be drawn with a success mark.
        let shown_ok = ok && !(call.name == "shell" && shell_command_failed(&result));
        self.presenter.emit(PresenterEvent::ToolResult {
            name: call.name.clone(),
            ok: shown_ok,
            summary: summarize(&result),
            detail: crate::tool_detail(&result),
        });
        let kept = self.surface_full_output(&call.name, &call.id, full.as_deref(), &result);
        let result = crate::tool_output::fit_for_model(result, kept.as_deref());
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            permission_label,
            if ok { "ok" } else { "error" },
        )?;

        if ok {
            self.note_read_files(&call.name, &effective_args);
            self.failure_tracker
                .record_success_with(&call.name, &call_args_json, &result);
            // A shell command that may move HEAD is when the commit tracker forgets clean files.
            if self.config.git.commit_nudge
                && call.name == "shell"
                && forge_types::shell_command_text(&call.args)
                    .as_deref()
                    .is_some_and(crate::git_hygiene::may_move_head)
            {
                let root = self.workspace.root();
                let head = crate::git_head(Some(root));
                self.git_hygiene.observe_head(root, head);
            }
        } else if let Some(warning) = self.failure_tracker.record_failure(&call.name, &result) {
            self.presenter
                .emit(PresenterEvent::Warning(warning.clone()));
            self.pending_hints.push(warning);
        }

        // PostToolUse hooks (hooks.md): observe the completed call (e.g. re-index, notify). The
        // tool result is already final; post hooks only surface notes, they don't change it.
        if !self.config.hooks.is_empty() {
            let payload =
                serde_json::json!({ "tool": call.name, "args": effective_args, "result": result, "ok": ok, "cwd": self.workspace.display(), "session_id": self.id })
                    .to_string();
            let runner = self.llm_hook_runner().await;

            let outcome = crate::hooks_llm::with_runner(
                runner,
                hooks::run_hooks(
                    &self.config.hooks,
                    forge_config::HookEvent::PostToolUse,
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
        }

        // Shell error interceptor (shell-error-interceptor.md): on a failed shell command,
        // auto-explain the likely cause + a fix with one cheap model call. Best-effort, never
        // alters the result the model sees.
        if side_effect == forge_types::SideEffect::Shell
            && self.config.shell.explain_errors
            && shell_command_failed(&result)
        {
            if let Some((command, output)) =
                crate::shell_batch::failing_command(&call.args, &result)
            {
                self.diagnose_shell_error(&command, &output).await;
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod always_allow_tests {
    use super::always_allow_patterns;

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
}
