//! Session-owned virtual tools: questions, tasks, plans, memory, and skills.
//!
//! This owner keeps their persistence and presenter events in the same stateful boundary.

use super::*;

impl Session {
    /// Handle an `ask_user` call: parse the question + options, ask the user through the
    /// presenter (interactive multi-choice / open-ended), and return their answer as the tool
    /// result (docs/features/ask-user-question.md).
    pub(crate) fn ask_user(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let question = call
            .args
            .get("question")
            .and_then(|q| q.as_str())
            .unwrap_or("")
            .to_string();
        if question.trim().is_empty() {
            let result = "error: ask_user requires a non-empty `question`".to_string();
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "error")?;
            return Ok(result);
        }
        let options: Vec<forge_types::QChoice> = call
            .args
            .get("options")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        let label = o.get("label").and_then(|l| l.as_str())?;
                        Some(forge_types::QChoice {
                            label: label.to_string(),
                            description: o
                                .get("description")
                                .and_then(|d| d.as_str())
                                .unwrap_or("")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Default to allowing a free-text answer (and force it when there are no options).
        let allow_other = call
            .args
            .get("allow_other")
            .and_then(|a| a.as_bool())
            .unwrap_or(true)
            || options.is_empty();

        let answer = self.presenter.ask(&question, &options, allow_other);
        self.store
            .record_tool_call(msg_id, &call.name, &args_json, &answer, "allowed", "ok")?;
        Ok(answer)
    }

    /// Replace the session's task list (the `update_tasks` virtual tool): parse the full list,
    /// persist it, emit it to the TUI, and return a one-line summary to the model.
    pub(crate) fn update_tasks(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        use forge_types::TodoStatus;
        let args_json = serde_json::to_string(&call.args)?;
        self.tasks = merge_task_update(&self.tasks, parse_tasks(&call.args));
        self.persist_tasks();
        self.presenter
            .emit(PresenterEvent::Tasks(self.tasks.clone()));

        let done = self
            .tasks
            .iter()
            .filter(|t| t.status == TodoStatus::Done)
            .count();
        let in_progress = self
            .tasks
            .iter()
            .filter(|t| t.status == TodoStatus::InProgress)
            .count();
        let result = format!(
            "task list updated: {} task(s) — {done} done, {in_progress} in progress",
            self.tasks.len()
        );
        self.store
            .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "ok")?;
        Ok(result)
    }

    /// Persist and stash a proposed plan for the turn-end approval flow. A proposal is not active
    /// work: its steps become tasks only after the user chooses Build.
    pub(crate) fn ingest_plan(&mut self, plan: forge_types::PlanProposal) {
        // `present_plan` is authoritative even when a one-shot `/plan` expansion reached the
        // provider without going through the interactive command dispatcher. Entering Plan here
        // captures the actual current permission so approval can restore it exactly.
        if self.mode != PermissionMode::Plan {
            self.set_temper(PermissionMode::Plan);
        }
        persist_plan(&self.id, &plan);
        self.pending_plan = Some(plan);
    }

    /// Turn an approved plan into the active, persisted task list.
    pub(crate) fn activate_plan_tasks(&mut self, plan: &forge_types::PlanProposal) {
        self.tasks = plan
            .steps
            .iter()
            .map(|s| forge_types::TodoItem {
                title: s.title.trim().to_string(),
                status: forge_types::TodoStatus::Pending,
                // A `PlanStep` has no owner field, so a seeded task starts unassigned; the model
                // can hand one out later via `update_tasks` if it delegates the step.
                assignee: None,
            })
            .collect();
        self.persist_tasks();
        self.presenter
            .emit(PresenterEvent::Tasks(self.tasks.clone()));
    }

    /// Leave read-only planning through the approval flow. New sessions restore the exact mode that
    /// entered Plan; a resumed/legacy Plan session has no in-memory predecessor and uses Auto-edit.
    pub(crate) fn restore_pre_plan_temper(&mut self) -> PermissionMode {
        let mode = self
            .pre_plan_mode
            .take()
            .unwrap_or(PermissionMode::AcceptEdits);
        self.set_temper(mode)
    }

