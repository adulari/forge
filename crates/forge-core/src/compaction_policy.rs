//! Session compaction and context-budget policy.
//!
//! This owner keeps related session invariants together behind the Session program.

use super::*;
pub(crate) use crate::model_failure_record::record_model_failure_in;

/// Minimum age of a prior auth failure before it corroborates a new one into a provider-wide
/// exclusion. Sized to separate a repeat from a burst of concurrent turns, not to be a cooldown.
///
/// Picked from the measured incident rather than from taste: on 2026-08-30 the false provider-wide
/// exclusion of a healthy claude-cli subscription fired seven times, and every occurrence followed
/// several hosted sessions starting against the same bridge — at 11:59 the fourth session's start
/// and the exclusion were about 60 seconds apart. 120 s is that observed spread doubled, which is
/// the right direction to round: a gap too SHORT re-admits exactly this bug, while a gap too long
/// only costs a genuinely dead credential some extra per-model benches, each of which still fails
/// over and completes the turn.
const AUTH_ESCALATION_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(120);

/// How recently the provider must have COMPLETED a turn for a fresh "not logged in" to be treated
/// as an accident of one subprocess rather than as a statement about the credential.
///
/// Measured from the incident of 2026-09-02: `claude-cli::fable` was excluded for 30 minutes on
/// `Not logged in · Please run /login` while the same account answered a direct probe seconds
/// later, and the same bridge had completed turns moments before. Forge's claude bridge points
/// every subprocess at one shared credentials file, so a concurrent OAuth token refresh leaves a
/// sibling process momentarily reading no valid token. Five minutes is well inside the lifetime of
/// a working session and well under the exclusion it prevents.
const AUTH_RECENT_SUCCESS_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);

/// Scope at which one authentication failure is persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthFailureScope {
    /// A repeat, separated in time from an earlier one: the credential really is bad.
    Provider,
    /// First failure with no counter-evidence: bench this model for the auth window.
    Model,
    /// The provider answered moments ago — bench only this model, briefly, and don't call it an
    /// exclusion.
    ModelUnconfirmed,
}

/// Decide the scope for one auth failure from the two facts that bear on it.
pub(crate) fn auth_failure_scope(
    corroborated: bool,
    provider_succeeded_recently: bool,
) -> AuthFailureScope {
    if corroborated {
        AuthFailureScope::Provider
    } else if provider_succeeded_recently {
        AuthFailureScope::ModelUnconfirmed
    } else {
        AuthFailureScope::Model
    }
}

/// Persist an authentication failure at the narrowest scope the evidence supports.
///
/// One failed turn is not proof that a subscription's credential is dead. A CLI bridge that
/// exits abnormally for an unrelated reason still prints something auth-shaped on stderr, and
/// that is enough to reach here — which is how a healthy claude-cli subscription was benched
/// provider-wide for 24 hours while `forge run --model claude-cli::sonnet` answered in four
/// seconds. So the first failure benches only the MODEL, and provider scope waits for a repeat
/// that is separated in time from the first by at least [`AUTH_ESCALATION_MIN_GAP`].
///
/// The gap matters as much as the repeat. Concurrent turns against one provider fail together
/// within the same second, so a bare "has it failed before?" would let a single burst of
/// parallel sessions escalate exactly as before — which is what normal parallel use of Forge
/// looks like.
///
/// A first failure is narrowed further still when the provider COMPLETED a turn within
/// [`AUTH_RECENT_SUCCESS_WINDOW`].
///
/// A free function over the store rather than a `Session` method so the observed sequence
/// (success → one "not logged in" → success) is testable without a live session.
pub(crate) fn record_auth_failure_in(
    store: &forge_store::Store,
    model: &str,
    reason: &str,
    now: i64,
) {
    let provider = forge_config::provider_of(model);
    let corroborated = store
        .provider_auth_failed_before(provider, now - AUTH_ESCALATION_MIN_GAP.as_secs() as i64)
        .unwrap_or(false);
    let succeeded_recently = store
        .provider_succeeded_since(provider, now - AUTH_RECENT_SUCCESS_WINDOW.as_secs() as i64)
        .unwrap_or(false);
    let _ = match auth_failure_scope(corroborated, succeeded_recently) {
        AuthFailureScope::Provider => store.exclude_provider(provider, reason),
        AuthFailureScope::Model => store.exclude_model_auth(model, reason),
        AuthFailureScope::ModelUnconfirmed => store.bench_model_auth_unconfirmed(model, reason),
    };
}

