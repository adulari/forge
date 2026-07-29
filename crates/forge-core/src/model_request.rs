//! Provider request, stream handling, and failover policy for a model-loop step.

use super::*;
use crate::model_stream::handle_stream_event;

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_provider_response(
    session: &mut Session,
    active_model: &mut String,
    specs: &[ToolSpec],
    decision: Option<&forge_mesh::RoutingDecision>,
    reuse_response_chain: bool,
    stream_idle: std::time::Duration,
    checkpoint_ctx: &forge_provider::CheckpointContext,
    verify_attempts: usize,
    in_plan_mode: bool,
    mut proposed_plan: &mut Option<forge_types::PlanProposal>,
    tools_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    inspect_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    bridge_build_fight: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    verification_ledger: &std::sync::Arc<std::sync::Mutex<VerificationLedger>>,
    bridge_observations: &std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, std::collections::VecDeque<VerificationObservation>>,
        >,
    >,
    mcp_tools_unavailable: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    explicit_pin: bool,
    failover_enabled: bool,
    default_cooldown: std::time::Duration,
    chain: &mut std::vec::IntoIter<String>,
    last_resort_used: &mut bool,
    compact_retries: &mut usize,
    transient_retries: &mut u32,
    rate_limit_waits: &mut u32,
    pinned_turn: bool,
    pinned_rl_attempts: &mut u32,
    pinned_rl_waited: &mut std::time::Duration,
    pinned_outage_attempts: &mut u32,
    pinned_outage_waited: &mut std::time::Duration,
    pinned_outage_warned_halfway: &mut bool,
    step: usize,
) -> Result<(u64, forge_provider::ModelResponse), CoreError> {
    let tools_before = tools_ran.load(std::sync::atomic::Ordering::Relaxed);
    // Stream the reply, with transparent failover for this step's completion.
    let mut failover_hop = 0u32;
    let resp = loop {
        // Bound what we send to the active model's context window (fetched/heuristic), so a
        // long conversation can't overflow it — which otherwise fails the turn as
        // "unavailable" on every model in the chain. Re-trimmed per model so failover to a
        // smaller-window model still fits. The immutable borrow ends before the block below.
        let sent = session.transcript_with_preamble(&active_model);
        // Auto-routed completions reserve a model before dispatch so independent sessions
        // can distribute across the fallback chain. Explicit pins deliberately bypass this
        // scheduler: their existing pin outage/failover policy remains authoritative.
        let reservation = (!explicit_pin)
            .then(|| session.store.try_reserve_model(&active_model))
            .flatten();
        let reserved = reservation.is_some();
        // Pre-dispatch key backstop: a model can reach here with NO provider key via a path
        // that isn't key-filtered (the last-resort fallback, or an architect editor/planner
        // default). Dispatching it just yields a no-auth genai "Resolver error" surfaced raw
        // to the user (the "groq for everything" report on a box with no groq key). Instead
        // synthesize a permanent Auth failure so the existing failover branch EXCLUDES it and
        // advances to a usable model. `has_api_key` is true for keyless providers (ollama,
        // the claude/codex bridges), so a legitimate bridge turn is never short-circuited.
        let attempt_started_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        let attempt_started = std::time::Instant::now();
        let result = if !explicit_pin && reservation.is_none() {
            Err(forge_provider::ProviderError::Unavailable(format!(
                "model '{active_model}' is serving another session"
            )))
        } else if forge_config::is_model_disabled(&active_model, &session.config.mesh.disabled)
            || !forge_config::has_api_key(forge_config::provider_of(&active_model))
        {
            Err(forge_provider::ProviderError::Auth(format!(
                "model '{}' is disabled or has no API key configured for provider '{}'",
                active_model,
                forge_config::provider_of(&active_model)
            )))
        } else {
            session.presenter.emit(PresenterEvent::ProviderRequest {
                model: active_model.clone(),
                step,
            });
            let provider = &session.provider;
            let presenter = &mut session.presenter;
            // Bump on every stream event so the idle watchdog can distinguish a live
            // stream from a stalled half-open connection — a stall fails over (below)
            // instead of hanging the turn forever.
            let activity = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let act = std::sync::Arc::clone(&activity);
            let active_tools = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let active = std::sync::Arc::clone(&active_tools);
            let tools = std::sync::Arc::clone(&tools_ran);
            let inspects = std::sync::Arc::clone(&inspect_ran);
            let build_fight = std::sync::Arc::clone(&bridge_build_fight);
            let verification = std::sync::Arc::clone(&verification_ledger);
            let pending_observations = std::sync::Arc::clone(&bridge_observations);
            let tools_unavailable = std::sync::Arc::clone(&mcp_tools_unavailable);
            let suppress_assistant_text = verify_attempts > 0;
            let mut sink = |ev: StreamEvent| {
                handle_stream_event(
                    ev,
                    presenter.as_mut(),
                    suppress_assistant_text,
                    in_plan_mode,
                    &mut proposed_plan,
                    &act,
                    &active,
                    &tools,
                    &inspects,
                    &build_fight,
                    &verification,
                    &pending_observations,
                    &tools_unavailable,
                );
            };
            let completion_opts = CompletionOptions {
                effort: session.pinned_effort,
                temperature: Some(CODING_TEMPERATURE),
                checkpoint: Some(checkpoint_ctx.clone()),
                prompt_cache_key: Some(checkpoint_ctx.session.clone()),
                max_output_tokens: None,
                reuse_response_chain,
                response_chain_prefix_tokens: completion_prefix_tokens(&sent, specs),
                response_format: None,
            };
            let fut =
                provider.complete_with(&active_model, &sent, specs, &completion_opts, &mut sink);
            stream_with_idle_timeout(fut, &activity, Some(&active_tools), stream_idle).await
        };
        if let Err(error) = &result {
            let error_kind = if error.is_auth() {
                "auth"
            } else if error.is_rate_limited() {
                "rate_limited"
            } else if error.is_context_overflow() {
                "context_overflow"
            } else if error.is_permanent() {
                "permanent"
            } else {
                "transient"
            };
            let _ = session.store.record_mesh_outcome(&MeshOutcome {
                session_id: session.id.clone(),
                model: active_model.clone(),
                tier: decision.map_or(TaskTier::Standard, |d| d.tier),
                started_at: attempt_started_at,
                completed_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_secs() as i64),
                latency_ms: attempt_started.elapsed().as_millis() as u64,
                outcome: "failure".to_string(),
                error_kind: Some(error_kind.to_string()),
                failover_hop,
                tool_calls: 0,
                verified_completion: false,
            });
        }
        match result {
            Ok(r) => {
                let _ = session.store.record_mesh_outcome(&MeshOutcome {
                    session_id: session.id.clone(),
                    model: active_model.clone(),
                    tier: decision.map_or(TaskTier::Standard, |d| d.tier),
                    started_at: attempt_started_at,
                    completed_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_secs() as i64),
                    latency_ms: attempt_started.elapsed().as_millis() as u64,
                    outcome: "success".to_string(),
                    error_kind: None,
                    failover_hop,
                    tool_calls: r.tool_calls.len() as u32,
                    verified_completion: !r.wants_tools(),
                });
                if !r.content.is_empty() && r.wants_tools() {
                    session.presenter.emit(PresenterEvent::AssistantDone);
                }
                break r;
            }
            Err(e) if failover_enabled && !reserved && !explicit_pin => {
                // Another session owns this model's reservation. This is scheduling
                // pressure, not provider health: immediately advance the existing chain
                // without benching a healthy shared model.
                let mut picked = None;
                for next in chain.by_ref() {
                    if forge_config::is_model_disabled(&next, &session.config.mesh.disabled)
                        || session.store.is_model_reserved(&next)
                    {
                        continue;
                    }
                    match session.admit_failover_model(&next).await {
                        Ok(true) => {
                            picked = Some(next);
                            break;
                        }
                        Ok(false) => {
                            session.presenter.emit(PresenterEvent::Warning(format!(
                                "skipped {next} (declined compaction) — trying the next model"
                            )));
                        }
                        Err(e) => return Err(e),
                    }
                }
                match picked {
                    Some(next) => {
                        session.presenter.emit(PresenterEvent::Routing {
                            tier: decision
                                .map(|d| d.tier.as_str().to_string())
                                .unwrap_or_default(),
                            model: next.clone(),
                            rationale: format!("model busy: {active_model}"),
                        });
                        *active_model = next;
                        failover_hop = failover_hop.saturating_add(1);
                        *transient_retries = 0;
                        continue;
                    }
                    None => return Err(e.into()),
                }
            }
            // Context-overflow self-heal — a SEPARATE arm, NOT nested in the `is_retryable`
            // arm below where it used to sit DEAD: an over-window input is a non-retryable
            // `Request` error (is_retryable() == false), so that guard never admitted it and
            // the turn failed hard instead of recovering. Overflow IS recoverable: lower the
            // usable window and retry the SAME healthy model so `transcript_with_preamble`
            // trims the sent view harder. Non-destructive (the stored transcript is untouched)
            // and convergent even when our o200k estimate diverges from the model's own
            // tokenizer — each retry multiplies the cap down. Bounded by `compact_retries`.
            Err(e) if *compact_retries < 3 && e.is_context_overflow() => {
                *compact_retries += 1;
                let shrunk = (session.effective_context_window(&active_model) as u64 * 55 / 100)
                    .max(1) as u32;
                session.overflow_window_cap = Some((active_model.clone(), shrunk));
                session.presenter.emit(PresenterEvent::Warning(format!(
                "{active_model}: input exceeded the context window — trimming context and retrying"
            )));
                // Best-effort LLM compaction too: a cleaner summary when the summarize call
                // itself fits. The window cap above is the guarantee that the retry shrinks
                // regardless of whether compaction runs.
                let _ = session.compact(true).await;
                session.emit_context_gauge(&active_model);
                continue;
            }
            Err(e) if failover_enabled && (e.is_retryable() || e.is_context_overflow()) => {
                // Persist credential failures before applying pinned-model policy. A strict
                // pin correctly makes *this* turn fail rather than switch models, but the
                // expired credential applies to all aliases and must not remain routable
                // on the next mesh decision.
                let auth_error = e.is_auth();
                if auth_error {
                    session.record_model_failure(&active_model, &e, default_cooldown);
                }
                // A transient failure other than an explicit provider outage (for example
                // a dropped stream) gets a short same-model retry. An `Unavailable`
                // response is already a shared health signal: bench it and immediately
                // advance the fallback chain instead of delaying every concurrent turn.
                if *transient_retries < MAX_TRANSIENT_RETRIES
                    && should_retry_same_model_transient(&active_model, &e)
                    && !e.is_permanent()
                    && !e.is_rate_limited()
                    && !e.is_context_overflow()
                {
                    *transient_retries += 1;
                    let backoff =
                        std::time::Duration::from_millis(500u64 << (*transient_retries - 1));
                    // Use ModelSearch (status-bar indicator, not chat history) so transient
                    // retries don't spam the scrollback. The spinner already signals "working".
                    session.presenter.emit(PresenterEvent::ModelSearch {
                        model: active_model.clone(),
                        retrying: true,
                    });
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                // Strict pin semantics (harness-robustness wave 2, fix 2): the single
                // chooser for what this error may do given pin state. An explicit pin
                // forbids cross-model failover — `mesh.pin_failover = true` is the escape
                // hatch that restores the old switch-away behaviour end to end. The outage
                // gate (`mesh.pin_outage_wait_secs > 0`) is folded into `transient_outage`
                // here rather than inside the arm below, so `0` collapses straight to
                // `FailTurn` — no separate disabled-outage branch to keep in sync.
                // Context overflow is excluded even though it rides `Unavailable`: after
                // the compact retries above are spent, waiting can never shrink the input,
                // so backing off would burn the whole outage budget on a lost cause.
                let transient_outage =
                    !e.is_permanent() && !e.is_rate_limited() && !e.is_context_overflow();
                match failover_policy(
                    pinned_turn,
                    session.config.mesh.pin_failover,
                    e.is_rate_limited(),
                    transient_outage && session.config.mesh.pin_outage_wait_secs > 0,
                ) {
                    FailoverPolicy::SwitchModels => {} // fall through to wait/bench/chain
                    // Pinned rate-limit backoff (fix 1): a pin must pin. Retry the SAME
                    // model on the documented schedule (5s/15s/45s, then 60s-capped, ±20%
                    // jitter, ≤6 attempts, ≤180s total — the PINNED_RL_* constants),
                    // honoring a server `Retry-After` verbatim when the error carried one.
                    // Multi-credential rotation already ran inside the provider (one
                    // next-key retry for API keys in genai_provider.rs, one next-account
                    // retry for OAuth in xai_oauth.rs), so by the time the error reaches
                    // this loop every configured key/account is limited and waiting is the
                    // only same-model option left.
                    FailoverPolicy::BackoffSameModel if e.is_rate_limited() => {
                        let retry_after = match &e {
                            forge_provider::ProviderError::RateLimited { retry_after, .. } => {
                                *retry_after
                            }
                            _ => None,
                        };
                        let attempt = *pinned_rl_attempts + 1;
                        // Cheap jitter without a rand dependency: sub-second wall-clock
                        // nanos.
                        let jitter = f64::from(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.subsec_nanos())
                                .unwrap_or(0),
                        ) / 1e9;
                        let delay = pinned_backoff_delay(attempt, retry_after, jitter);
                        let budget = std::time::Duration::from_secs(PINNED_RL_TOTAL_WAIT_SECS);
                        if attempt <= PINNED_RL_MAX_ATTEMPTS && *pinned_rl_waited + delay <= budget
                        {
                            *pinned_rl_attempts = attempt;
                            *pinned_rl_waited += delay;
                            session.presenter.emit(PresenterEvent::Warning(format!(
                                "{active_model}: rate limited — retrying pinned model in \
                             {}s (attempt {attempt}/{PINNED_RL_MAX_ATTEMPTS})",
                                delay.as_secs().max(1)
                            )));
                            session.presenter.emit(PresenterEvent::ModelSearch {
                                model: active_model.clone(),
                                retrying: true,
                            });
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        // Backoff budget exhausted: fail the turn with the REAL error
                        // rather than silently running a different model than the pin.
                        session.presenter.emit(PresenterEvent::Warning(format!(
                            "{active_model}: still rate limited after \
                         {pinned_rl_attempts} backoff retries — failing the turn \
                         (pinned model; cross-model failover disabled)"
                        )));
                        return Err(e.into());
                    }
                    // Transient outage (Unavailable, typically) that survived the hot
                    // same-model retries above (pinned-outage-resilience §1): same
                    // schedule as the RL backoff, but its OWN, longer budget
                    // (`mesh.pin_outage_wait_secs`, default 600s) via separate counters —
                    // an outage recovers in minutes, not on a signaled `Retry-After`, and
                    // must not eat into (or be eaten by) the RL budget above. The match
                    // above already gated `mesh.pin_outage_wait_secs > 0` into
                    // `transient_outage`, so this arm never runs with the budget disabled.
                    FailoverPolicy::BackoffSameModel => {
                        let attempt = *pinned_outage_attempts + 1;
                        // Cheap jitter without a rand dependency: sub-second wall-clock
                        // nanos.
                        let jitter = f64::from(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.subsec_nanos())
                                .unwrap_or(0),
                        ) / 1e9;
                        // No server Retry-After for an outage — always the blind schedule.
                        let delay = pinned_backoff_delay(attempt, None, jitter);
                        let budget = std::time::Duration::from_secs(
                            session.config.mesh.pin_outage_wait_secs,
                        );
                        if *pinned_outage_waited + delay <= budget {
                            *pinned_outage_attempts = attempt;
                            *pinned_outage_waited += delay;
                            // One-shot warning the first time cumulative wait crosses 50%
                            // of the budget: frequent enough that the user knows this is
                            // still going, rare enough not to spam the scrollback across
                            // many 60s-capped retries. Every retry still surfaces via the
                            // status-bar ModelSearch event below, no scrollback spam.
                            if !*pinned_outage_warned_halfway
                                && pinned_outage_waited.as_secs_f64() >= budget.as_secs_f64() * 0.5
                            {
                                *pinned_outage_warned_halfway = true;
                                let remaining = budget.saturating_sub(*pinned_outage_waited);
                                session.presenter.emit(PresenterEvent::Warning(format!(
                                    "{active_model}: provider unreachable — retrying \
                                 pinned model for up to {}s more (a pin never \
                                 switches models; `/model` to unpin, or set \
                                 `mesh.pin_failover = true` to allow mesh fallback)",
                                    remaining.as_secs().max(1)
                                )));
                            }
                            session.presenter.emit(PresenterEvent::ModelSearch {
                                model: active_model.clone(),
                                retrying: true,
                            });
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        // Outage budget exhausted: fail the turn with the REAL error,
                        // mirroring the rate-limit exhaustion wording above.
                        session.presenter.emit(PresenterEvent::Warning(format!(
                            "{active_model}: still unreachable after \
                         {pinned_outage_attempts} backoff retries — failing the turn \
                         (pinned model; cross-model failover disabled; `/model` to \
                         unpin, or set `mesh.pin_failover = true` to allow mesh \
                         fallback)"
                        )));
                        return Err(e.into());
                    }
                    FailoverPolicy::FailTurn => {
                        // A pinned model with a permanent incapability, or a transient
                        // outage with `mesh.pin_outage_wait_secs = 0` (outage backoff
                        // disabled), can't serve this turn, and switching models is
                        // forbidden: surface the real error.
                        return Err(e.into());
                    }
                }
                // Rate-limit on the current (best-ranked) model with a SHORT reset: WAIT for
                // it to reset and retry the SAME model instead of degrading to a lower-ranked
                // (or, pre-strict, paid) one. This is the per-minute free-tier case
                // (NIM/Groq/Gemini) — "retry when it's reset", not an instant fall to a worse
                // model. Bounded by a per-turn wait budget and a cap on the reset length, so a
                // long/daily quota (or a model that stays limited) still falls through to the
                // normal bench + failover below.
                let wait_cap =
                    std::time::Duration::from_secs(session.config.mesh.rate_limit_wait_secs);
                if pinned_turn
                    && e.is_rate_limited()
                    && !wait_cap.is_zero()
                    && *rate_limit_waits < MAX_RATE_LIMIT_WAITS
                {
                    let reset = e.cooldown(default_cooldown);
                    if reset <= wait_cap {
                        *rate_limit_waits += 1;
                        session.presenter.emit(PresenterEvent::Warning(format!(
                            "{active_model}: rate-limited — waiting {}s for reset, then retrying",
                            reset.as_secs().max(1)
                        )));
                        session.presenter.emit(PresenterEvent::ModelSearch {
                            model: active_model.clone(),
                            retrying: true,
                        });
                        tokio::time::sleep(reset).await;
                        continue;
                    }
                }
                // Auth failures exclude the whole provider; permanent capability failures
                // exclude only this model; transient failures take a short bench.
                if !auth_error {
                    session.record_model_failure(&active_model, &e, default_cooldown);
                }
                // Drive the single animated "finding a model" indicator instead of emitting
                // one scrollback warning per hop (the failover spam). It clears itself when
                // real output begins; the chain-exhausted case below still surfaces an error.
                session.presenter.emit(PresenterEvent::ModelSearch {
                    model: active_model.clone(),
                    retrying: false,
                });
                // Lazy 429-skip: the chain is in strict mesh-rank order, but a rate limit is
                // usually provider-wide, so trying the failed provider's lower-ranked
                // siblings next would just 429 again. ONLY on a rate-limit, skip this
                // provider's remaining chain entries and cross to the next provider; every
                // other failure keeps rank order intact. (Without this, dropping the old
                // provider-interleave would re-expose the 429-storm the interleave guarded.)
                let skip_provider = if e.is_rate_limited() || e.is_permanent() {
                    Some(forge_config::provider_of(&active_model).to_string())
                } else {
                    None
                };
                // Advance down the chain to the next model we can use. A model whose window
                // still holds the conversation is used immediately; one that's too small is
                // a switch that needs (lossy) compaction, so it's gated by consent
                // (Yes/No/Always) — "No" skips it and we keep looking for one that fits.
                let freshly_benched = session.store.current_benched().unwrap_or_default();
                let mut picked = None;
                for next in chain.by_ref() {
                    if forge_config::is_model_disabled(&next, &session.config.mesh.disabled) {
                        continue;
                    }
                    if let Some(p) = &skip_provider {
                        if forge_config::provider_of(&next) == p.as_str() {
                            continue;
                        }
                    }
                    // The original chain was built before this failure. Re-read health
                    // so an auth failure's new provider-wide bench immediately skips its
                    // sibling aliases in THIS turn, not only on the next one.
                    if freshly_benched.is_benched(&next) {
                        continue;
                    }
                    match session.admit_failover_model(&next).await {
                        Ok(true) => {
                            picked = Some(next);
                            break;
                        }
                        Ok(false) => {
                            session.presenter.emit(PresenterEvent::Warning(format!(
                                "skipped {next} (declined compaction) — trying the next model"
                            )));
                        }
                        Err(e) => return Err(e),
                    }
                }
                let Some(d) = decision else {
                    return Err(CoreError::Internal(
                        "failover engaged without a routing decision".into(),
                    ));
                };
                match picked {
                    Some(next) => {
                        session.presenter.emit(PresenterEvent::Routing {
                            tier: d.tier.as_str().to_string(),
                            model: next.clone(),
                            rationale: format!("failover from {active_model}"),
                        });
                        *active_model = next;
                        failover_hop = failover_hop.saturating_add(1);
                        *transient_retries = 0;
                        continue;
                    }
                    // The routed chain is exhausted. Rather than hard-fail, make ONE
                    // last-resort attempt on the "least dead" model — the non-excluded
                    // model whose transient bench expires soonest. This keeps a turn
                    // working when every model is briefly rate-limited but none is
                    // permanently incapable. Guarded by `last_resort_used` so a model that
                    // fails again can't loop.
                    None => match session.last_resort_model(&active_model, *last_resort_used) {
                        Some(m) => {
                            *last_resort_used = true;
                            session.presenter.emit(PresenterEvent::Routing {
                                tier: d.tier.as_str().to_string(),
                                model: m.clone(),
                                rationale: "last-resort: least-recently-benched model".to_string(),
                            });
                            *active_model = m;
                            failover_hop = failover_hop.saturating_add(1);
                            *transient_retries = 0;
                            continue;
                        }
                        // Nothing left to try. The per-hop failure only ever surfaced as a
                        // status-bar `ModelSearch`, so without this the provider's real,
                        // actionable message (expired credential, capability failure) was
                        // dropped and the user was told to wait out a rate limit that
                        // doesn't exist. Mirror `advance_fallback`'s terminal warning and
                        // carry the error into the returned CoreError.
                        None => {
                            let reason = e.reason();
                            session.presenter.emit(PresenterEvent::Warning(format!(
                                "{active_model} {reason} — model chain exhausted: {e}"
                            )));
                            return Err(CoreError::NoHealthyModel {
                                model: active_model.clone(),
                                reason,
                                last_error: e.to_string(),
                            });
                        }
                    },
                }
            }
            Err(e) => return Err(e.into()),
        }
    };
    Ok((tools_before, resp))
}
