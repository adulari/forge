//! The session side of the expert on call: resolving the expert for a trigger, metering and
//! refusing consults, making the call, and recording what came back.

use super::*;

impl Session {
    /// The expert for `trigger`, if the config, the trigger switch, privacy and the model's
    /// health all allow one. `active` is the model driving the turn: an expert that is the same
    /// model adds nothing to a stall or a question. `Err` says why a configured expert is
    /// unusable.
    pub(crate) fn peek_expert(
        &self,
        trigger: Trigger,
        active: Option<&str>,
    ) -> Result<Option<Resolved>, String> {
        let cfg = &self.config.harness.expert;
        if !cfg.is_set() {
            return Ok(None);
        }
        let allowed = match trigger {
            Trigger::Stall => cfg.triggers.stall,
            Trigger::Done => cfg.triggers.done,
            Trigger::Tool => cfg.triggers.tool,
        };
        if !allowed {
            return Ok(None);
        }
        let Some((model, spec)) = resolve_config(cfg)? else {
            return Ok(None);
        };
        if trigger != Trigger::Done && active.is_some_and(|a| a == model) {
            return Ok(None);
        }
        self.model_problem(&model)?;
        Ok(Some(Resolved {
            model,
            spec,
            counted: true,
        }))
    }

    /// Why `model` cannot take a side call right now, as a sentence: privacy mode, a benched
    /// model, or a keyless local endpoint nothing has vouched for.
    fn model_problem(&self, model: &str) -> Result<(), String> {
        if let Some(reason) = forge_mesh::privacy::retention_block(&self.config, model) {
            return Err(format!("privacy mode blocks {model} ({reason})"));
        }
        let health = self.provider_readiness().health;
        if health.is_benched(model) {
            return Err(format!(
                "{model} is benched after recent failures; see `forge models`"
            ));
        }
        if !crate::auxiliary_candidates::usable_for_side_call(model, &health, self.catalog.as_ref())
        {
            return Err(format!(
                "{model} is a local or keyless endpoint that has not answered model discovery; \
                 start the server and check `forge models`"
            ));
        }
        Ok(())
    }

    /// Tell the owner, once per session, that a configured expert cannot be used, at the first
    /// turn rather than at the first stall hours in.
    pub(crate) fn note_expert_problem(&mut self) {
        let cfg = &self.config.harness.expert;
        if !cfg.is_set() || self.expert.config_error_told {
            return;
        }
        let problem = match resolve_config(cfg) {
            Err(why) => Some(why),
            Ok(Some((model, _))) => self.model_problem(&model).err(),
            Ok(None) => None,
        };
        if let Some(why) = problem {
            self.expert.config_error_told = true;
            tracing::warn!(%why, "ignoring [harness.expert]");
            self.presenter.emit(PresenterEvent::Warning(format!(
                "expert unavailable: {why}"
            )));
        }
    }

    /// [`Self::peek_expert`], telling the owner once per session why a configured expert is
    /// unusable.
    pub(crate) fn resolve_expert(
        &mut self,
        trigger: Trigger,
        active: Option<&str>,
    ) -> Option<Resolved> {
        match self.peek_expert(trigger, active) {
            Ok(resolved) => resolved,
            Err(why) => {
                if !self.expert.config_error_told {
                    self.expert.config_error_told = true;
                    tracing::warn!(%why, "ignoring [harness.expert]");
                    self.presenter.emit(PresenterEvent::Warning(format!(
                        "expert unavailable: {why}"
                    )));
                }
                None
            }
        }
    }

    /// Whether `consult_expert` is advertised: an expert is configured, usable and allowed to be
    /// asked by the model.
    pub(crate) fn expert_tool_on(&self) -> bool {
        matches!(self.peek_expert(Trigger::Tool, None), Ok(Some(_)))
    }

    /// Whether another consult may start, and if not, why in a sentence.
    fn consult_refusal(&mut self, r: &Resolved) -> Option<String> {
        let max = self.config.harness.expert.max_consults_per_turn;
        if r.counted && self.expert.turn_consults >= max {
            return Some(format!("this turn already used its {max} expert consults"));
        }
        if let Some(why) = self.session_spend_refusal() {
            return Some(why);
        }
        if self.past_turn_deadline() {
            return Some("the turn is past its deadline".to_string());
        }
        let spec = r.spec.as_ref()?;
        let windows = self.store.subscription_windows().unwrap_or_default();
        let now = chrono::Utc::now().timestamp();
        let check = meter(
            &mut self.expert.envelope,
            spec,
            &windows,
            self.expert.usd_spent,
            now,
        );
        envelope_refusal(
            check,
            self.expert.envelope.as_ref(),
            &mut self.expert.unverified_consults,
            &self.config.harness.expert.budget,
        )
    }

