//! Session routing, context-window, and architect model policy.
//!
//! This owner keeps related session invariants together behind the Session program.

use super::*;

impl Session {
    /// One source of truth for the health and quota inputs of every mesh decision.
    pub fn provider_readiness(&self) -> readiness::ProviderReadiness {
        readiness::ProviderReadiness::snapshot(&self.config, &self.store)
    }

    /// The current budget snapshot (spend vs caps) used for routing decisions.
    pub(crate) fn budget_snapshot(&self) -> BudgetState {
        let (today, week, month) = self.store.spend_summary_usd().unwrap_or_default();
        BudgetState {
            spent_today_usd: today,
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: week,
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: month,
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            // Same coding-context floor as the main turn loop, so the architect planner's failover
            // route also skips windows too small to hold the work.
            min_context_tokens: Some(self.routing_min_context()),
        }
    }

    /// The effective pin set for this turn's model choice: the in-session `/model` pin if set,
    /// else the router's own `--model` pin set. `None` = no pin (normal mesh routing). Kept as ONE
    /// source so subagent pin inheritance and the parent's own routing agree on what "the pin" is.
    pub(crate) fn effective_pin(&self) -> Option<Vec<String>> {
        self.pinned_model
            .clone()
            .or_else(|| self.router.pin().map(|p| p.to_vec()))
    }

    /// Explain how the mesh would route `prompt` right now, using this session's live catalog,
    /// quota, benched-model health and budget — the data behind the `/mesh` inspector. `None` when
    /// auto-discovery routing isn't active (no catalog), since the candidate table would be empty.
    pub fn explain_routing(&self, prompt: &str) -> Option<forge_mesh::RoutingExplanation> {
        let catalog = self.catalog.clone()?;
        let router = forge_mesh::HeuristicRouter::new(self.config.clone()).with_catalog(catalog);
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let mut exp = router.explain(
            prompt,
            self.budget_snapshot(),
            &health,
            &readiness.quota,
            self.pinned_effort(),
            &self.project,
        );
        use forge_config::ClassifierKind;
        exp.classifier_label = match self.config.mesh.classifier {
            ClassifierKind::Heuristic => "heuristic".to_string(),
            ClassifierKind::Llm | ClassifierKind::Hybrid => {
                let m = self
                    .config
                    .mesh
                    .classifier_model
                    .as_deref()
                    .unwrap_or("trivial-tier fallback");
                format!("llm ({m}) — actual tier may differ from this heuristic preview")
            }
        };
        Some(exp)
    }

    /// Snapshot the live router and routing inputs for an asynchronous `/mesh` inspection.
    /// `None` when there is no discovered catalog to inspect.
    pub fn routing_inspector(&self) -> Option<RoutingInspector> {
        let catalog = self.catalog.clone()?;
        let readiness = self.provider_readiness();
        let health = readiness.health;
        Some(RoutingInspector {
            router: Arc::clone(&self.router),
            selection_router: HeuristicRouter::new(self.config.clone()).with_catalog(catalog),
            budget: self.budget_snapshot(),
            health,
            quota: readiness.quota,
            tier_override: self.pinned_tier,
            effort: self.pinned_effort(),
            project: self.project.clone(),
            routing_context: RoutingContext::from_messages(&self.transcript).with_session_affinity(
                self.route_affinity.clone(),
                self.estimated_reusable_prefix_tokens(),
            ),
        })
    }

    /// The last-resort model to try when the routed fallback chain is exhausted: the non-excluded
    /// model whose transient bench expires soonest (the "least dead"). Returns `None` once already
    /// used, or when the only candidate is the model that just failed (`just_failed`), or when
    /// nothing transient is benched — so the caller falls through to [`CoreError::NoHealthyModel`].
    pub(crate) fn last_resort_model(
        &self,
        just_failed: &str,
        already_used: bool,
    ) -> Option<String> {
        if already_used {
            return None;
        }
        // Soonest-recovering transiently-benched model, but NEVER one whose provider has no key —
        // otherwise a keyless built-in default (e.g. groq) that got benched becomes the last-resort
        // pick, dispatches, hits a no-auth "Resolver error", and re-benches forever (the "groq for
        // everything" churn on a box with no groq key). `has_api_key` is true for keyless providers
        // (ollama, the claude/codex bridges), so those still qualify.
        let ordered = self.store.transient_benched_ordered().unwrap_or_default();
        ordered.into_iter().find(|m| {
            m != just_failed
                && !forge_config::is_model_disabled(m, &self.config.mesh.disabled)
                && forge_config::has_api_key(forge_config::provider_of(m))
        })
    }

