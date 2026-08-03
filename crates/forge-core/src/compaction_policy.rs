//! Session compaction and context-budget policy.
//!
//! This owner keeps related session invariants together behind the Session program.

use super::*;

impl Session {
    /// Real BPE token count of the current transcript (content + tool calls + per-message framing),
    /// via [`tokens`]. Used to decide compaction + drive the gauge; not billed. UI-only messages
    /// are excluded — they never reach a provider, so they must not inflate the gauge or trip
    /// auto-compaction.
    pub(crate) fn estimated_transcript_tokens(&self) -> u64 {
        self.transcript
            .iter()
            .filter(|m| m.visibility.is_llm())
            .map(|m| message_tokens(m) as u64)
            .sum()
    }

    /// Estimated request prefix another exact provider/model id would ingest cold.
    ///
    /// Provider telemetry exposes cached input only after a call and does not prove cache sharing,
    /// so routing uses a conservative deterministic estimate: live transcript + system preamble +
    /// stable ordered tool schemas. This is an affinity input, not billing accounting.
    pub(crate) fn estimated_reusable_prefix_tokens(&self) -> u64 {
        let preamble = self
            .system_preamble()
            .iter()
            .map(|message| message_tokens(message) as u64)
            .sum::<u64>();
        let tools = self
            .tool_specs()
            .iter()
            .map(|spec| {
                tokens::count_text(&spec.name)
                    + tokens::count_text(&spec.description)
                    + tokens::count_text(&spec.schema.to_string())
                    + 8
            })
            .sum::<usize>() as u64;
        self.estimated_transcript_tokens()
            .saturating_add(preamble)
            .saturating_add(tools)
    }

    /// Context-window floor to hand the router for the next turn, so mesh auto-rotation never picks
    /// a window this turn will immediately overflow. See [`routing_min_context_tokens`].
    pub(crate) fn routing_min_context(&self) -> u32 {
        let reserve =
            output_planning_reserve_tokens(self.config.mesh.effective_max_output_tokens());
        let transcript = self.estimated_transcript_tokens().min(u32::MAX as u64) as u32;
        routing_min_context_tokens(transcript, reserve)
    }

    /// Whether the transcript comfortably fits `model`'s window — under 80% of the post-reply room.
    /// Below this, the turn proceeds as-is; at/over it, auto-compaction kicks in (and a failover to
    /// a model that fails this check triggers the consent prompt).
    pub(crate) fn transcript_fits(&self, model: &str) -> bool {
        let window = self.effective_context_window(model) as u64;
        let reserve = output_planning_reserve_tokens(self.config.mesh.max_output_tokens) as u64;
        let usable = window.saturating_sub(reserve) * 8 / 10;
        self.estimated_transcript_tokens() <= usable
    }

    /// Decide whether to admit a mesh-chosen failover `model`. If the transcript already fits, use
    /// it. Otherwise it's a switch to a smaller-window model that needs (lossy) compaction: proceed
    /// silently when the user picked "always" this session, else ask Yes/No/Always. `Ok(false)` =
    /// the user declined (skip this model; the caller advances to the next fallback that fits).
    pub(crate) async fn admit_failover_model(&mut self, model: &str) -> Result<bool, CoreError> {
        if self.transcript_fits(model) {
            return Ok(true);
        }
        if !self.always_compact_on_switch {
            let window_k = (self.effective_context_window(model) / 1000).max(1);
            let q = format!(
                "Mesh switched to {model} (~{window_k}k context) — too small for this conversation. \
                 Compact (summarize older messages) and continue on it?"
            );
            let opts = [
                forge_types::QChoice {
                    label: "Yes".into(),
                    description: "Compact now and continue on this model".into(),
                },
                forge_types::QChoice {
                    label: "No".into(),
                    description: "Skip it — try the next model that fits".into(),
                },
                forge_types::QChoice {
                    label: "Always".into(),
                    description: "Compact on every such switch for the rest of this session".into(),
                },
            ];
            let ans = self.presenter.ask(&q, &opts, false).trim().to_lowercase();
            if ans == "always" {
                self.always_compact_on_switch = true;
            } else if ans != "yes" {
                return Ok(false); // No / cancelled → skip this model
            }
        }
        self.compact(true).await?;
        Ok(true)
    }

