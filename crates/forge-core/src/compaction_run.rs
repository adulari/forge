//! `Session::compact`: fold the older transcript into a model-written summary, then append the
//! user's own words verbatim.

use super::*;
use crate::compaction_budget::{size_directive, CompactionBudget, COMPACT_OUTPUT_TOKENS};
use crate::compaction_chain::chain_for;
use crate::compaction_headroom::callable_summarizers;
use crate::compaction_input::{fold_inputs, render_entry, trim_notice, FoldedInput};
use crate::compaction_policy::round_aligned_split;
use crate::compaction_shape::{kept_tail_start, summary_too_thin};
use crate::compaction_summary::{
    extract_summary, focus_directive, head_tail, render_verbatim_block_within, rolling_payload,
    system_prompt, SUMMARY_PREFIX,
};
use crate::spend_guard::priced;

/// Longest one summary call may take before the chain moves to the next model. Generous because
/// the call writes up to [`COMPACT_OUTPUT_TOKENS`]; a hung stream is caught earlier by the idle
/// watchdog.
pub(crate) const COMPACT_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Room a part's payload keeps for the wrapper tags, and the user's `/compact` instructions, on
/// top of the standing system prompt.
const COMPACT_FRAMING_TOKENS: usize = 600;

/// What a part's wrapper tags and instructions cost, taken off the part's room.
const PART_WRAPPER_TOKENS: usize = 100;

/// How a summarizer call failed.
enum SummarizerFailure {
    Provider(forge_provider::ProviderError),
    TimedOut,
}

/// End (exclusive) of the part that starts at `start`: as many whole entries as fit `budget`
/// tokens, and always at least one so an entry larger than the budget still makes progress (the
/// last-resort guard, [`fit_compaction_payload`], then cuts it down).
pub(crate) fn next_chunk_end(costs: &[usize], start: usize, budget: usize) -> usize {
    let mut end = start;
    let mut used = 0usize;
    while end < costs.len() && (end == start || used + costs[end] <= budget) {
        used += costs[end];
        end += 1;
    }
    end
}