    /// Persist the current task list, surfacing a write failure as a Warning instead of silently
    /// swallowing it. A silently-dropped task write means a resumed session's completion gate (which
    /// reloads tasks from the store) would judge against a stale list — so the user must be told.
    pub(crate) fn persist_tasks(&mut self) {
        if let Err(e) = self.store.set_tasks(&self.id, &self.tasks) {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "could not persist the task list ({e}) — it may not survive a resume; the \
                 completion gate could judge against a stale list"
            )));
        }
    }

    /// Ask the user to approve a proposed plan (called at turn end, after the model loop, so it's
    /// safe to block on the presenter). Returns the follow-up prompt to run next — the build prompt
    /// (after switching to Auto-edit) or a revision prompt — or `None` to cancel (stay in planning).
    pub(crate) fn resolve_plan_approval(
        &mut self,
        plan: &forge_types::PlanProposal,
    ) -> Option<String> {
        let n = plan.steps.len();
        let q = format!(
            "Build this plan? — \"{}\" ({n} step{}). Choose Build it / Cancel, or type changes to revise.",
            plan.title.trim(),
            if n == 1 { "" } else { "s" }
        );
        let build_mode = self
            .pre_plan_mode
            .unwrap_or(PermissionMode::AcceptEdits)
            .label();
        let opts = [
            forge_types::QChoice {
                label: "Build it".into(),
                description: format!("Return to {build_mode} and implement the plan now"),
            },
            forge_types::QChoice {
                label: "Cancel".into(),
                description: "Discard the plan; stay in planning mode".into(),
            },
        ];
        let ans = self.presenter.ask(&q, &opts, true);
        let a = ans.trim();
        if a.eq_ignore_ascii_case("Build it")
            || a.eq_ignore_ascii_case("build")
            || a.eq_ignore_ascii_case("yes")
        {
            self.activate_plan_tasks(plan);
            let label = self.restore_pre_plan_temper().label();
            self.presenter
                .emit(PresenterEvent::Temper(label.to_string()));
            self.presenter.emit(PresenterEvent::Warning(format!(
                "plan approved — building with {label} permissions"
            )));
            Some(PLAN_BUILD_PROMPT.to_string())
        } else if a.is_empty()
            || a == forge_types::NO_ANSWER
            || a.eq_ignore_ascii_case("Cancel")
            || a.eq_ignore_ascii_case("no")
        {
            let label = self.restore_pre_plan_temper().label();
            self.presenter
                .emit(PresenterEvent::Temper(label.to_string()));
            self.presenter.emit(PresenterEvent::Warning(format!(
                "plan cancelled — restored {label} permissions"
            )));
            None
        } else {
            // Free-text feedback → revise. Stay in planning mode so present_plan remains available.
            Some(format!(
                "The user did not approve the plan yet. They want these changes before building:\n\n\
                 {a}\n\nRevise the plan accordingly and call present_plan again with the updated steps."
            ))
        }
    }

    /// The current task list (for the composition root / TUI to render on resume).
    pub fn tasks(&self) -> &[forge_types::TodoItem] {
        &self.tasks
    }

    /// Present a plan for review (the `present_plan` virtual tool, planning mode). Renders the plan
    /// card, seeds the live task list from its steps, persists it to `.forge/plans/`, and stashes it
    /// for the turn-end approval flow. Returns a result that tells the model to STOP — the user
    /// approves it interactively (and on approval is switched to Auto-edit to build).
    pub(crate) fn present_plan(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let plan = parse_plan(&call.args);
        if plan.steps.is_empty() {
            let result = "error: present_plan requires a non-empty `steps` array".to_string();
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "error")?;
            return Ok(result);
        }
        // Render the card now (in-process path); the bridge path emits this from the sink instead.
        self.presenter
            .emit(PresenterEvent::PlanProposed(plan.clone()));
        // Persist + seed tasks + stash for the turn-end approval flow (shared with the bridge path).
        self.ingest_plan(plan);
        let result = "Plan presented to the user for approval. STOP now — do NOT start \
                      implementing. The user will review the plan and decide; if they approve, \
                      you'll be switched to Auto-edit and asked to build it."
            .to_string();
        self.store
            .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "ok")?;
        Ok(result)
    }

    /// Load a Forge skill's methodology (the `use_skill` virtual tool) and return it as the tool
    /// result so the model applies it this turn. Unknown name → an error listing valid skills.
    pub(crate) async fn remember(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let kind_raw = call
            .args
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("fact");
        let text = call
            .args
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let kind_norm = kind_raw.trim().to_lowercase();
        let kind_cat = match kind_norm.as_str() {
            "preference" | "decision" | "fact" | "reference" => kind_norm.clone(),
            _ => "fact".to_string(),
        };
        let (result, ok) = if text.len() < 4 {
            (
                "error: memory text too short (minimum 4 characters)".to_string(),
                false,
            )
        } else {
            let scope = memory_scope_at(self.workspace.root());
            let cfg = self.config.lattice.embeddings.clone();
            match embed_one(&cfg, &text).await {
                Some(emb) => {
                    let _ = self
                        .store
                        .add_memory_with_embedding(&scope, &kind_cat, &text, &self.id, &emb);
                }
                None => {
                    let _ = self.store.add_memory(&scope, &kind_cat, &text, &self.id);
                }
            }
            self.presenter
                .emit(PresenterEvent::Warning(format!("◈ memory · {kind_cat}")));
            (format!("memory saved: [{kind_cat}] {text}"), true)
        };
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }

    pub(crate) fn use_skill(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let name = call
            .args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let (result, ok) = match self.skills.as_ref().and_then(|c| c.skill_guidance(name)) {
            Some(guidance) => {
                self.presenter
                    .emit(PresenterEvent::Warning(format!("⚒ skill loaded · {name}")));
                (
                    format!("Loaded the '{name}' skill. Apply this methodology now:\n\n{guidance}"),
                    true,
                )
            }
            None => {
                let available = self
                    .skills
                    .as_ref()
                    .map(|c| {
                        c.skill_listing()
                            .into_iter()
                            .map(|(n, _)| n)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                (
                    format!("no Forge skill named '{name}'. Available: {available}"),
                    false,
                )
            }
        };
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }
}

/// The on-demand memory-write virtual tool name.
pub const REMEMBER_TOOL: &str = "remember";

/// The `ToolSpec` advertised to the model for [`REMEMBER_TOOL`].
pub fn remember_spec() -> ToolSpec {
    ToolSpec {
        name: REMEMBER_TOOL.to_string(),
        description: "Persist a durable fact to memory so it's available in future sessions. \
            Use proactively when you learn something worth remembering: a project decision, user \
            preference, key architecture fact, or stable reference. Kind must be one of \
            `preference`, `decision`, `fact`, or `reference`."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["preference", "decision", "fact", "reference"],
                    "description": "memory category"
                },
                "text": {
                    "type": "string",
                    "description": "the fact to remember (1–2 sentences max)"
                }
            },
            "required": ["kind", "text"]
        }),
    }
}

/// The interactive-question virtual tool name (AskUserQuestion).
pub(crate) const ASK_USER_TOOL: &str = "ask_user";