    /// Auto-compact (silently) when the transcript has grown past 80% of `model`'s window — the
    /// normal "conversation got long" case for the routed model, no prompt (the `compact` call
    /// emits its own one-line note). No-op when it already fits or the transcript is too short to
    /// compact. Distinct from the failover consent path ([`admit_failover_model`]).
    pub(crate) async fn auto_compact_if_needed(&mut self, model: &str) {
        let window = self.base_context_window(model) as u64;
        let trigger = auto_compact_trigger_tokens(
            window,
            self.config.mesh.compact_cap_tokens,
            AUTO_COMPACT_THRESHOLD,
        );
        if self.estimated_transcript_tokens() > trigger || !self.transcript_fits(model) {
            // Cheap first: the pipeline's mutating phase — prune bulky OLD tool results in place
            // (no model call). Often reclaims enough that the LLM summarize below isn't needed.
            if prune_and_inject(&mut self.transcript, COMPACT_KEEP_RECENT) > 0 {
                self.emit_context_gauge(model);
            }
            if !self.transcript_fits(model) {
                let _ = self.compact(true).await;
            }
            // Refresh the gauge NOW so it reflects the reduced context immediately, instead of
            // showing the old (over-window) size until the turn's first model call returns.
            self.emit_context_gauge(model);
        }
    }

    /// Emit a [`Cost`](PresenterEvent::Cost) event reflecting the CURRENT transcript size as the
    /// live context fill, so the statusline gauge + compaction band update right away (e.g. right
    /// after auto-compaction) rather than waiting for the next model call's real input-token count
    /// at turn end. Uses the conservative transcript estimate as a stand-in until the real count
    /// arrives.
    pub(crate) fn emit_context_gauge(&mut self, model: &str) {
        let (session_in, session_out) = self.store.session_tokens(&self.id).unwrap_or((0, 0));
        let session_total_usd = self.store.session_cost(&self.id).unwrap_or(0.0);
        self.presenter.emit(PresenterEvent::Cost {
            session_total_usd,
            session_in,
            session_cached_in: self
                .store
                .session_cached_input_tokens(&self.id)
                .unwrap_or(0),
            session_out,
            context_tokens: self.estimated_transcript_tokens(),
            // The gauge denominator is the model's REAL window, not the transient overflow cap.
            context_limit: Some(self.base_context_window(model)),
        });
    }

    /// Emit the terminal accounting snapshot followed by the result event. Terminal accounting
    /// uses the complete provider-consumption ledger; the context gauge above intentionally keeps
    /// using only active transcript messages.
    pub(crate) fn emit_terminal_events(
        &mut self,
        final_text: &str,
        stop_reason: StopReason,
        context_tokens: u64,
        active_model: &str,
    ) -> Result<(), CoreError> {
        let usage = self.store.session_token_usage(&self.id)?;
        self.presenter.emit(PresenterEvent::Cost {
            session_total_usd: self.store.session_cost(&self.id)?,
            session_in: usage.input_tokens,
            session_cached_in: usage.cached_input_tokens,
            session_out: usage.output_tokens,
            context_tokens,
            context_limit: Some(self.effective_context_window(active_model)),
        });
        self.presenter.emit(PresenterEvent::Done {
            final_text: final_text.to_string(),
            stop_reason,
        });
        Ok(())
    }

