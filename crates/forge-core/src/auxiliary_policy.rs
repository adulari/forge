//! Best-effort session side-call policy.
//!
//! This owner keeps related session invariants together behind the Session program.

use super::*;

fn report_auxiliary_persistence_failure(
    session_id: &str,
    purpose: &str,
    error: &dyn std::fmt::Display,
    emit: impl FnOnce(String),
) {
    tracing::warn!(
        session_id,
        purpose,
        error = %error,
        "failed to persist auxiliary call result"
    );
    emit(format!(
        "auxiliary {purpose} result was not persisted: {error}"
    ));
}

impl Session {
    const RECAP_SYSTEM: &'static str = "You are a one-line summarizer for a coding assistant. \
Given the user's request and the assistant's response, write a SINGLE sentence (≤12 words, \
past tense, no punctuation at end) describing ONLY what the assistant's RESPONSE actually shows it \
did — never assume the request was fulfilled. If the response does not clearly show completed \
work (it stalled, errored, only planned, or asked a question), say that instead (e.g. \
\"stalled without completing the task\"). Do not invent success. \
Output ONLY that sentence — no preamble, no quotation marks.";

    const SUGGEST_SYSTEM: &'static str = "You are predicting a coding assistant user's NEXT \
prompt, Claude-Code-style. Given the user's last request and the tail of the assistant's \
response, propose the SINGLE most likely next thing this user will ask for: a short imperative \
instruction, ≤120 characters, no quotation marks, no markdown, no preamble. Output ONLY the \
prompt text, nothing else.";

    /// After a turn completes, make one cheap trivial-tier call to generate a one-line recap,
    /// emitted via [`PresenterEvent::Recap`]. Best-effort: silently skipped on budget exhaustion
    /// or any model error so it can never derail the session.
    const MEMORY_CAPTURE_SYSTEM: &'static str =
        "You extract DURABLE facts worth remembering across FUTURE sessions in this project: user \
         preferences, project decisions/conventions, key architecture or config, and stable \
         constraints. Output 0 to 3 lines, each exactly `kind: fact`, where kind is one of \
         preference, decision, fact, reference. Skip transient task details, one-off actions, and \
         anything specific to only this turn. If nothing is durable, output NOTHING at all.";

    /// After a turn, make one cheap trivial-tier call to extract 0-3 DURABLE facts and persist them
    /// as project-scoped memories (dedup + salience handled by the store). Best-effort: any
    /// budget/model failure is silently skipped so it can never derail the session. Recall of these
    /// happens at the start of a later session (see `run_turn_with`).
    // Spawns memory capture so it doesn't block turn completion — the spinner clears when the AI
    // response finishes. Returns a JoinHandle so the caller can await it in one-shot mode (forge
    // run) before the process exits; interactive turns drop the handle and it runs in background.
    pub(crate) fn capture_memories(
        &self,
        prompt: &str,
        final_text: &str,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.mesh.auto_memory || final_text.trim().is_empty() {
            return None;
        }
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        if budget.status() == BudgetStatus::Exhausted {
            return None;
        }
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let quota = readiness.quota;
        let provider = self.provider.clone();
        let store = self.store.clone();
        let router = self.router.clone();
        let id = self.id.clone();
        let config = self.config.clone();
        let pinned_effort = self.pinned_effort;
        let project = self.project.clone();
        let user_snippet: String = prompt.chars().take(500).collect();
        let assistant_snippet: String = final_text.chars().take(1200).collect();
        let workspace = self.workspace.clone();
        let mut warning_sink = self.presenter.recap_sink();
        Some(tokio::spawn(async move {
            let decision = router
                .route_hinted(
                    "extract durable facts",
                    false,
                    budget,
                    &health,
                    &quota,
                    Some(TaskTier::Trivial),
                    pinned_effort,
                    &project,
                )
                .await;
            if forge_provider::is_cli_bridge(&decision.model) {
                return;
            }
            let messages = vec![
                Message::system(Session::MEMORY_CAPTURE_SYSTEM),
                Message::user(format!(
                    "User request:\n{user_snippet}\n\nAssistant response:\n{assistant_snippet}"
                )),
            ];
            let mut on_event = |_: StreamEvent| {};
            let completion_opts = Session::auxiliary_completion_options(&id, "memory");
            let Ok(r) = provider
                .complete_with(
                    &decision.model,
                    &messages,
                    &[],
                    &completion_opts,
                    &mut on_event,
                )
                .await
            else {
                return;
            };
            if let Err(error) = store.record_side_call_usage(&id, "memory", &r.usage) {
                report_auxiliary_persistence_failure(&id, "memory usage", &error, |warning| {
                    if let Some(sink) = warning_sink.as_mut() {
                        sink.emit(PresenterEvent::Warning(warning));
                    }
                });
            }
            let scope = memory_scope_at(workspace.root());
            // Collect lines into owned Strings before the per-line await to avoid holding
            // a borrow across the embed_one await point.
            let lines: Vec<String> = r.content.lines().take(3).map(str::to_string).collect();
            for raw in lines {
                let line = raw.trim().trim_start_matches(['-', '*', '•']).trim();
                let Some((kind, text)) = line.split_once(':') else {
                    continue;
                };
                let kind_norm = kind.trim().to_lowercase();
                let kind_cat = match kind_norm.as_str() {
                    "preference" | "decision" | "fact" | "reference" => kind_norm.as_str(),
                    _ => "fact",
                };
                let text = text.trim();
                if text.len() >= 4 {
                    match embed_one(&config.lattice.embeddings, text).await {
                        Some(emb) => {
                            if let Err(error) =
                                store.add_memory_with_embedding(&scope, kind_cat, text, &id, &emb)
                            {
                                report_auxiliary_persistence_failure(
                                    &id,
                                    "memory",
                                    &error,
                                    |warning| {
                                        if let Some(sink) = warning_sink.as_mut() {
                                            sink.emit(PresenterEvent::Warning(warning));
                                        }
                                    },
                                );
                            }
                        }
                        None => {
                            if let Err(error) = store.add_memory(&scope, kind_cat, text, &id) {
                                report_auxiliary_persistence_failure(
                                    &id,
                                    "memory",
                                    &error,
                                    |warning| {
                                        if let Some(sink) = warning_sink.as_mut() {
                                            sink.emit(PresenterEvent::Warning(warning));
                                        }
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }))
    }

    pub(crate) async fn generate_recap(
        &mut self,
        prompt: &str,
        final_text: &str,
        tasks_before: &[forge_types::TodoItem],
    ) {
        if !self.config.recap.enabled {
            return;
        }
        // A stalled turn (empty-response give-up, hard failover exhaustion) leaves `final_text`
        // empty: there is nothing the assistant actually did to summarize. Recapping anyway makes
        // the trivial-tier summarizer lean on the *request* and invent success ("Fixed the bug…")
        // for a turn that accomplished nothing — so skip it outright.
        if final_text.trim().is_empty() {
            return;
        }
        if let Some(text) = completed_tasks_recap(tasks_before, &self.tasks, final_text) {
            self.presenter.emit(PresenterEvent::Recap { text });
            return;
        }
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        if budget.status() == BudgetStatus::Exhausted {
            return;
        }
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let quota = readiness.quota;
        let decision = self
            .router
            .route_hinted(
                "summarize in one sentence",
                false,
                budget,
                &health,
                &quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;
        let Some(model) = self.post_turn_auxiliary_model(&decision) else {
            return;
        };
        let user_snippet: String = prompt.chars().take(400).collect();
        let assistant_snippet: String = final_text.chars().take(800).collect();
        let messages = vec![
            Message::system(Self::RECAP_SYSTEM),
            Message::user(format!(
                "User request:\n{user_snippet}\n\nAssistant response:\n{assistant_snippet}"
            )),
        ];
        // Routing above is local/fast; the only slow part is the provider completion. If the
        // presenter can hand out a Send sink (the channel-backed TUI), run that completion on a
        // DETACHED task and return now — so the turn ends, the spinner stops, and input frees the
        // instant the response is done; the recap streams in a moment later. Synchronous presenters
        // (headless / tests) have no sink, so it runs inline exactly as before.
        let provider = self.provider.clone();
        let store = self.store.clone();
        let id = self.id.clone();
        match self.presenter.recap_sink() {
            Some(mut sink) => {
                tokio::spawn(async move {
                    let mut on_event = |_: StreamEvent| {};
                    let completion_opts = Session::auxiliary_completion_options(&id, "recap");
                    if let Ok(r) = provider
                        .complete_with(&model, &messages, &[], &completion_opts, &mut on_event)
                        .await
                    {
                        if let Err(error) = store.record_side_call_usage(&id, "recap", &r.usage) {
                            report_auxiliary_persistence_failure(
                                &id,
                                "recap usage",
                                &error,
                                |warning| sink.emit(PresenterEvent::Warning(warning)),
                            );
                        }
                        if let Some(text) = recap_line(&r.content) {
                            sink.emit(PresenterEvent::Recap { text });
                        }
                    }
                });
            }
            None => {
                let mut on_event = |_: StreamEvent| {};
                let completion_opts = Session::auxiliary_completion_options(&id, "recap");
                if let Ok(r) = provider
                    .complete_with(&model, &messages, &[], &completion_opts, &mut on_event)
                    .await
                {
                    if let Err(error) = store.record_side_call_usage(&id, "recap", &r.usage) {
                        report_auxiliary_persistence_failure(
                            &id,
                            "recap usage",
                            &error,
                            |warning| self.presenter.emit(PresenterEvent::Warning(warning)),
                        );
                    }
                    if let Some(text) = recap_line(&r.content) {
                        self.presenter.emit(PresenterEvent::Recap { text });
                    }
                }
            }
        }
    }

    /// After a turn completes, make one cheap trivial-tier call predicting the user's likely
    /// next prompt, emitted via [`PresenterEvent::SuggestionReady`] and shown as dim ghost text
    /// in an empty, idle input box (Tab accepts it — editable, never auto-sent). Best-effort:
    /// silently skipped on budget exhaustion or any model error, exactly like `generate_recap`,
    /// whose detachment pattern (and reasoning) this mirrors.
    pub(crate) async fn generate_suggestion(&mut self, prompt: &str, final_text: &str) {
        if !self.config.suggest.enabled {
            return;
        }
        if final_text.trim().is_empty() {
            return;
        }
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        if budget.status() == BudgetStatus::Exhausted {
            return;
        }
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let quota = readiness.quota;
        let decision = self
            .router
            .route_hinted(
                "propose the next likely user prompt",
                false,
                budget,
                &health,
                &quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;
        let Some(model) = self.post_turn_auxiliary_model(&decision) else {
            return;
        };
        let user_snippet: String = prompt.chars().take(400).collect();
        // The TAIL of the response (not the head, unlike the recap's snippet) is what best
        // predicts a likely follow-up — it's what the user is looking at right now.
        let assistant_chars: Vec<char> = final_text.chars().collect();
        let tail_start = assistant_chars.len().saturating_sub(2000);
        let assistant_snippet: String = assistant_chars[tail_start..].iter().collect();
        let messages = vec![
            Message::system(Self::SUGGEST_SYSTEM),
            Message::user(format!(
                "User's last prompt:\n{user_snippet}\n\nAssistant response (tail):\n{assistant_snippet}"
            )),
        ];
        // Same detached-task reasoning as `generate_recap`: hand off to a channel-backed
        // presenter's sink when available so the turn ends and input frees immediately, with the
        // suggestion landing a moment later.
        let provider = self.provider.clone();
        let store = self.store.clone();
        let id = self.id.clone();
        let prev_prompt = prompt.to_string();
        match self.presenter.recap_sink() {
            Some(mut sink) => {
                tokio::spawn(async move {
                    let mut on_event = |_: StreamEvent| {};
                    let completion_opts = Session::auxiliary_completion_options(&id, "suggest");
                    if let Ok(r) = provider
                        .complete_with(&model, &messages, &[], &completion_opts, &mut on_event)
                        .await
                    {
                        if let Err(error) = store.record_side_call_usage(&id, "suggest", &r.usage) {
                            report_auxiliary_persistence_failure(
                                &id,
                                "suggestion usage",
                                &error,
                                |warning| sink.emit(PresenterEvent::Warning(warning)),
                            );
                        }
                        if let Some(text) = sanitize_suggestion(&r.content, &prev_prompt) {
                            sink.emit(PresenterEvent::SuggestionReady { text });
                        }
                    }
                });
            }
            None => {
                let mut on_event = |_: StreamEvent| {};
                let completion_opts = Session::auxiliary_completion_options(&id, "suggest");
                if let Ok(r) = provider
                    .complete_with(&model, &messages, &[], &completion_opts, &mut on_event)
                    .await
                {
                    if let Err(error) = store.record_side_call_usage(&id, "suggest", &r.usage) {
                        report_auxiliary_persistence_failure(
                            &id,
                            "suggestion usage",
                            &error,
                            |warning| self.presenter.emit(PresenterEvent::Warning(warning)),
                        );
                    }
                    if let Some(text) = sanitize_suggestion(&r.content, &prev_prompt) {
                        self.presenter
                            .emit(PresenterEvent::SuggestionReady { text });
                    }
                }
            }
        }
    }

    /// On a failed shell command, make one cheap trivial-tier model call explaining the likely
    /// cause + a concrete fix, surfaced via [`PresenterEvent::ShellDiagnosis`]. Best-effort: it
    /// is skipped when the budget is exhausted and stays silent on any model error, so it can
    /// never derail the turn (shell-error-interceptor.md).
    pub(crate) async fn diagnose_shell_error(&mut self, command: &str, result: &str) {
        // Fast path: common patterns don't need a model call.
        if let Some(cached) = pattern_diagnose(result) {
            self.pending_hints
                .push(format!("[shell diagnosis] {cached}"));
            self.presenter.emit(PresenterEvent::ShellDiagnosis {
                command: command.to_string(),
                diagnosis: cached.to_string(),
                fix: None,
            });
            return;
        }
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        if budget.status() == BudgetStatus::Exhausted {
            return;
        }
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let quota = readiness.quota;
        let decision = self
            .router
            .route_hinted(
                "explain a shell error",
                false,
                budget,
                &health,
                &quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;
        let Some(model) = self.post_turn_auxiliary_model(&decision) else {
            return;
        };
        let messages = [
            Message::system(SHELL_DIAGNOSE_SYSTEM),
            Message::user(format!("Command:\n{command}\n\nResult:\n{result}")),
        ];
        self.presenter.emit(PresenterEvent::AuxiliaryRequest {
            model: model.clone(),
            purpose: "diagnosing the failed shell command".to_string(),
        });
        let provider = self.provider.clone();
        let completion_opts = Self::auxiliary_completion_options(&self.id, "shell-diagnose");
        let presenter = &mut self.presenter;
        let activity = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let activity_for_sink = std::sync::Arc::clone(&activity);
        let mut sink = |event: StreamEvent| match event {
            StreamEvent::Text(delta) | StreamEvent::Reasoning(delta) if !delta.is_empty() => {
                activity_for_sink.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                presenter.emit(PresenterEvent::AuxiliaryProgress {
                    chars: delta.chars().count(),
                });
            }
            _ => {}
        };
        let configured_idle = self.config.mesh.stream_idle_timeout_secs;
        let idle = std::time::Duration::from_secs(if configured_idle == 0 {
            SHELL_DIAGNOSE_MAX_SECS
        } else {
            configured_idle.min(SHELL_DIAGNOSE_MAX_SECS)
        });
        let completion =
            provider.complete_with(&model, &messages, &[], &completion_opts, &mut sink);
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(SHELL_DIAGNOSE_MAX_SECS),
            stream_with_idle_timeout(completion, &activity, None, idle),
        )
        .await;
        if let Ok(Ok(r)) = response {
            if let Err(error) =
                self.store
                    .record_side_call_usage(&self.id, "shell/diagnose", &r.usage)
            {
                report_auxiliary_persistence_failure(
                    &self.id,
                    "shell diagnosis usage",
                    &error,
                    |warning| self.presenter.emit(PresenterEvent::Warning(warning)),
                );
            }
            // Parse structured response: cause on line 1, optional "FIX: <cmd>" on line 2.
            let mut cause = String::new();
            let mut fix: Option<String> = None;
            for line in r.content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(cmd) = trimmed.strip_prefix("FIX: ") {
                    fix = Some(cmd.trim().to_string());
                } else if cause.is_empty() {
                    cause = trimmed.to_string();
                }
            }
            if cause.is_empty() {
                cause = r.content.trim().to_string();
            }
            if !cause.is_empty() {
                let hint = if let Some(ref f) = fix {
                    format!("[shell diagnosis] {cause}  fix: {f}")
                } else {
                    format!("[shell diagnosis] {cause}")
                };
                self.pending_hints.push(hint);
                self.presenter.emit(PresenterEvent::ShellDiagnosis {
                    command: command.to_string(),
                    diagnosis: cause,
                    fix,
                });
            }
        } else {
            let detail = if response.is_err() {
                format!("timed out after {SHELL_DIAGNOSE_MAX_SECS}s")
            } else {
                "provider unavailable".to_string()
            };
            self.presenter.emit(PresenterEvent::Warning(format!(
                "optional shell diagnosis {detail} — continuing without it"
            )));
        }
    }
}