impl Session {
    /// Real BPE token count of the current transcript (content + tool calls + per-message framing),
    /// via [`tokens`]. Used to decide compaction + drive the gauge; not billed. UI-only messages
    /// are excluded — they never reach a provider, so they must not inflate the gauge or trip
    /// auto-compaction.
    pub(crate) fn estimated_transcript_tokens(&self) -> u64 {
        // The pinned block rides on every request though it is not in the transcript; counting it
        // keeps the gauge and the compaction trigger where they were when its text was.
        self.transcript
            .iter()
            .filter(|m| m.visibility.is_llm())
            .map(|m| message_tokens(m) as u64)
            .sum::<u64>()
            + self.pinned_tokens() as u64
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
            if ans == forge_types::NO_ANSWER {
                // Nobody can answer (headless `forge run`, a detached serve session). Treating
                // that as "No" skipped every smaller-window fallback and ended the turn on the
                // last provider error, so an unattended session compacts and carries on.
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "no one to ask — compacting to continue on {model}"
                )));
            } else if ans == "always" {
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
        let trigger = self.compaction_trigger(model);
        if needs_compaction(
            self.context_pressure_tokens(),
            trigger,
            self.transcript_fits(model),
        ) {
            // Cheap first: advance the elision floor to the working set and cut the bulky results
            // it releases (no model call). Often reclaims enough that the summary isn't needed.
            if self.advance_floor_if_due(model, true) > 0 {
                self.emit_context_gauge(model);
            }
            // Either step goes down to the low-water mark, not just under the trigger: stopping at
            // the trigger means the next tool result trips it again, and every trip rewrites the
            // prompt — a session at its ceiling re-trimmed on every single request.
            let low_water =
                (trigger as f64 * self.config.mesh.compact_low_water.clamp(0.1, 1.0)) as u64;
            if needs_compaction(
                self.context_pressure_tokens(),
                low_water,
                self.transcript_fits(model),
            ) {
                let (before, after) = self.compact(true).await.unwrap_or_default();
                self.enforce_post_compact_headroom(trigger);
                // Continual Harness auto-refine gate (`harness.auto_refine = "compact"`,
                // refinement.rs): only when this call actually folded something — `compact`
                // no-ops (before == after) when there isn't enough older history yet, and firing
                // a refinement pass off a no-op compaction would have nothing new to learn from.
                if before != after
                    && self.config.harness.enabled
                    && self.config.harness.auto_refine == forge_config::AutoRefineMode::Compact
                {
                    let _ = self.refine(None, false, "auto_compact").await;
                }
            }
            // Refresh the gauge NOW so it reflects the reduced context immediately, instead of
            // showing the old (over-window) size until the turn's first model call returns.
            self.emit_context_gauge(model);
        }
    }

    /// Emit the session's PERSISTED totals the moment it is resumed, before any turn runs.
    ///
    /// Cost, tokens, and context fill all live on the `Cost` event, and that event is only emitted
    /// once a turn reaches its end (or `emit_context_gauge` fires mid-turn). So a resumed session
    /// showed a spend meter of `untracked` and an empty token counter until its first turn
    /// COMPLETED in the new process — on a long turn that is many minutes of the statusline
    /// claiming the session had cost nothing, when the store knew it had cost $1.69.
    ///
    /// The numbers were never lost: `session.total_cost_usd` accumulates across processes and
    /// [`Store::session_cost`] reads it. Only the display started from zero, which reads exactly
    /// like the spend was reset by restarting.
    ///
    /// `context_limit` is deliberately `None`: no model has been routed yet, so there is no honest
    /// denominator to show. The gauge fills in on the first real call.
    pub fn emit_restored_totals(&mut self) {
        let session_total_usd = self.store.session_cost(&self.id).unwrap_or(0.0);
        let (session_in, session_out) = self.store.session_tokens(&self.id).unwrap_or((0, 0));
        if session_total_usd <= 0.0 && session_in == 0 && session_out == 0 {
            return; // a genuinely fresh session has nothing to restore
        }
        self.presenter.emit(PresenterEvent::Cost {
            session_total_usd,
            session_in,
            session_cached_in: self
                .store
                .session_cached_input_tokens(&self.id)
                .unwrap_or(None),
            session_out,
            context_tokens: self.estimated_transcript_tokens(),
            // Without this the gauge falls back to a 128k "~approx" denominator, which for a
            // million-token model reads as 100% full on a session that is nowhere near it — the
            // restored cost is right but the context bar is alarming and wrong. The window is
            // knowable here: the pin if there is one, else the model this session last routed to.
            context_limit: self.restored_context_window(),
        });
    }

    /// The context window to show for a session being resumed, before any turn has run.
    ///
    /// Prefers the pin (that is the model the next turn will use) and falls back to the model the
    /// session last routed to. `None` only when neither is known, which keeps the old
    /// conservative-fallback behaviour for a session with no routing history.
    fn restored_context_window(&self) -> Option<u32> {
        let pinned = self
            .pinned_model
            .as_ref()
            .and_then(|set| set.first())
            .cloned();
        let model = pinned.or_else(|| self.store.session_last_model(&self.id).ok().flatten())?;
        Some(self.base_context_window(&model))
    }

    /// Emit a [`Cost`](PresenterEvent::Cost) event reflecting the CURRENT transcript size as the
    /// live context fill, so the statusline gauge + compaction band update right away (e.g. right
    /// after auto-compaction) rather than waiting for the next model call's real input-token count
    /// at turn end. Uses the conservative transcript estimate as a stand-in until the real count
    /// arrives.
    pub(crate) fn emit_context_gauge(&mut self, model: &str) {
        self.gauge_model = Some(model.to_string());
        let (session_in, session_out) = self.store.session_tokens(&self.id).unwrap_or((0, 0));
        let session_total_usd = self.store.session_cost(&self.id).unwrap_or(0.0);
        self.presenter.emit(PresenterEvent::Cost {
            session_total_usd,
            session_in,
            session_cached_in: self
                .store
                .session_cached_input_tokens(&self.id)
                .unwrap_or(None),
            session_out,
            context_tokens: self.context_pressure_tokens(),
            // The gauge denominator is the model's REAL window, not the transient overflow cap.
            context_limit: Some(self.base_context_window(model)),
        });
    }

    /// Emit the session's accounting as it stands in the store: spend, token totals (from which the
    /// TUI derives the per-turn deltas and throughput) and the live context fill. Called after
    /// every model call, not only at turn end: the statusline used to freeze for the whole turn
    /// because the only per-call write (`record_usage`) never told the presenter, so a long turn
    /// showed the cost, tokens and context gauge of the previous turn until it finished.
    pub(crate) fn emit_cost_snapshot(
        &mut self,
        context_tokens: u64,
        active_model: &str,
    ) -> Result<(), CoreError> {
        self.gauge_model = Some(active_model.to_string());
        let usage = self.store.session_token_usage(&self.id)?;
        self.presenter.emit(PresenterEvent::Cost {
            session_total_usd: self.store.session_cost(&self.id)?,
            session_in: usage.input_tokens,
            session_cached_in: usage.cached_input_tokens,
            session_out: usage.output_tokens,
            context_tokens,
            context_limit: Some(self.effective_context_window(active_model)),
        });
        Ok(())
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
        self.emit_cost_snapshot(context_tokens, active_model)?;
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
    /// authentication failure eventually applies to every alias of its provider and must stop
    /// sibling churn — but only once a repeat corroborates it (see the auth branch below).
    pub(crate) fn record_model_failure(
        &self,
        model: &str,
        err: &forge_provider::ProviderError,
        default_cooldown: std::time::Duration,
    ) {
        record_model_failure_in(&self.store, model, err, default_cooldown);
    }
}

