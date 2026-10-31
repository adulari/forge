//! Session subagent, workflow, and duel orchestration.
//!
//! This owner preserves parent-child persistence and presenter ordering across delegated work.

use super::*;
use crate::detached_subagents::lifecycle_event;

impl Session {
    /// The subagent types this session's `spawn_agents` can use: Forge's built-ins overlaid by the
    /// user's and the project's agent files (`/agents` lists them).
    pub fn agent_types(&self) -> std::collections::HashMap<String, forge_config::AgentDef> {
        forge_config::load_agents_layered(
            self.workspace.root(),
            &self.config.mesh.subagents.agents_dir,
        )
    }

    /// Handle a `spawn_agents` call: resolve each requested child against the loaded agent
    /// types, then run them **concurrently** (bounded by `max_concurrency`), each in its own
    /// mesh-routed, persisted child session. Children run on tokio tasks (they share the
    /// parent's `Arc` backends); since the presenter is single-threaded, each child reports its
    /// lifecycle over a channel that this method drains on the main task — so `SubagentResult`
    /// events surface live as children finish (RFC subagent-orchestration, Phase 2).
    pub(crate) async fn spawn_agents(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        if !self.config.mesh.subagents.enabled {
            return Ok("spawn_agents is disabled ([mesh.subagents] enabled = false)".to_string());
        }
        if self.delegation_off {
            return Ok(
                "delegation is switched off for this thread by the user; do the work yourself \
                 with your own tools"
                    .to_string(),
            );
        }
        let args_json = serde_json::to_string(&call.args)?;
        let max = self.config.mesh.subagents.max_agents;
        let requests = match subagent::parse_requests(&call.args, max) {
            Ok(r) => r,
            Err(msg) => {
                let result = format!("error: {msg}");
                self.store.record_tool_call(
                    msg_id, &call.name, &args_json, &result, "allowed", "error",
                )?;
                return Ok(result);
            }
        };

        // Budget snapshot so children also down-tier when the day/week/month is under pressure.
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd()?,
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd()?,
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd()?,
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };

        let agents = Arc::new(forge_config::load_agents_layered(
            self.workspace.root(),
            &self.config.mesh.subagents.agents_dir,
        ));
        let repo_root = self.workspace.root().to_path_buf();
        let ctx = subagent::AgentCtx {
            provider: Arc::clone(&self.provider),
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            pricing: self.pricing.clone(),
            mode: self.mode,
            rules: self.rules.clone(),
            depth: 0,
            max_depth: self.config.mesh.subagents.max_depth,
            agents,
            worktree_root: None,
            repo_root,
            effective_pin: self.effective_pin(),
            inherit_pin: !self.subagents_free(),
            asker: None,
        };
        let parent_id = self.id.clone();
        let max_concurrency = self.config.mesh.subagents.max_concurrency;

        // Detached admission (RFC retained-async-subagents): `detached: true` skips the whole
        // blocking fan-out below — admit each child, hand back its id/name/model immediately, and
        // let it run to completion independently. Its result reaches the parent later via
        // `deliver_pending_detached_results` at the next turn boundary.
        if subagent::parse_detached_flag(&call.args) {
            let result = match subagent::spawn_detached(
                &ctx,
                &self.detached_registry,
                &parent_id,
                requests,
                budget,
                self.detached_sink(),
            )
            .await
            {
                Ok(handles) => subagent::format_admission(&handles),
                Err(e) => format!("error: {e}"),
            };
            let ok = !result.starts_with("error:");
            self.store.record_tool_call(
                msg_id,
                &call.name,
                &args_json,
                &result,
                "allowed",
                if ok { "ok" } else { "error" },
            )?;
            return Ok(result);
        }