    /// The context window (tokens) to assume for `model`: a narrow authoritative override first,
    /// then a fetched per-model window (provider API, persisted in the store), the family heuristic,
    /// and finally a conservative floor. Always returns a usable number so a turn can be bounded
    /// even for a model we've never seen. The cap only shrinks what we SEND, not the model's actual
    /// limit.
    pub(crate) fn base_context_window(&self, model: &str) -> u32 {
        forge_mesh::pricing::authoritative_context_limit(model)
            .or_else(|| {
                self.store
                    .model_context(model)
                    .ok()
                    .flatten()
                    .filter(|w| *w > 0)
            })
            .or_else(|| forge_mesh::pricing::context_limit(model))
            .unwrap_or(forge_mesh::pricing::CONSERVATIVE_CONTEXT_WINDOW)
    }

    pub(crate) fn effective_context_window(&self, model: &str) -> u32 {
        let window = self.base_context_window(model);
        // A context-overflow self-heal (see `overflow_window_cap`) lowers the usable window for the
        // rest of the turn so `transcript_with_preamble` trims the sent view below the model's real
        // limit — needed when our o200k estimate diverges from the model's own tokenizer.
        match &self.overflow_window_cap {
            Some((capped_model, cap)) if capped_model == model => window.min(*cap),
            _ => window,
        }
    }

    /// The transcript trimmed to fit `model`'s context window, reserving room for the reply. Keeps
    /// the system preamble + the most recent turns so a long conversation never overflows the
    /// window (which otherwise fails the turn as "unavailable" on every model). Cheap; computed per
    /// active model each step so failover to a smaller-window model re-trims appropriately.
    pub(crate) fn transcript_for(&self, model: &str) -> Vec<Message> {
        let window = self.effective_context_window(model) as usize;
        let reserve = output_planning_reserve_tokens(self.config.mesh.max_output_tokens) as usize;
        // Real-token budget: window minus the reply reservation, with 5% headroom for the small
        // magnitude difference between our o200k counter and the target model's own tokenizer.
        let budget_tokens = window.saturating_sub(reserve) * 95 / 100;
        to_llm(
            &self.transcript,
            budget_tokens.max(256),
            self.config.mesh.tool_result_context_token_budget,
            self.config.mesh.tool_result_context_keep_recent,
        )
    }

    /// The base harness preamble prepended (fresh, never persisted) to every main-loop request:
    /// the Forge coding-agent system prompt + a small live environment block (cwd / OS / git
    /// branch). Recomputed each call so it's always current, and placed first so the provider's
    /// cache breakpoint anchors on this stable prefix.
    pub(crate) fn system_preamble(&self) -> Vec<Message> {
        let cwd = self.workspace.display();
        let os = std::env::consts::OS;
        // No blocking syscall here: this hot per-request helper is `&self` (sync), and making it
        // `async` to read `.git/HEAD` would hold a `&Session` across an `.await` inside the spawned
        // turn future — `Session` is not `Sync` (`Receiver`/`dyn Presenter`), so the future would
        // stop being `Send` and could no longer be `tokio::spawn`ed. Instead the branch is read off
        // the async path (eagerly at session construction, refreshed via `tokio::fs` at each turn
        // start) and cached, so we just read the field.
        let mut env = format!("<env>\nworking_directory: {cwd}\nplatform: {os}\n");
        if let Some(b) = &self.cached_git_branch {
            env.push_str(&format!("git_branch: {b}\n"));
        }
        env.push_str("</env>");
        let mut msgs = vec![Message::system(FORGE_SYSTEM), Message::system(env)];
        // Headless code-change turns (bench swe) get the minimal-diff bias — per-request system
        // context, so it reaches direct AND bridge providers without touching the bridge preamble.
        if self.expect_code_change {
            msgs.push(Message::system(MINIMAL_DIFF_BIAS));
        }
        msgs
    }