pub(crate) fn model_health_reason<'a>(
    err: &forge_provider::ProviderError,
    detail: &'a str,
) -> &'a str {
    if matches!(err, forge_provider::ProviderError::Unavailable(_)) {
        if let Some(start) = detail.find("produced no output for ") {
            let tail = &detail[start..];
            if let Some(end) = tail.find(" — killed") {
                return &tail[..end];
            }
        }
    }
    err.reason()
}

impl Session {
    /// Leave the transcript exactly as it was when no summarizer could be had, and take down the
    /// progress band `CompactionStarted` raised — otherwise the TUI keeps showing a compaction
    /// that is no longer running.
    pub(crate) fn abandon_compaction(&mut self, before: usize) {
        self.presenter.emit(PresenterEvent::CompactionFinished {
            before,
            after: before,
        });
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

#[cfg(test)]
mod auth_failure_tests {
    use super::*;
    use forge_store::{MeshOutcome, Store};

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }

    /// Record a completed turn exactly as the model loop does on success.
    fn success(store: &Store, model: &str, at: i64) {
        store
            .record_mesh_outcome(&MeshOutcome {
                session_id: "s".into(),
                model: model.to_string(),
                tier: forge_types::TaskTier::Standard,
                started_at: at,
                completed_at: at,
                latency_ms: 1_000,
                outcome: "success".into(),
                error_kind: None,
                failover_hop: 0,
                tool_calls: 0,
                verified_completion: true,
            })
            .unwrap();
        store.clear_health_after_success(model).unwrap();
    }

    fn provider_excluded(store: &Store) -> bool {
        !store.current_excluded_providers().unwrap().is_empty()
    }

    fn health_of(store: &Store, model: &str) -> Option<(i64, String)> {
        store
            .current_benched_report()
            .unwrap()
            .into_iter()
            .find(|(m, _, _)| m == model)
            .map(|(_, until, reason)| (until, reason))
    }

    /// The observed 2026-09-02 sequence: the bridge completed turns, then one subprocess printed
    /// `Not logged in · Please run /login` while the account was logged in the whole time, then it
    /// completed turns again. The provider must not be excluded, and the single false verdict must
    /// not cost the model a 30-minute exclusion.
    #[test]
    fn one_false_not_logged_in_between_successes_does_not_exclude_the_provider() {
        let store = Store::open_in_memory().unwrap();
        success(&store, "claude-cli::opus", now() - 30);

        record_auth_failure_in(
            &store,
            "claude-cli::fable",
            "claude error: Not logged in · Please run /login",
            now(),
        );

        assert!(!provider_excluded(&store), "provider must stay routable");
        let (until, reason) = health_of(&store, "claude-cli::fable").expect("model benched");
        assert!(
            !reason.starts_with("excluded:"),
            "an unconfirmed auth failure is a bench, not an exclusion: {reason}"
        );
        assert!(
            until - now() <= 300,
            "and it must be minutes, not the 30-minute auth exclusion"
        );

        // The next successful turn retires it entirely.
        success(&store, "claude-cli::fable", now());
        assert!(health_of(&store, "claude-cli::fable").is_none());
        assert!(!provider_excluded(&store));
    }

    /// With no recent success behind it, one auth failure still costs the model the full auth
    /// exclusion — the pre-existing guarantee, unchanged.
    #[test]
    fn a_first_failure_without_a_recent_success_still_excludes_the_model() {
        let store = Store::open_in_memory().unwrap();
        record_auth_failure_in(&store, "claude-cli::fable", "not logged in", now());
        assert!(!provider_excluded(&store));
        let (_, reason) = health_of(&store, "claude-cli::fable").unwrap();
        assert!(reason.starts_with("excluded: auth failed"), "{reason}");
    }

    /// A genuinely logged-out account still loses the provider: the unconfirmed bench keeps the
    /// auth marker, so a repeat separated by more than the escalation gap corroborates it.
    #[test]
    fn a_repeated_failure_after_a_success_still_excludes_the_provider() {
        let store = Store::open_in_memory().unwrap();
        success(&store, "claude-cli::opus", now() - 30);

        let first = now() - 180;
        record_auth_failure_in(&store, "claude-cli::fable", "not logged in", first);
        // Age the unconfirmed bench to when it was actually written (the helper owns the clock);
        // it is still in force, which is what lets the repeat corroborate.
        let (_, reason) = health_of(&store, "claude-cli::fable").unwrap();
        store
            .bench_model_at("claude-cli::fable", first + 300, &reason, first)
            .unwrap();

        record_auth_failure_in(&store, "claude-cli::sonnet", "not logged in", now());
        assert!(
            provider_excluded(&store),
            "a repeat separated in time is a real logout"
        );
    }
}

/// Whether the transcript must be folded: over the configured ceiling, OR no longer fitting the
/// model's window. Both call sites in [`Session::auto_compact_if_needed`] ask this same question —
/// the one before the cheap prune, and the one after it that decides whether to spend a model call
/// on summarizing.
///
/// That second call site used to ask only `transcript_fits`, i.e. the WINDOW, which made the
/// ceiling decorative on any large-window model. The outer check let a 757k-token transcript
/// through, the prune reclaimed what it could, and then the summarize step asked "does 757k fit in
/// 1,048,576?", answered yes, and returned. So a ceiling could only ever trigger the cheap prune,
/// never the compaction that actually bounds a transcript — and a session grew until it approached
/// the window itself (832,307 tokens on a 1M-token model) no matter what the ceiling said.
pub(crate) fn needs_compaction(transcript_tokens: u64, trigger: u64, fits_window: bool) -> bool {
    transcript_tokens > trigger || !fits_window
}

/// Move a compaction split back off the middle of a tool round. `split` is the first message kept
/// verbatim; when that is a tool result its call would be folded into the summary and the results
/// left behind would answer a call the provider cannot see. Moonshot rejects such a request
/// outright, and everyone else loses those results to the pairing repair. Walking back to the
/// call keeps a few more messages than `COMPACT_KEEP_RECENT`, which is the cheaper mistake.
#[cfg(test)]
pub(crate) use crate::compaction_shape::{kept_tail_start, summary_too_thin};

pub(crate) fn round_aligned_split(messages: &[Message], mut split: usize) -> usize {
    while split > 0 && split < messages.len() && messages[split].role == Role::Tool {
        split -= 1;
    }
    split
}

/// The absolute token ceiling at which a session auto-compacts, before the model's own window is
/// taken into account. Two different ceilings, because they answer two different questions.
///
/// [`forge_config::MeshConfig::compact_cap_tokens`] is a SPEND control — it exists to stop a
/// large-window subscription model burning quota on an ever-growing prompt. A free model has no
/// quota to burn, so applying it there just throws away context for nothing: on a 1M-window free
/// model the 217_600 default fired auto-compaction at ~21% of the window.
///
/// [`forge_config::MeshConfig::free_model_cap_tokens`] is what a free model gets INSTEAD. Exempting
/// free models from the spend cap by handing them `u64::MAX` left them with no absolute ceiling at
/// all, so the only remaining limit was `AUTO_COMPACT_THRESHOLD * window` — 838,860 tokens on a 1M
/// window. Free buys tokens, not attention: a turn carrying 800k of mostly-stale tool output is not
/// better than one carrying a compacted 400k, and every later turn re-sends the whole thing.
///
/// Both are the GLOBAL fallback. A per-model or per-provider ceiling (`[compact_cap]`,
/// [`forge_config::compact_cap_for`]) takes precedence at the call site: a request-metered
/// subscription with a mid-sized window (Kimi Code) is better served by a lower ceiling than a
/// pay-per-token API with a 1M window, and one number cannot fit both.
pub(crate) fn compaction_cap(free: bool, mesh: &forge_config::MeshConfig) -> u64 {
    if free {
        mesh.free_model_cap_tokens
    } else {
        mesh.compact_cap_tokens
    }
}

#[cfg(test)]
mod compaction_cap_tests {
    use super::*;
    use crate::{auto_compact_trigger_tokens, AUTO_COMPACT_THRESHOLD};