/// One summarizer call, bounded by the idle watchdog and by `timeout`. A free function over the
/// provider handle: a future holding `&Session` across an await is not `Send`, and compaction runs
/// on a spawned task.
async fn call_summarizer(
    provider: std::sync::Arc<dyn Provider>,
    model: &str,
    system: String,
    payload: String,
    options: &forge_provider::CompletionOptions,
    idle: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<forge_provider::ModelResponse, SummarizerFailure> {
    let messages = [Message::system(system), Message::user(payload)];
    let activity = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let activity_for_sink = std::sync::Arc::clone(&activity);
    let mut sink = move |event: StreamEvent| match event {
        StreamEvent::Text(delta) | StreamEvent::Reasoning(delta) if !delta.is_empty() => {
            activity_for_sink.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        _ => {}
    };
    let completion = provider.complete_with(model, &messages, &[], options, &mut sink);
    match tokio::time::timeout(
        timeout,
        stream_with_idle_timeout(completion, &activity, None, idle),
    )
    .await
    {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(SummarizerFailure::Provider(error)),
        Err(_) => Err(SummarizerFailure::TimedOut),
    }
}

impl Session {
    /// Room reserved for the summary a call writes back: the output ceiling, but never more than
    /// a quarter of a small window.
    pub(crate) fn compact_summary_reserve(&self, model: &str) -> usize {
        let window = self.effective_context_window(model) as usize;
        COMPACT_OUTPUT_TOKENS.min(window / 4).max(1_024)
    }

    /// Token budget for the input of ONE summary call against `model`: its window, minus the
    /// standing prompt, minus room for the summary it has to write back, with the same 5% headroom
    /// [`Self::transcript_for`] keeps for the divergence between our o200k count and the target
    /// model's own tokenizer. Floored so a pathologically small window still gets a request worth
    /// making rather than an empty one.
    pub(crate) fn compact_input_budget(&self, model: &str) -> usize {
        let window = self.effective_context_window(model) as usize;
        let reserve = self.compact_summary_reserve(model)
            + tokens::count_message(&system_prompt(None, true))
            + COMPACT_FRAMING_TOKENS;
        (window.saturating_sub(reserve) * 95 / 100).max(512)
    }

    /// Carry `/compact <focus>` into the next compaction.
    pub fn set_compact_focus(&mut self, focus: &str) {
        let focus = focus.trim();
        if !focus.is_empty() {
            self.compact_directive = Some(focus_directive(focus));
        }
    }

    pub async fn compact(&mut self, auto: bool) -> Result<(usize, usize), CoreError> {
        let result = self.compact_for(auto, None).await;
        // The auto path refreshes the gauge itself; a manual compaction must too.
        if let (false, Ok((before, after))) = (auto, &result) {
            if before != after {
                self.refresh_gauge_after_history_change();
            }
        }
        result
    }

    /// The number auto-compaction compares against for `model` (the session's own when `None`):
    /// the configured trigger, or the room the window leaves for a request when that is smaller.
    /// Every part a compaction puts back is budgeted from this, so the result lands under it.
    pub(crate) fn effective_compaction_trigger(&self, model: Option<&str>) -> u64 {
        let model = model
            .map(str::to_string)
            .or_else(|| self.session_model_hint())
            .or_else(|| self.gauge_model.clone());
        match model {
            Some(model) => self
                .compaction_trigger(&model)
                .min(self.usable_window_tokens(&model)),
            None => crate::auto_compact_trigger_tokens(
                u64::from(forge_mesh::pricing::CONSERVATIVE_CONTEXT_WINDOW),
                self.config.mesh.compact_cap_tokens,
                crate::AUTO_COMPACT_THRESHOLD,
            ),
        }
    }

    /// [`Self::compact`] for a session about to run on `model`, whose trigger sizes the budgets.
    pub(crate) async fn compact_for(
        &mut self,
        auto: bool,
        model: Option<&str>,
    ) -> Result<(usize, usize), CoreError> {
        // Taken first so a no-op compaction cannot leave it armed for an unrelated later one.
        let directive = self.compact_directive.take();
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
        let trigger = self.effective_compaction_trigger(model);
        let budget = CompactionBudget::for_trigger(trigger, self.pinned_tokens());
        tracing::debug!(?budget, auto, "compaction budget");
        let tail_budget = usize::try_from(self.estimated_transcript_tokens() / 4)
            .unwrap_or(usize::MAX)
            .min(budget.keep_rounds_tokens);
        let split = round_aligned_split(
            &self.transcript,
            kept_tail_start(&self.transcript, COMPACT_KEEP_RECENT, tail_budget),
        );
        if split < COMPACT_MIN_OLDER {
            self.abandon_compaction(before);
            return Ok((before, before));
        }

        // The stored rows hold each tool result as the model first saw it; the in-memory copy has
        // had its older results cut to a head by the prune pass.
        let source = self.store.compaction_source(&self.id).unwrap_or_default();
        let folded = fold_inputs(&self.transcript[..split], &source.tool_results);
        let (model, summary) = match self.fold_summary(&folded, directive, &budget).await {
            Ok(written) => written,
            Err(error) => {
                self.abandon_compaction(before);
                return Err(error);
            }
        };

        let mut compacted = Vec::with_capacity(COMPACT_KEEP_RECENT + 1);
        compacted.push(Message::system(format!("{SUMMARY_PREFIX}\n{summary}")));
        // The store must keep exactly these rows: the round-aligned tail, not a fixed count.
        let kept_tail = self.transcript.len() - split;
        compacted.extend(self.transcript.split_off(split));
        self.transcript = compacted;
        self.note_transcript_folded(split);
        // The summarised blocks are gone, so the next edit to a file reports from scratch.
        self.lsp_baselines.clear();

        // Persist: soft-delete the summarised messages and store the summary so a resumed
        // session rehydrates the compacted view instead of the full uncompacted transcript.
        let _ = self
            .store
            .compact_session_store(&self.id, &summary, kept_tail);
        self.progress.note_compaction();

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
        self.history_epoch += 1;
        self.cache.note_compaction();
        // The bridge process is respawned from the compacted transcript, so what it last reported
        // describes a context that no longer exists.
        self.bridge_context_tokens = 0;
        // Every task still open just lost the part of the conversation that explains it. The
        // staleness tracker remembers that, so if one of them then stops moving the escalation can
        // say why the model cannot work out what it meant (task_staleness.rs).
        self.stale_tasks.note_compaction();
        self.rehydrate_after_compaction(&budget).await;
        self.settle_after_compaction(&budget);
        Ok((before, after))
    }

    /// The summary text for `folded`: the model's part, held to `budget`, then the user's own
    /// words verbatim. Returns the model that wrote it and the text. `directive` is the user's
    /// focus, if any; the size directive is added when the budget scales the summary down.
    pub(crate) async fn fold_summary(
        &mut self,
        folded: &FoldedInput,
        directive: Option<String>,
        budget: &CompactionBudget,
    ) -> Result<(String, String), CoreError> {
        let directive = match (directive, budget.summary_is_scaled()) {
            (Some(focus), true) => Some(format!(
                "{focus}\n\n{}",
                size_directive(budget.summary_tokens)
            )),
            (None, true) => Some(size_directive(budget.summary_tokens)),
            (directive, false) => directive,
        };
        let (model, body) = self
            .write_summary(folded, directive.as_deref(), budget)
            .await?;
        let body = if tokens::count_message(&body) > budget.summary_hard_cap() {
            tracing::debug!(
                tokens = tokens::count_message(&body),
                cap = budget.summary_hard_cap(),
                "summary over its budget, cutting its middle"
            );
            head_tail(&body, budget.summary_tokens * 4)
        } else {
            body
        };
        let block = render_verbatim_block_within(
            &folded.previous_verbatim,
            &folded.user_messages,
            budget.verbatim_tokens,
            budget.verbatim_older_chars(),
        );
        let summary = if block.is_empty() {
            body.trim().to_string()
        } else {
            format!("{}\n\n{block}", body.trim())
        };
        Ok((model, summary))
    }

    /// Put back what the summary cannot carry: the task list as it stands now, the memories most
    /// relevant to the work that is left, and the text of the files the session was working on.
    /// The project instructions need nothing: the pinned block never left the request. Directory
    /// and path-scoped instruction files rode on tool results the summary folded away, so the next
    /// read or edit that touches them delivers them again.
    async fn rehydrate_after_compaction(&mut self, budget: &CompactionBudget) {
        self.refresh_pinned_tasks();
        self.instruction_scope = Default::default();
        let query = self.recall_query(&self.latest_user_request());
        self.recall_pinned_memories(&query).await;
        // Before the files go back, so they get what is left rather than what was there.
        self.enforce_post_compact_headroom(budget.trigger);
        if let Err(error) = self.restore_recent_files(budget) {
            tracing::warn!(%error, "could not record the files restored after compaction");
        }
    }

    /// The summarizer chain for this compaction, per `[mesh] compact_model`.
    fn compact_chain(
        &self,
        decision: &forge_mesh::RoutingDecision,
        health: &forge_types::ModelHealth,
    ) -> Vec<String> {
        let pin = self.pinned_model();
        // The session's own model: the `/model` pin, else the model its last turn ran on. The
        // routed decision for "summarize this conversation" only stands in for a session that has
        // not run a turn.
        let session_model = pin
            .and_then(|set| set.first().cloned())
            .or_else(|| decision.pinned.then(|| decision.model.clone()))
            .or_else(|| self.route_affinity.as_ref().map(|a| a.model.clone()))
            .or_else(|| self.store.session_last_model(&self.id).ok().flatten())
            .unwrap_or_else(|| decision.model.clone());
        // The old order's "guaranteed" model: it is the one a stale router pin must not displace.
        let guaranteed = pin
            .and_then(|set| set.first().cloned())
            .unwrap_or_else(|| decision.model.clone());
        let mode = self.config.mesh.compact_model;
        let anchor = match mode {
            forge_config::CompactModel::Session => &session_model,
            forge_config::CompactModel::Auto => &guaranteed,
        };
        let usable = |models: Vec<String>| {
            callable_summarizers(
                models,
                anchor,
                &self.config.mesh.disabled,
                forge_config::has_api_key,
            )
        };
        // The routed chain comes from the ROUTER's pin, which is the `--model` the session was
        // created with and is never updated by `/model`. When the session carries its own pin that
        // is the user's current, deliberate choice; the router's stale one must not sit in the
        // chain ahead of it (that is how a free-pinned session summarized on a ChatGPT plan).
        let routed = if pin.is_some() {
            Vec::new()
        } else {
            let mut routed = vec![self.auxiliary_model(decision)];
            routed.extend(decision.fallbacks.clone());
            routed
        };
        let session_fallbacks = match pin {
            Some(set) => set.iter().skip(1).cloned().collect(),
            None => {
                let mut models = vec![decision.model.clone()];
                models.extend(decision.fallbacks.clone());
                models
            }
        };
        let chain = chain_for(
            mode,
            usable(self.router.compact_candidates()),
            usable(session_fallbacks),
            usable(routed),
            anchor,
            |m| health.is_benched(m),
            forge_mesh::catalog::is_subscription,
        );
        tracing::info!(
            ?mode,
            session_model,
            pinned = pin.is_some(),
            router_pinned = decision.pinned,
            session_model_benched = health.is_benched(&session_model),
            ?chain,
            "compaction chain"
        );
        chain
    }

    /// Write the model's part of the summary: walk the candidate chain, summarizing the folded
    /// range in as many parts as its window needs. Each part's call receives the summary so far
    /// and writes the updated whole, oldest part first, so nothing is dropped from the middle.
    /// Returns the model that finished and the summary body.
    async fn write_summary(
        &mut self,
        folded: &FoldedInput,
        directive: Option<&str>,
        shape: &CompactionBudget,
    ) -> Result<(String, String), CoreError> {
        let entries: Vec<String> = folded.messages.iter().map(render_entry).collect();
        let costs: Vec<usize> = entries.iter().map(|e| tokens::count_message(e)).collect();

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
                // Standard, not Trivial. A bad summary silently destroys everything the
                // session knows and nothing downstream can recover it.
                Some(TaskTier::Standard),
                self.pinned_effort,
                &self.project,
            )
            .await;
        // The session's own effort, not the model's default: the summary is the session's work.
        let effort = self.exact_effort.or(self.pinned_effort).or(decision.effort);

        let failover = self.config.mesh.failover;
        let candidates = self.compact_chain(&decision, &health);
        let mut chain = candidates.into_iter();
        let mut model = chain.next().expect("compact chain is non-empty");
        let mut options = Self::auxiliary_completion_options(&self.id, "compact");
        options.effort = effort;

        // The summary so far (scratch section already stripped), and the next entry to cover.
        let mut rolling: Option<String> = None;
        let mut position = 0usize;
        let mut best_thin: Option<String> = None;

        loop {
            // Fit the part to THIS candidate's own window. Built inside the loop because the chain
            // hops between models whose windows differ by orders of magnitude, and because sending
            // an un-fitted transcript here was once the whole defect: every small-window model
            // returned a context-overflow error and got benched for it.
            let input_budget = self.compact_input_budget(&model);
            let previous = rolling.as_ref().map(|summary| {
                crate::compaction_summary::head_tail(summary, input_budget * 3 / 5 * 3)
            });
            let previous_tokens = previous.as_deref().map_or(0, tokens::count_message);
            let available = input_budget
                .saturating_sub(previous_tokens + PART_WRAPPER_TOKENS)
                .max(input_budget / 4)
                .max(256);
            let end = next_chunk_end(&costs, position, available);
            let is_last = end == entries.len();
            if position == 0 && !is_last {
                self.presenter.emit(PresenterEvent::Warning(
                    "compaction: the conversation is larger than the summarizer's window — \
                     summarizing it in parts"
                        .to_string(),
                ));
            }
            let part = fit_compaction_payload(&entries[position..end], available);
            let payload = rolling_payload(previous.as_deref(), &part, is_last);
            let system = system_prompt(directive, previous.is_some() || !is_last);
            options.max_output_tokens = Some(
                self.compact_summary_reserve(&model)
                    .min(shape.output_tokens().max(1_024)) as u32,
            );

            let outcome = call_summarizer(
                self.provider.clone(),
                &model,
                system,
                payload,
                &options,
                std::time::Duration::from_secs(self.config.mesh.stream_idle_timeout_secs),
                self.compact_timeout,
            )
            .await;
            match outcome {
                Ok(response) => {
                    let _ = self.store.record_side_call_usage_for(
                        &self.id,
                        "compact/summarize",
                        Some(&model),
                        &priced(&self.pricing, &model, &response.usage),
                    );
                    let text = extract_summary(&response.content);
                    // A reply that says nothing is not a summary. Folding it in would replace
                    // everything the session knew with an empty string — which is what happened
                    // on 2026-09-16: a summarizer answered 52K tokens of transcript with 13
                    // tokens of nothing, the empty summary overwrote the previous real one, and
                    // the session forgot its task. Walk the chain as for a failed call, but
                    // WITHOUT benching: the session's own model is in the chain, and benching it
                    // would fail the very turn this compaction is trying to make room for.
                    if summary_too_thin(&text, &entries[position..end]) {
                        tracing::warn!(
                            model = %model,
                            chars = text.len(),
                            "compaction: summary too thin, trying the next model"
                        );
                        if !text.is_empty()
                            && best_thin
                                .as_ref()
                                .is_none_or(|best| best.trim().len() < text.trim().len())
                        {
                            best_thin = Some(text);
                        }
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "compaction: {model} returned an empty or unstructured summary — \
                             trying the next model"
                        )));
                        match chain.next() {
                            Some(next) => model = next,
                            // Every candidate was thin. Never fold in less than the user's own
                            // words: the fullest thin summary (or the summary so far) gets a
                            // notice of what was dropped, and the verbatim section follows.
                            None => {
                                let partial = best_thin.take().or(rolling.take());
                                let notice = trim_notice(&folded.messages);
                                let body = match partial {
                                    Some(partial) if !partial.trim().is_empty() => {
                                        format!("{}\n\n{notice}", partial.trim())
                                    }
                                    _ => notice,
                                };
                                return Ok((model, body));
                            }
                        }
                        continue;
                    }
                    rolling = Some(text);
                    position = end;
                    if is_last {
                        return Ok((model, rolling.take().unwrap_or_default()));
                    }
                }
                Err(SummarizerFailure::TimedOut) => {
                    let secs = self.compact_timeout.as_secs();
                    tracing::warn!(model = %model, secs, "compaction: summarizer timed out");
                    self.presenter.emit(PresenterEvent::Warning(format!(
                        "compaction: {model} did not finish within {secs}s — trying the next model"
                    )));
                    match chain.next() {
                        Some(next) => model = next,
                        None => {
                            return Err(CoreError::Provider(
                                forge_provider::ProviderError::Unavailable(format!(
                                    "compaction summary timed out after {secs}s on every model"
                                )),
                            ))
                        }
                    }
                }
                // Advance on ANY error (not just retryable ones) while failover is on: a
                // PERMANENT error on a small model (e.g. "provider unavailable" because ollama
                // isn't running) must still walk the chain to the session's model instead of
                // aborting — `advance_fallback` already excludes/benches the dead model
                // appropriately either way.
                Err(SummarizerFailure::Provider(error)) if failover => {
                    tracing::warn!(
                        model = %model,
                        reason = error.reason(),
                        %error,
                        "compaction: summarizer call failed, advancing the chain"
                    );
                    match self.advance_fallback(&model, &error, &mut chain, "compact") {
                        Some(next) => model = next,
                        None => return Err(CoreError::Provider(error)),
                    }
                }
                Err(SummarizerFailure::Provider(error)) => return Err(CoreError::Provider(error)),
            }
        }
    }
}