    /// Bench (or, for a permanent incapability, exclude) `model` after a retryable error and
    /// return the next model to try from `chain`, or `None` when the chain is exhausted. Emits the
    /// standard failover warning. Shared by the single-shot auxiliary calls (compaction, the
    /// architect plan phase) so a transient rate-limit on one provider no longer kills the whole
    /// turn — they now fail over down a chain exactly like the main model loop.
    pub(crate) fn advance_fallback(
        &mut self,
        model: &str,
        err: &forge_provider::ProviderError,
        chain: &mut dyn Iterator<Item = String>,
        label: &str,
    ) -> Option<String> {
        let reason = err.reason();
        let default_cooldown =
            std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs);
        self.record_model_failure(model, err, default_cooldown);
        let next = chain.next();
        match &next {
            // A hop drives the animated "finding a model" indicator (no per-hop scrollback spam).
            Some(_) => self.presenter.emit(PresenterEvent::ModelSearch {
                model: model.to_string(),
                retrying: false,
            }),
            // The chain is exhausted — a real, terminal failure worth a visible warning.
            None => self.presenter.emit(PresenterEvent::Warning(format!(
                "{model} {reason} — {label} chain exhausted"
            ))),
        }
        next
    }

    /// Persist health at the correct scope: a capability failure is model-specific, while an
    /// authentication failure applies to every alias of its provider and must stop sibling churn.
    pub(crate) fn record_model_failure(
        &self,
        model: &str,
        err: &forge_provider::ProviderError,
        default_cooldown: std::time::Duration,
    ) {
        // An over-window request is a statement about the payload, not about the model: the same
        // model answers fine the moment we send less. Benching for it sidelines a healthy model for
        // a full cooldown and, because the auxiliary chains are trivial-tier, walks the identical
        // oversized payload into the next cheap model and benches that one too — so the damage
        // outlives the request that caused it and degrades routing for ordinary turns afterwards.
        if err.is_context_overflow() {
            return;
        }
        let reason = err.reason();
        if err.is_auth() {
            let _ = self
                .store
                .exclude_provider(forge_config::provider_of(model), reason);
        } else if err.is_permanent() {
            let _ = self.store.exclude_model(model, reason);
        } else {
            let _ = self
                .store
                .bench_for(model, err.cooldown(default_cooldown), reason);
        }
    }

    /// Token budget for ONE compaction request against `model`: its window, minus the standing
    /// [`COMPACT_SYSTEM`] prompt, minus room for the summary it has to write back, with the same 5%
    /// headroom [`Self::transcript_for`] keeps for the divergence between our o200k count and the
    /// target model's own tokenizer. Floored so a pathologically small window still gets a request
    /// worth making rather than an empty one.
    pub(crate) fn compact_input_budget(&self, model: &str) -> usize {
        let window = self.effective_context_window(model) as usize;
        let reserve = COMPACT_SUMMARY_RESERVE_TOKENS + tokens::count_message(COMPACT_SYSTEM);
        (window.saturating_sub(reserve) * 95 / 100).max(512)
    }

    pub async fn compact(&mut self, auto: bool) -> Result<(usize, usize), CoreError> {
        let before = self.transcript.len();
        if before <= COMPACT_KEEP_RECENT + COMPACT_MIN_OLDER {
            return Ok((before, before)); // not worth a model call yet
        }
        // Drive the TUI's animated progress band (cleared by CompactionFinished below).
        self.presenter
            .emit(PresenterEvent::CompactionStarted { auto });
        // PreCompact lifecycle hook (Claude-Code parity): fires before the summary call.
        self.fire_lifecycle(
            forge_config::HookEvent::PreCompact,
            serde_json::json!({ "trigger": if auto { "auto" } else { "manual" } }),
        )
        .await;
        let split = before - COMPACT_KEEP_RECENT;
        let older = &self.transcript[..split];
        // Kept as one entry per message instead of a single pre-joined string: the candidate chain
        // deliberately crosses models with wildly different windows, so the payload has to be
        // re-fitted on every failover hop (see the loop below).
        let entries = older
            .iter()
            // UI-only notes never reach a provider — don't pay to summarize them either.
            .filter(|m| m.visibility.is_llm())
            .map(|m| {
                // Include the assistant's tool calls — they're the only record of WHAT the turn did
                // (tool name + args = the files touched / commands run). Without them an editing turn
                // renders as a blank `assistant: ` line and the summary can't say what changed.
                let mut line = format!("{}: {}", m.role.as_str(), m.content);
                for tc in &m.tool_calls {
                    line.push_str(&format!("\n  [call {} {}]", tc.name, tc.args));
                }
                line
            })
            .collect::<Vec<_>>();

        // Route the summary at the trivial tier (it's cheap, fixed work) and call the model once.
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
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let quota = readiness.quota;
        let decision = self
            .router
            .route_hinted(
                "summarize this conversation",
                false,
                budget,
                &health,
                &quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;

        // Compaction must NEVER hard-fail because a cheap trivial model is unreachable (e.g. a
        // local ollama model when ollama isn't running): losing the summary drops the task plan
        // with it. Mirror the LLM classifier's approach (#648): try the top trivial candidates,
        // then fall back to the session's OWN model, which is guaranteed reachable.
        let failover = self.config.mesh.failover;
        let guaranteed = self
            .pinned_model()
            .and_then(|set| set.first().cloned())
            .unwrap_or_else(|| decision.model.clone());
        // The routed model + its failover chain, preserved so a rate-limited summarizer still walks
        // to the routed fallback (not just to the guaranteed model).
        let mut routed = vec![self.auxiliary_model(&decision)];
        routed.extend(decision.fallbacks.clone());
        let candidates =
            compact_candidate_chain(self.router.trivial_candidates(), routed, &guaranteed, |m| {
                health.is_benched(m)
            });
        let mut chain = candidates.into_iter();
        let mut model = chain.next().expect("compact_candidate_chain is non-empty");
        let completion_opts = Self::auxiliary_completion_options(&self.id, "compact");
        let resp = loop {
            let mut sink = |_: StreamEvent| {};
            // Fit the payload to THIS candidate's own window. Built inside the loop because the
            // chain hops between models whose windows differ by orders of magnitude, and because
            // sending an un-fitted transcript here was the whole defect: every trivial-tier model
            // returned a context-overflow error and got benched for it, so a long session's
            // auto-compaction reliably failed AND degraded routing for the turns after it.
            let messages = [
                Message::system(COMPACT_SYSTEM),
                Message::user(fit_compaction_payload(
                    &entries,
                    self.compact_input_budget(&model),
                )),
            ];
            match self
                .provider
                .complete_with(&model, &messages, &[], &completion_opts, &mut sink)
                .await
            {
                Ok(r) => break r,
                // Advance on ANY error (not just retryable ones) while failover is on: a
                // PERMANENT error on a cheap trivial model (e.g. "provider unavailable" because
                // ollama isn't running) must still walk the chain to the guaranteed model instead
                // of aborting — `advance_fallback` already excludes/benches the dead model
                // appropriately either way.
                Err(e) if failover => {
                    match self.advance_fallback(&model, &e, &mut chain, "compact") {
                        Some(next) => model = next,
                        None => return Err(CoreError::Provider(e)),
                    }
                }
                Err(e) => return Err(CoreError::Provider(e)),
            }
        };
        let _ = self
            .store
            .record_side_call_usage(&self.id, "compact/summarize", &resp.usage);
        let summary = resp.content;

        let mut compacted = Vec::with_capacity(COMPACT_KEEP_RECENT + 1);
        compacted.push(Message::system(format!(
            "[Earlier conversation summarized to save context]\n{}",
            summary.trim()
        )));
        compacted.extend(self.transcript.split_off(split));
        self.transcript = compacted;

        // Persist: soft-delete the summarised messages and store the summary so a resumed
        // session rehydrates the compacted view instead of the full uncompacted transcript.
        let _ = self
            .store
            .compact_session_store(&self.id, summary.trim(), COMPACT_KEEP_RECENT);

        let after = self.transcript.len();
        self.presenter
            .emit(PresenterEvent::CompactionFinished { before, after });
        self.presenter.emit(PresenterEvent::Warning(format!(
            "compacted {before} messages → {after} (summary via {model})"
        )));
        // PostCompact lifecycle hook: fires after the summary is folded in (Forge extension beyond
        // CC, which only has PreCompact).
        self.fire_lifecycle(
            forge_config::HookEvent::PostCompact,
            serde_json::json!({ "before": before, "after": after }),
        )
        .await;
        Ok((before, after))
    }

    /// Undo a `/compact`: reactivate every soft-deleted message in the store and reload the full
    /// transcript into memory. A no-op (`before == after`) if the session was never compacted —
    /// mirrors [`compact`](Self::compact)'s "nothing to do" signal shape.
    pub fn uncompact(&mut self) -> Result<(usize, usize), CoreError> {
        let before = self.transcript.len();
        if !self.was_compacted() {
            return Ok((before, before));
        }
        self.store.uncompact_session_store(&self.id)?;
        self.reload_full_context()?;
        let after = self.transcript.len();
        self.presenter.emit(PresenterEvent::Warning(format!(
            "restored full history: {before} messages → {after} (compaction undone)"
        )));
        Ok((before, after))
    }
}