    /// The window of `meta::muse-spark-1.3-contributor` and every other muse-spark model, as
    /// OpenRouter publishes it and as the store records it.
    const MUSE_WINDOW: u64 = 1_048_576;

    /// The measured failure, and why this PR's ceiling alone would have changed nothing.
    /// `compact_cap_tokens` was restored to its 217_600 default on a real machine and the session
    /// still sent 757,178 input tokens per request, because the step that would have folded it
    /// asked about the WINDOW instead of the ceiling.
    #[test]
    fn the_ceiling_governs_the_summarize_step_not_only_the_prune() {
        // The reserve for the default `max_output_tokens = 0` is
        // UNBOUNDED_OUTPUT_PLANNING_RESERVE, and `transcript_fits` keeps 80% of what is left.
        let fits_window_at = (MUSE_WINDOW - 8_192) * 8 / 10;
        assert_eq!(fits_window_at, 832_307);

        let mesh = forge_config::Config::default().mesh;
        let trigger = auto_compact_trigger_tokens(
            MUSE_WINDOW,
            compaction_cap(false, &mesh),
            AUTO_COMPACT_THRESHOLD,
        );
        assert_eq!(trigger, 217_600);

        // What a real session was carrying, per request, with that 217_600 ceiling in force.
        let observed = 757_178;
        let fits_window = observed <= fits_window_at;
        assert!(
            fits_window,
            "it fit the window — which is exactly why the old gate returned without compacting"
        );
        assert!(
            needs_compaction(observed, trigger, fits_window),
            "3.5x over the ceiling must compact even though it still fits the window"
        );
    }