        // Drive the shared orchestrator, turning each child lifecycle into a presenter event
        // (running children animate live; completed ones fold into the scrollback box).
        // A child's `Ask` decision is hosted on this session's prompt while it waits in here; an
        // unattended surface has no one to ask, so the child's call is refused with a reason.
        let (asker, mut questions) = crate::child_ask::channel_asker();
        let ctx = subagent::AgentCtx {
            asker: self.presenter.is_attended().then_some(asker),
            ..ctx
        };
        let (combined, all_ok) = {
            let presenter = std::sync::Mutex::new(&mut self.presenter);
            let mut on_event = |ev: subagent::Lifecycle| {
                if let Ok(mut p) = presenter.lock() {
                    p.emit(lifecycle_event(ev));
                }
            };
            let orchestrated = subagent::orchestrate(
                &ctx,
                &parent_id,
                requests,
                budget,
                max_concurrency,
                &mut on_event,
            );
            tokio::pin!(orchestrated);
            let mut granted = crate::child_ask::Granted::default();
            loop {
                tokio::select! {
                    done = &mut orchestrated => break done?,
                    Some((question, reply)) = questions.recv() => {
                        let _ = reply.send(crate::child_ask::answer(&presenter, &mut granted, &question));
                    }
                }
            }
        };

        // SubagentStop lifecycle hook (Claude-Code parity): the spawned child agent(s) finished.
        // Enforce a block decision at the subagent boundary: this `spawn_agents` call returns a tool
        // result that the PARENT model loop reacts to, so a hook that blocks ("don't let the
        // subagents stop yet") has its reason appended to that result — feeding the continuation
        // signal back into the loop that's actually running, instead of merely noting it. Bounded
        // by construction (a single append; the parent decides what to do next — no auto re-spawn),
        // so there's no risk of an unbounded re-run loop here.
        let stop_outcome = self
            .fire_lifecycle(
                forge_config::HookEvent::SubagentStop,
                serde_json::json!({ "ok": all_ok }),
            )
            .await;
        let combined = match stop_outcome.blocked {
            Some(reason) => {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "subagent_stop hook requested continuation: {reason}"
                )));
                let note = format!("[subagent_stop hook] {reason}");
                if crate::subagent::structured::is_envelope(&combined) {
                    crate::subagent::structured::add_note(&combined, &note)
                } else {
                    format!("{combined}\n\n{note}")
                }
            }
            None => combined,
        };

        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &combined,
            "allowed",
            if all_ok { "ok" } else { "error" },
        )?;
        Ok(combined)
    }

    /// Handle a `send_to_agent` call: follow up with a child spawned earlier — this turn, a
    /// previous turn, or before a resume — by rebuilding its persisted transcript and running
    /// the same child loop again (persistent subagents, gap-analysis #12). The child keeps its
    /// full prior context; the depth-1 guard stays structural (children never see this tool).
    pub(crate) async fn send_to_agent(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let fail = |result: String, store: &Store| -> Result<String, CoreError> {
            store.record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "error")?;
            Ok(result)
        };
        let address = call
            .args
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let message = call
            .args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if address.is_empty() || message.is_empty() {
            return fail(
                "error: send_to_agent needs both `agent` (name or id prefix) and `message`".into(),
                &self.store,
            );
        }
        let children = self.store.named_child_sessions(&self.id)?;
        let Some((child_id, agent_name)) = subagent::resolve_child_address(&children, &address)
        else {
            let known: Vec<String> = children
                .iter()
                .map(|(id, t)| format!("{} ({})", t.as_deref().unwrap_or("unnamed"), &id[..8]))
                .collect();
            return fail(
                format!(
                    "error: no child agent matches '{address}'. Children this session: [{}] — \
                     spawn one first with spawn_agents",
                    known.join(", ")
                ),
                &self.store,
            );
        };

        let outcome = self
            .follow_up_child(&child_id, &agent_name, &message, true)
            .await?;
        let label = if agent_name.is_empty() {
            child_id[..8].to_string()
        } else {
            agent_name
        };
        let result = format!("[{label}] {}", outcome.final_text);
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if outcome.ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }

    /// Run `message` as a follow-up turn of an existing child session: the persisted transcript
    /// is rebuilt, the agent definition re-resolved by name, and the same child loop runs again.
    /// `announce` emits the `SubagentStart` that gives a model-driven `send_to_agent` its own
    /// panel row; a user typing into an open child view already has one.
    pub(crate) async fn follow_up_child(
        &mut self,
        child_id: &str,
        agent_name: &str,
        message: &str,
        announce: bool,
    ) -> Result<subagent::SubagentOutcome, CoreError> {
        // Re-resolve the agent definition by its recorded name so a named type keeps its
        // persona + toolset; the follow-up message becomes the routed "task".
        let agents = Arc::new(forge_config::load_agents_layered(
            self.workspace.root(),
            &self.config.mesh.subagents.agents_dir,
        ));
        let request = subagent::AgentRequest {
            agent: agent_name.to_string(),
            task: message.to_string(),
            ..Default::default()
        };
        let resolved = subagent::resolve(&request, &agents);
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd()?,
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd()?,
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd()?,
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        let repo_root = self.workspace.root().to_path_buf();
        let ctx = subagent::AgentCtx {
            provider: Arc::clone(&self.provider),
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            pricing: self.pricing.clone(),
            mode: self.mode,
            rules: self.rules.clone(),
            depth: 0,
            max_depth: self.config.mesh.subagents.max_depth,
            agents,
            worktree_root: None,
            repo_root,
            effective_pin: self.effective_pin(),
            inherit_pin: !self.subagents_free(),
            asker: None,
        };
        let decision = subagent::route_child(&ctx, &resolved, budget).await;

        if announce {
            self.presenter.emit(PresenterEvent::SubagentStart {
                id: child_id.to_string(),
                agent: agent_name.to_string(),
                task: format!("↩ {message}"),
                model: Some(decision.model.clone()),
                phase: None,
            });
        }
        let presenter = &mut self.presenter;
        let mut on_delta = |ev: StreamEvent| {
            if let StreamEvent::Text(snippet) | StreamEvent::Reasoning(snippet) = ev {
                presenter.emit(PresenterEvent::SubagentProgress {
                    id: child_id.to_string(),
                    snippet,
                });
            }
        };
        let outcome = subagent::resume_subagent(
            &ctx,
            child_id,
            &resolved,
            message,
            decision,
            budget,
            &mut on_delta,
        )
        .await?;
        let cost = self.store.session_cost(child_id).unwrap_or(0.0);
        self.presenter.emit(PresenterEvent::SubagentResult {
            id: child_id.to_string(),
            agent: agent_name.to_string(),
            ok: outcome.ok,
            summary: outcome.final_text.clone(),
            cost_usd: cost,
        });
        Ok(outcome)
    }

    /// Send a message the user typed in a subagent's session view to that subagent. `child_id`
    /// must be one of this session's children.
    pub async fn message_child(
        &mut self,
        child_id: &str,
        message: &str,
    ) -> Result<subagent::SubagentOutcome, CoreError> {
        let children = self.store.named_child_sessions(&self.id)?;
        let Some((_, title)) = children.into_iter().find(|(id, _)| id == child_id) else {
            return Err(CoreError::Precondition(format!(
                "{child_id} is not a subagent of this session"
            )));
        };
        self.follow_up_child(child_id, title.as_deref().unwrap_or(""), message, false)
            .await
    }

    /// Handle a `run_workflow` call: build the shared mesh-routed execution context (same shape
    /// as `spawn_agents`') and hand the script off to `workflow::run`, converting its
    /// `WorkflowEvent`s into the same `SubagentStart`/`Progress`/`Result` presenter events
    /// `spawn_agents` uses (docs/rfcs/forge-workflow.md) — one flat activity feed either way.
    pub(crate) async fn run_workflow(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let script_body = match call.args.get("script").and_then(|s| s.as_str()) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                let result = "error: run_workflow requires a non-empty `script` string".to_string();
                self.store.record_tool_call(
                    msg_id, &call.name, &args_json, &result, "allowed", "error",
                )?;
                return Ok(result);
            }
        };

        let budget = self.budget_snapshot();
        let agents = Arc::new(forge_config::load_agents_layered(
            self.workspace.root(),
            &self.config.mesh.subagents.agents_dir,
        ));
        let repo_root = self.workspace.root().to_path_buf();
        let ctx = subagent::AgentCtx {
            provider: Arc::clone(&self.provider),
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            pricing: self.pricing.clone(),
            mode: self.mode,
            rules: self.rules.clone(),
            depth: 0,
            max_depth: self.config.mesh.subagents.max_depth,
            agents,
            worktree_root: None,
            repo_root: repo_root.clone(),
            effective_pin: self.effective_pin(),
            inherit_pin: !self.subagents_free(),
            asker: None,
        };
        let workflows_dir = repo_root.join(".forge").join("workflows");

        // Bracket the run: Started/Finished tell the TUI a workflow owns the Subagent* events in
        // between, so they render in the dedicated workflow view, not the subagent activity panel.
        self.presenter
            .emit(PresenterEvent::WorkflowStarted { name: None });
        let presenter = &mut self.presenter;
        let on_event = |ev: workflow::WorkflowEvent| match ev {
            workflow::WorkflowEvent::AgentStart {
                id,
                agent,
                task,
                model,
                phase,
            } => presenter.emit(PresenterEvent::SubagentStart {
                id,
                agent,
                task,
                model: Some(model),
                phase,
            }),
            workflow::WorkflowEvent::AgentProgress { id, snippet } => {
                presenter.emit(PresenterEvent::SubagentProgress { id, snippet })
            }
            workflow::WorkflowEvent::AgentDone {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => presenter.emit(PresenterEvent::SubagentResult {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            }),
            workflow::WorkflowEvent::Phase(title) => {
                presenter.emit(PresenterEvent::WorkflowPhase { title })
            }
            workflow::WorkflowEvent::Log(msg) => presenter.emit(PresenterEvent::WorkflowLog(msg)),
        };

        let (value, all_ok) = workflow::run(
            ctx,
            self.id.clone(),
            budget,
            self.config.mesh.subagents.max_concurrency,
            self.config.mesh.subagents.max_per_provider,
            self.config.mesh.workflows.max_total_agents,
            workflows_dir,
            &script_body,
            on_event,
        )
        .await
        .map_err(CoreError::Internal)?;

        let combined = match &value {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
        };
        self.presenter.emit(PresenterEvent::WorkflowFinished {
            ok: all_ok,
            summary: workflow::summary(&combined),
        });
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &combined,
            "allowed",
            if all_ok { "ok" } else { "error" },
        )?;
        Ok(combined)
    }

    /// Run a saved `.forge/workflows/<name>.js` script directly — the `/workflow run <name>
    /// [args]` path (docs/rfcs/forge-workflow.md), which skips the authoring turn entirely (no
    /// model call decides the script). `args` is passed through as-is; the CLI passes the raw
    /// user-typed string, wrapped as a JSON string value so a script can reference it via the
    /// `args` global exactly like `workflow(name, args)` calls from inside another script would.
    pub async fn run_saved_workflow(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<String, CoreError> {
        // Slash commands bypass `run_turn`, so without explicit persistence a completed saved
        // workflow replay contained no prompt and none of its phases/results. Record the command
        // plus a compact audit transcript on the parent session.
        let command = match args.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => format!("/workflow run {name} {raw}"),
            None => format!("/workflow run {name}"),
        };
        let command_seq = self.next_seq();
        self.store
            .add_ui_note(&self.id, command_seq, Role::User, &command)?;

        let budget = self.budget_snapshot();
        let agents = Arc::new(forge_config::load_agents_layered(
            self.workspace.root(),
            &self.config.mesh.subagents.agents_dir,
        ));
        let repo_root = self.workspace.root().to_path_buf();
        let ctx = subagent::AgentCtx {
            provider: Arc::clone(&self.provider),
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            pricing: self.pricing.clone(),
            mode: self.mode,
            rules: self.rules.clone(),
            depth: 0,
            max_depth: self.config.mesh.subagents.max_depth,
            agents,
            worktree_root: None,
            repo_root: repo_root.clone(),
            effective_pin: self.effective_pin(),
            inherit_pin: !self.subagents_free(),
            asker: None,
        };
        let workflows_dir = repo_root.join(".forge").join("workflows");
        if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
            return Err(CoreError::Precondition(format!(
                "workflow name '{name}' must be a plain filename with no path separators"
            )));
        }

        // Same Started/Finished bracket as the `run_workflow` tool path, carrying the saved name.
        self.presenter.emit(PresenterEvent::WorkflowStarted {
            name: Some(name.to_string()),
        });
        // Persisted run history (the workflow library's "past runs" strip). Only this path records:
        // the `run_workflow` tool authors an anonymous script, which belongs to no saved workflow
        // and so has no history to belong to. The guard closes the row out as `interrupted` if this
        // future is dropped mid-run (Esc aborts the whole turn task — no completion path runs).
        let run_id = forge_types::new_id();
        self.store
            .start_workflow_run(&run_id, name, &self.id, &repo_root.display().to_string())?;
        let mut run_guard = workflow::WorkflowRunGuard {
            store: Arc::clone(&self.store),
            id: Some(run_id.clone()),
        };
        let stats = Arc::new(std::sync::Mutex::new(workflow::WorkflowRunStats::default()));
        let stats_events = Arc::clone(&stats);
        let audit = Arc::new(std::sync::Mutex::new(vec![format!(
            "⛓ workflow '{name}' started"
        )]));
        let audit_events = Arc::clone(&audit);
        let presenter = &mut self.presenter;
        let on_event = move |ev: workflow::WorkflowEvent| match ev {
            workflow::WorkflowEvent::AgentStart {
                id,
                agent,
                task,
                model,
                phase,
            } => {
                audit_events
                    .lock()
                    .unwrap()
                    .push(format!("├─ [{agent}] started: {task}"));
                stats_events.lock().unwrap().agents += 1;
                presenter.emit(PresenterEvent::SubagentStart {
                    id,
                    agent,
                    task,
                    model: Some(model),
                    phase,
                });
            }
            workflow::WorkflowEvent::AgentProgress { id, snippet } => {
                presenter.emit(PresenterEvent::SubagentProgress { id, snippet })
            }
            workflow::WorkflowEvent::AgentDone {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => {
                audit_events.lock().unwrap().push(format!(
                    "├─ {} [{agent}] {summary}",
                    if ok { "✓" } else { "✗" }
                ));
                stats_events.lock().unwrap().cost_usd += cost_usd;
                presenter.emit(PresenterEvent::SubagentResult {
                    id,
                    agent,
                    ok,
                    summary,
                    cost_usd,
                });
            }
            workflow::WorkflowEvent::Phase(title) => {
                audit_events
                    .lock()
                    .unwrap()
                    .push(format!("▶ phase: {title}"));
                stats_events.lock().unwrap().phases += 1;
                presenter.emit(PresenterEvent::WorkflowPhase { title })
            }
            workflow::WorkflowEvent::Log(msg) => {
                audit_events.lock().unwrap().push(format!("💬 {msg}"));
                presenter.emit(PresenterEvent::WorkflowLog(msg));
            }
        };

        let run_result = workflow::run_saved(
            ctx,
            self.id.clone(),
            budget,
            self.config.mesh.subagents.max_concurrency,
            self.config.mesh.subagents.max_per_provider,
            self.config.mesh.workflows.max_total_agents,
            workflows_dir,
            name,
            args,
            on_event,
        )
        .await;

        let (value, all_ok) = match run_result {
            Ok(result) => result,
            Err(error) => {
                let summary = format!("'{name}': {error}");
                self.presenter.emit(PresenterEvent::WorkflowFinished {
                    ok: false,
                    summary: summary.clone(),
                });
                run_guard.finished();
                stats
                    .lock()
                    .unwrap()
                    .record(&self.store, &run_id, false, &summary);
                audit
                    .lock()
                    .unwrap()
                    .push(format!("⛓ ✗ workflow failed: {summary}"));
                let text = audit.lock().unwrap().join("\n");
                let seq = self.next_seq();
                self.store
                    .add_ui_note(&self.id, seq, Role::Assistant, &text)?;
                return Err(CoreError::Internal(error));
            }
        };

        let combined = match value {
            serde_json::Value::String(s) => s,
            other => serde_json::to_string(&other).unwrap_or_else(|_| other.to_string()),
        };
        // Unlike the `run_workflow` tool (whose return value the model reads and relays), a saved
        // script run directly via `/workflow run` has no model in the loop — the Finished event's
        // summary is the only surfacing of the script's own return value.
        self.presenter.emit(PresenterEvent::WorkflowFinished {
            ok: all_ok,
            summary: format!("'{name}': {}", workflow::summary(&combined)),
        });
        run_guard.finished();
        stats
            .lock()
            .unwrap()
            .record(&self.store, &run_id, all_ok, &workflow::summary(&combined));
        audit.lock().unwrap().push(format!(
            "⛓ {} workflow finished: '{name}': {}",
            if all_ok { "✓" } else { "✗" },
            workflow::summary(&combined)
        ));
        let audit_text = audit.lock().unwrap().join("\n");
        let audit_seq = self.next_seq();
        self.store
            .add_ui_note(&self.id, audit_seq, Role::Assistant, &audit_text)?;
        Ok(combined)
    }

    /// Run `/duel <task>`: race up to `duel::MAX_CANDIDATES` mesh models on the SAME task, each in
    /// its own isolated worktree (docs/features/duel.md). Unlike `run_workflow`/`spawn_agents`, the
    /// result isn't a single tool answer for a model to read — it's a report plus the still-alive
    /// worktree guards, returned to the CALLER (the TUI) so it can show a picker over the
    /// candidates and merge the winner back once the user decides. Lifecycle events reuse the same
    /// `Subagent*` presenter events `spawn_agents` uses, so a duel shows up in the same activity
    /// panel.
    pub async fn run_duel(
        &mut self,
        spec: &crate::duel_judge::DuelSpec,
    ) -> Result<(duel::DuelReport, Vec<worktree::WorktreeGuard>), CoreError> {
        let budget = self.budget_snapshot();
        let agents = Arc::new(forge_config::load_agents_layered(
            self.workspace.root(),
            &self.config.mesh.subagents.agents_dir,
        ));
        let repo_root = self.workspace.root().to_path_buf();
        let ctx = subagent::AgentCtx {
            provider: Arc::clone(&self.provider),
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            pricing: self.pricing.clone(),
            mode: self.mode,
            rules: self.rules.clone(),
            depth: 0,
            max_depth: self.config.mesh.subagents.max_depth,
            agents,
            worktree_root: None,
            repo_root,
            effective_pin: self.effective_pin(),
            inherit_pin: !self.subagents_free(),
            asker: None,
        };
        let parent_id = self.id.clone();

        let presenter = &mut self.presenter;
        let mut on_event = |ev: subagent::Lifecycle| match ev {
            subagent::Lifecycle::Start {
                id,
                agent,
                task,
                model,
            } => presenter.emit(PresenterEvent::SubagentStart {
                id: id.to_string(),
                agent: agent.to_string(),
                task: task.to_string(),
                model: Some(model.to_string()),
                phase: Some("duel".to_string()),
            }),
            subagent::Lifecycle::Progress { id, snippet } => {
                presenter.emit(PresenterEvent::SubagentProgress {
                    id: id.to_string(),
                    snippet: snippet.to_string(),
                })
            }
            subagent::Lifecycle::Done {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => presenter.emit(PresenterEvent::SubagentResult {
                id: id.to_string(),
                agent: agent.to_string(),
                ok,
                summary: summary.to_string(),
                cost_usd,
            }),
        };

        duel::run(&ctx, &parent_id, budget, spec, &mut on_event).await
    }
}