    /// Make one consult call. `Err` is a refusal in a sentence (cap, budget, spend, deadline);
    /// the call was not made. `Ok` is whatever came back, judged by the caller, who then
    /// settles it with [`Self::settle_consult`].
    pub(crate) async fn consult_call(
        &mut self,
        r: &Resolved,
        trigger: Trigger,
        system: &str,
        input: String,
        purpose: &str,
        timeout_secs: u64,
    ) -> Result<RawCall, String> {
        if let Some(why) = self.consult_refusal(r) {
            return Err(why);
        }
        self.expert.turn_consults += 1;
        self.expert.session_consults += 1;
        self.expert.turn_triggers.push(trigger.as_str());
        self.expert.turn_model = Some(r.model.clone());
        self.presenter.emit(PresenterEvent::AuxiliaryRequest {
            model: r.model.clone(),
            purpose: purpose.to_string(),
        });
        let started = std::time::Instant::now();
        let (call, hints) = raw_call(
            &self.provider,
            &self.pricing,
            &r.model,
            system,
            input,
            format!("{}:expert", self.id),
            timeout_secs,
        )
        .await;
        persist_quota_hints(&self.store, &r.model, &hints);
        self.expert.usd_spent += call.usage.cost_usd;
        tracing::info!(
            model = r.model,
            trigger = trigger.as_str(),
            latency_ms = started.elapsed().as_millis() as u64,
            output_tokens = call.usage.output_tokens,
            "expert consult"
        );
        Ok(call)
    }

    /// Record a consult that was made: its usage row under `label`, the status line and the
    /// event every surface shows.
    pub(crate) fn settle_consult(
        &mut self,
        r: &Resolved,
        trigger: Trigger,
        label: &str,
        usage: &forge_types::Usage,
        summary: &str,
    ) {
        if let Err(error) =
            self.store
                .record_side_call_usage_for(&self.id, label, Some(&r.model), usage)
        {
            tracing::warn!(%error, "expert usage was not recorded");
        }
        if let Some(spec) = &r.spec {
            let windows = self.store.subscription_windows().unwrap_or_default();
            let now = chrono::Utc::now().timestamp();
            meter(
                &mut self.expert.envelope,
                spec,
                &windows,
                self.expert.usd_spent,
                now,
            );
        }
        if !summary.is_empty() && !summary.starts_with("no usable reply") {
            self.expert.turn_diagnosis = Some(clip_chars(summary, 400));
        }
        let status = status_line(
            &r.model,
            self.expert.session_consults,
            self.expert.envelope.as_ref().filter(|_| r.spec.is_some()),
        );
        if !r.counted {
            return;
        }
        self.expert.status = Some(status.clone());
        self.presenter.emit(PresenterEvent::ExpertConsult {
            trigger: trigger.as_str().to_string(),
            model: r.model.clone(),
            summary: clip_chars(summary, 400),
            status,
        });
    }