    #[test]
    fn a_transcript_under_the_ceiling_that_fits_is_left_alone() {
        assert!(!needs_compaction(100_000, 217_600, true));
    }

    /// The ceiling is an upper bound, never a licence to overflow: a model whose window is smaller
    /// than the ceiling must still compact on the window alone.
    #[test]
    fn a_transcript_under_the_ceiling_that_no_longer_fits_still_compacts() {
        assert!(needs_compaction(30_000, 217_600, false));
    }

    #[test]
    fn a_free_model_still_has_a_ceiling_on_a_million_token_window() {
        let mesh = forge_config::Config::default().mesh;

        // The regression: with the spend cap replaced by `u64::MAX`, the ONLY remaining limit was
        // the fraction of the window, and a 1M-window free model ran to 838,860 tokens before
        // anything folded it. Measured consequence on a real session: requests carrying 300k-720k
        // of context, 90% of it stale tool output.
        let unbounded = auto_compact_trigger_tokens(MUSE_WINDOW, u64::MAX, AUTO_COMPACT_THRESHOLD);
        assert_eq!(unbounded, 838_860);

        let free = auto_compact_trigger_tokens(
            MUSE_WINDOW,
            compaction_cap(true, &mesh),
            AUTO_COMPACT_THRESHOLD,
        );
        assert_eq!(free, 400_000, "a free model compacts at its own ceiling");
        assert!(free < unbounded / 2);
    }

    #[test]
    fn the_free_ceiling_never_second_guesses_the_spend_cap_for_a_paid_model() {
        let mesh = forge_config::Config::default().mesh;
        let paid = compaction_cap(false, &mesh);
        assert_eq!(paid, mesh.compact_cap_tokens);
        assert!(
            compaction_cap(true, &mesh) > paid,
            "the free ceiling is a backstop, deliberately looser than the spend control — a free \
             model that compacted EARLIER than a paid one would be the original bug inverted"
        );
    }

    #[test]
    fn a_small_window_still_wins_over_either_ceiling() {
        // Both caps are absolute ceilings, not targets: a model whose real window is smaller must
        // still compact at its own 80%, or the transcript stops fitting at all.
        let mesh = forge_config::Config::default().mesh;
        for free in [true, false] {
            let trigger = auto_compact_trigger_tokens(
                32_768,
                compaction_cap(free, &mesh),
                AUTO_COMPACT_THRESHOLD,
            );
            assert_eq!(trigger, 26_214, "free={free}");
        }
    }
}