    /// The request body for a main-loop call: the base harness preamble (system prompt + env)
    /// followed by the window-fitted transcript. The preamble's token cost is subtracted from the
    /// trim budget so the prepended prompt can't push the request over the model's window.
    pub(crate) fn transcript_with_preamble(&self, model: &str) -> Vec<Message> {
        let preamble = self.system_preamble();
        let window = self.effective_context_window(model) as usize;
        let reserve = output_planning_reserve_tokens(self.config.mesh.max_output_tokens) as usize;
        let preamble_tokens: usize = preamble.iter().map(message_tokens).sum();
        let budget_tokens = window
            .saturating_sub(reserve)
            .saturating_sub(preamble_tokens)
            * 95
            / 100;
        let mut out = preamble;
        out.extend(to_llm(
            &self.transcript,
            budget_tokens.max(256),
            self.config.mesh.tool_result_context_token_budget,
            self.config.mesh.tool_result_context_keep_recent,
        ));
        out
    }

    /// System prompt for the architect planner phase. Instructs the planner to produce a concrete
    /// prose plan only — no tool calls are available in this phase.
    const ARCHITECT_PLANNER_SYSTEM: &'static str =
        "You are the PLANNER in a two-phase coding-assistant pipeline. \
Your job is to think through the request carefully and produce a concise, concrete, step-by-step \
plan of the edits and tool calls that an EDITOR agent will execute next. \
Rules:\n\
- Output ONLY the plan as structured prose or a numbered list. No preamble, no summary of what \
  you were asked, no sign-off.\n\
- Be specific: name the exact files to create/modify, the functions to add/change, \
  and the commands to run (if any).\n\
- Do NOT attempt to call any tools — none are available in this phase. \
  Describe what SHOULD be done, not do it.";