    /// The stall trigger: ask the expert why the turn is going nowhere. `stopping` is whether
    /// the supervisor is about to end the turn. `Some` only for a parsed reply; every failure is
    /// recorded and the supervisor goes on as it would have without an expert.
    pub(crate) async fn expert_on_stall(
        &mut self,
        report: &crate::progress_report::StallReport,
        unresolved_checks: Option<&str>,
        stopping: bool,
        active: &str,
    ) -> Option<(Resolved, ExpertReply)> {
        let r = self.resolve_expert(Trigger::Stall, Some(active))?;
        let open_tasks = crate::nudge_policy::open_titles(&self.tasks).len();
        let mut signal = report.describe(unresolved_checks, open_tasks);
        if stopping {
            signal.push_str(" Forge is about to stop the turn.");
        }
        let input = consult_input(&self.transcript, &signal, None);
        let timeout = self.config.harness.expert.timeout_secs;
        let call = match self
            .consult_call(
                &r,
                Trigger::Stall,
                EXPERT_SYSTEM,
                input,
                "expert consult (stall)",
                timeout,
            )
            .await
        {
            Ok(call) => call,
            Err(why) => {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "expert not consulted: {why}"
                )));
                return None;
            }
        };
        let (label, parsed) = judge(&call, Trigger::Stall);
        let summary = parsed.as_ref().map_or_else(
            || failure_line(&label, &call.detail),
            ExpertReply::owner_line,
        );
        self.settle_consult(&r, Trigger::Stall, &label, &call.usage, &summary);
        parsed.map(|p| (r, p))
    }

    /// The `consult_expert` tool: answer the model's question from the expert, or say plainly
    /// that the expert is unavailable so the model goes on alone.
    pub(crate) async fn consult_expert_tool(
        &mut self,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let question = call
            .args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if question.is_empty() {
            return Ok("error: consult_expert needs a `question`".to_string());
        }
        let active = self.harness_model.clone();
        let Some(r) = self.resolve_expert(
            Trigger::Tool,
            Some(active.as_str()).filter(|a| !a.is_empty()),
        ) else {
            return Ok(
                "No expert is available for this session. Go on with your own judgement."
                    .to_string(),
            );
        };
        let input = consult_input(
            &self.transcript,
            "The agent asked for help.",
            Some(&question),
        );
        let timeout = self.config.harness.expert.timeout_secs;
        let raw = match self
            .consult_call(
                &r,
                Trigger::Tool,
                EXPERT_SYSTEM,
                input,
                "expert consult (question)",
                timeout,
            )
            .await
        {
            Ok(raw) => raw,
            Err(why) => {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "expert not consulted: {why}"
                )));
                return Ok(format!(
                    "The expert was not consulted: {why}. Go on with your own judgement."
                ));
            }
        };
        let (label, parsed) = judge(&raw, Trigger::Tool);
        let summary = parsed.as_ref().map_or_else(
            || failure_line(&label, &raw.detail),
            ExpertReply::owner_line,
        );
        self.settle_consult(&r, Trigger::Tool, &label, &raw.usage, &summary);
        Ok(tool_answer(&r.model, &label, parsed.as_ref()))
    }

    /// Consults a bridged model made through `forge mcp-serve` this turn: that process records a
    /// usage row per consult and nothing else reaches this one. Fold them into the turn's count
    /// and show the status line.
    pub(crate) fn fold_bridge_consults(&mut self) {
        let Ok(stored) =
            self.store
                .count_side_calls_since(&self.id, "expert/tool:", self.expert.prompt_seq)
        else {
            return;
        };
        let direct = self
            .expert
            .turn_triggers
            .iter()
            .filter(|t| **t == "tool")
            .count() as u32;
        let bridged = stored.saturating_sub(direct);
        if bridged == 0 {
            return;
        }
        let Ok(Some((model, spec))) = resolve_config(&self.config.harness.expert) else {
            return;
        };
        self.expert.turn_consults += bridged;
        self.expert.session_consults += bridged;
        self.expert.turn_model = Some(model.clone());
        self.expert
            .turn_triggers
            .extend(std::iter::repeat_n("tool", bridged as usize));
        if let Some(spec) = &spec {
            let windows = self.store.subscription_windows().unwrap_or_default();
            let now = chrono::Utc::now().timestamp();
            meter(
                &mut self.expert.envelope,
                spec,
                &windows,
                self.expert.usd_spent,
                now,
            );
        }
        let status = status_line(
            &model,
            self.expert.session_consults,
            self.expert.envelope.as_ref().filter(|_| spec.is_some()),
        );
        self.expert.status = Some(status.clone());
        self.presenter.emit(PresenterEvent::ExpertConsult {
            trigger: "tool".to_string(),
            model,
            summary: format!(
                "the bridged model asked the expert {bridged} time{}; its answers are in that \
                 model's tool results",
                if bridged == 1 { "" } else { "s" }
            ),
            status,
        });
    }

    /// The turn's expert facts for the stored turn detail.
    pub(crate) fn expert_turn_detail(&self) -> Option<serde_json::Value> {
        (self.expert.turn_consults > 0).then(|| {
            serde_json::json!({
                "expert_consults": self.expert.turn_consults,
                "expert_model": self.expert.turn_model,
                "expert_triggers": self.expert.turn_triggers,
                "expert_status": self.expert.status,
                "expert_diagnosis": self.expert.turn_diagnosis,
            })
        })
    }
}
