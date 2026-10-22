//! Best-effort session side-call policy.
//!
//! This owner keeps related session invariants together behind the Session program.

use super::*;
use crate::compaction_policy::record_model_failure_in;
use crate::spend_guard::priced;

pub(crate) fn report_auxiliary_persistence_failure(
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
    /// Mark this session as serving one invocation that exits once its turn is answered
    /// (`forge run`). Its post-turn side calls are skipped: nobody sees a recap or a suggestion
    /// after the process is gone, and a memory capture would hold the exit for a model round-trip
    /// (`mesh.auto_memory_one_shot = true` keeps the capture).
    pub fn set_one_shot(&mut self, one_shot: bool) {
        self.one_shot = one_shot;
    }

    /// Optional side calls after a turn whose result has already been emitted. A channel-backed
    /// surface detaches them (model choice included, see `post_turn_calls`), so input frees at
    /// once and their results land a moment later; a synchronous surface runs them inline after
    /// the answer is shown; a one-shot run skips them.
    pub(crate) async fn run_post_turn_side_calls(
        &mut self,
        prompt: &str,
        final_text: &str,
        tasks_before: &[forge_types::TodoItem],
    ) {
        // A tripped session spend cap ends the turn; optional side calls must not keep spending.
        let spend_ok = self.session_spend_refusal().is_none();
        let detached = self.presenter.recap_sink().is_some();
        if spend_ok && !self.one_shot {
            if detached {
                self.spawn_post_turn_calls(prompt, final_text, tasks_before);
            } else {
                self.generate_recap(prompt, final_text, tasks_before).await;
                self.generate_suggestion(prompt, final_text).await;
            }
        }
        // Continual Harness auto-refine gate (`harness.auto_refine = "turns"`, refinement.rs).
        // Inline, not detached: it mutates durable harness state and its own presenter note should
        // land in order with the turn it concluded, not race a later turn's output.
        self.auto_refine_after_turns().await;
        // The detached batch above already carries the memory capture.
        let captured_by_detached_batch = detached && !self.one_shot;
        if !spend_ok
            || captured_by_detached_batch
            || (self.one_shot && !self.config.mesh.auto_memory_one_shot)
        {
            return;
        }
        // Dropping the JoinHandle detaches (does not cancel) the capture; a synchronous surface
        // awaits it so persistence has finished when the turn returns.
        if let Some(handle) = self.capture_memories(prompt, final_text).await {
            if !detached {
                let _ = handle.await;
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
        if let Ok(Err(error)) = &response {
            record_model_failure_in(
                &self.store,
                &model,
                error,
                std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs),
            );
        }
        if let Ok(Ok(r)) = response {
            if let Err(error) = self.store.record_side_call_usage_for(
                &self.id,
                "shell/diagnose",
                Some(&model),
                &priced(&self.pricing, &model, &r.usage),
            ) {
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