    /// Resolve the model to use for the architect PLAN phase.
    /// Priority: in-session `/model` pin > `mesh.architect_model` config > mesh-routed Complex tier.
    pub(crate) fn resolve_planner_model(&self) -> String {
        // An active /model pin overrides everything (its first member as the planner model).
        if let Some(pin) = &self.pinned_model {
            return pin
                .first()
                .cloned()
                .unwrap_or_else(|| "anthropic::claude-opus-5".to_string());
        }
        // Explicit config override.
        if let Some(m) = &self.config.mesh.architect_model {
            if !m.is_empty() {
                return m.clone();
            }
        }
        // Fall back to the first USABLE Complex-tier candidate. `model_for` returns the first
        // configured candidate regardless of key — and the built-in defaults lead with
        // `groq::…`, so on a box with no groq key the architect planner would dispatch groq and
        // auth-fail EVERY turn (it recovers via the failover chain, but wastes a hop + warns).
        // Pick the first candidate whose provider has a key instead (keyless bridges qualify).
        self.first_usable_for_tier(forge_types::TaskTier::Complex)
            .or_else(|| {
                self.config
                    .model_for(forge_types::TaskTier::Complex)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "anthropic::claude-opus-5".to_string())
    }

    /// The first configured candidate for `tier` whose provider has a key — keyless providers
    /// (ollama, the claude/codex bridges) always qualify. `None` when the config lists nothing
    /// usable. Used to keep the architect planner/editor off a keyless built-in default (groq).
    pub(crate) fn first_usable_for_tier(&self, tier: forge_types::TaskTier) -> Option<String> {
        self.config
            .candidates_for(tier)
            .into_iter()
            .find(|m| forge_config::has_api_key(forge_config::provider_of(m)))
    }

    /// Resolve the model to use for the architect EDIT phase.
    /// Priority: in-session `/model` pin > `mesh.editor_model` config > mesh-routed Standard tier.
    pub(crate) fn resolve_editor_model(&self) -> String {
        // An active /model pin overrides everything (both phases use the same pinned set's first
        // member).
        if let Some(pin) = &self.pinned_model {
            return pin
                .first()
                .cloned()
                .unwrap_or_else(|| "anthropic::claude-opus-5".to_string());
        }
        // Explicit config override.
        if let Some(m) = &self.config.mesh.editor_model {
            if !m.is_empty() {
                return m.clone();
            }
        }
        // Fall back to the first USABLE Standard-tier candidate (see resolve_planner_model): never
        // a keyless built-in default. The architect EDIT phase runs with failover DISABLED
        // (decision=None), so a keyless editor model would hard-fail the turn instead of recovering
        // — picking a keyed model here is what keeps the edit phase alive.
        self.first_usable_for_tier(forge_types::TaskTier::Standard)
            .or_else(|| {
                self.config
                    .model_for(forge_types::TaskTier::Standard)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "anthropic::claude-opus-5".to_string())
    }

    pub(crate) fn auxiliary_model(&self, routed: &forge_mesh::RoutingDecision) -> String {
        if routed.pinned {
            self.first_usable_for_tier(TaskTier::Trivial)
                .unwrap_or_else(|| routed.model.clone())
        } else {
            routed.model.clone()
        }
    }

    /// Subscription CLI bridges start a full external agent process and MCP handshake. They are
    /// appropriate for the primary coding turn, but not for best-effort recap/suggestion/error
    /// side calls: three extra bridge launches made a completed `forge run` linger until its outer
    /// timeout. Prefer a lightweight routed model when available; otherwise omit the optional call.
    pub(crate) fn post_turn_auxiliary_model(
        &self,
        routed: &forge_mesh::RoutingDecision,
    ) -> Option<String> {
        let model = self.auxiliary_model(routed);
        (!forge_provider::is_cli_bridge(&model)).then_some(model)
    }

    /// Side calls are deliberately cheap/low-reasoning. Tiny fixed-shape results also receive a
    /// narrow output ceiling so a model that ignores "one line" cannot spend thousands of hidden
    /// reasoning/output tokens. Compaction remains uncapped because its necessary answer length
    /// scales with the conversation. A stable purpose-scoped key enables provider prefix caching.
    pub(crate) fn auxiliary_completion_options(
        session_id: &str,
        purpose: &str,
    ) -> CompletionOptions {
        let max_output_tokens = match purpose {
            "recap" | "suggest" => Some(128),
            "memory" | "shell-diagnose" => Some(256),
            _ => None,
        };
        CompletionOptions {
            effort: Some(EffortLevel::Low),
            prompt_cache_key: Some(format!("{session_id}:{purpose}")),
            max_output_tokens,
            ..CompletionOptions::default()
        }
    }

    /// Run the PLAN phase of the architect pipeline.
    ///
    /// Calls the planner model with the current transcript and NO tools advertised, streams its
    /// response as a normal assistant turn (persisted + streamed to the presenter), records
    /// usage/cost, and returns the plan text. Returns `Ok(None)` when `architect_mode` is off —
    /// the early-exit guard that makes the non-architect path byte-for-byte unchanged.
    pub(crate) async fn run_plan(&mut self) -> Result<Option<String>, CoreError> {
        if !self.config.mesh.architect_mode {
            return Ok(None);
        }

        let planner = self.resolve_planner_model();
        // Cross-provider failover chain for the plan phase: the resolved planner first, then the
        // mesh's Complex-tier chain (deduped, planner removed). Without this, a single rate-limit
        // on the planner would abort the whole architect turn before the edit loop ever runs.
        let failover = self.config.mesh.failover;
        let fallbacks: Vec<String> = if failover {
            let budget = self.budget_snapshot();
            let readiness = self.provider_readiness();
            let health = readiness.health;
            let quota = readiness.quota;
            let d = self
                .router
                .route_hinted(
                    "plan a complex software task",
                    false,
                    budget,
                    &health,
                    &quota,
                    Some(TaskTier::Complex),
                    self.pinned_effort,
                    &self.project,
                )
                .await;
            std::iter::once(d.model)
                .chain(d.fallbacks)
                .filter(|m| m != &planner)
                .collect()
        } else {
            Vec::new()
        };

        let stream_idle = std::time::Duration::from_secs(self.config.mesh.stream_idle_timeout_secs);
        let completion_opts = CompletionOptions {
            effort: self.pinned_effort,
            temperature: Some(CODING_TEMPERATURE),
            // The planner runs with no tools (it can't edit files), so it needs no checkpoint context.
            checkpoint: None,
            // Planning repeatedly reuses the same standing prompt/transcript. Keep it in a
            // session-specific provider cache shard even though the planner has no tool checkpoint.
            prompt_cache_key: Some(format!("{}:architect", self.id)),
            max_output_tokens: None,
            reuse_response_chain: false,
            response_chain_prefix_tokens: 0,
            response_format: None,
        };

        let mut chain = fallbacks.into_iter();
        let mut model = planner;
        let mut resp = loop {
            self.presenter.emit(PresenterEvent::Routing {
                tier: forge_types::TaskTier::Complex.as_str().to_string(),
                model: model.clone(),
                rationale: "architect plan phase (no tools)".to_string(),
            });

            // Re-window the transcript for THIS model (a smaller fallback still fits), then prepend
            // the planner system prompt.
            let mut planner_msgs = self.transcript_for(&model);
            planner_msgs.insert(0, Message::system(Self::ARCHITECT_PLANNER_SYSTEM));
            self.presenter.emit(PresenterEvent::ProviderRequest {
                model: model.clone(),
                step: 0,
            });

            // Collect plan text while streaming it live to the presenter.
            let mut plan_text = String::new();
            let result = {
                let provider = &self.provider;
                let presenter = &mut self.presenter;
                let activity = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let act = std::sync::Arc::clone(&activity);
                let mut sink = |ev: StreamEvent| {
                    act.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if let StreamEvent::Text(ref t) = ev {
                        plan_text.push_str(t);
                    }
                    match ev {
                        StreamEvent::Text(t) => presenter.emit(PresenterEvent::AssistantDelta(t)),
                        StreamEvent::Reasoning(t) => presenter.emit(PresenterEvent::Reasoning(t)),
                        _ => {}
                    }
                };
                // Empty tool slice — the planner must not call tools.
                let fut =
                    provider.complete_with(&model, &planner_msgs, &[], &completion_opts, &mut sink);
                stream_with_idle_timeout(fut, &activity, None, stream_idle).await
            };

            match result {
                Ok(mut r) => {
                    // Use the streamed text if the provider returns empty content (some do).
                    if r.content.is_empty() && !plan_text.is_empty() {
                        r.content = plan_text;
                    }
                    break r;
                }
                Err(e) if failover && e.is_retryable() => {
                    match self.advance_fallback(&model, &e, &mut chain, "architect plan") {
                        Some(next) => model = next,
                        None => return Err(CoreError::Provider(e)),
                    }
                }
                Err(e) => return Err(CoreError::Provider(e)),
            }
        };

        if !resp.content.is_empty() {
            self.presenter.emit(PresenterEvent::AssistantDone);
        }

        // Record cost/usage for the plan phase.
        resp.usage.cost_usd = self.pricing.cost_for_usage(&model, &resp.usage);
        let seq = self.next_seq();
        let msg_id = self.store.add_message_full(
            &self.id,
            seq,
            Role::Assistant,
            &resp.content,
            Some(&model),
            &[],
            None,
        )?;
        self.store
            .record_usage(&self.id, &msg_id, &resp.usage, Some(&model))?;

        // Push the plan into the live transcript so the editor model sees it.
        self.transcript.push(Message::assistant(&resp.content));

        Ok(Some(resp.content))
    }
}
