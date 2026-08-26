//! Stateful provider, failover, and tool-execution loop for a session.

use super::*;

impl Session {
    /// Shared model↔tool inner loop used by both the primary turn and the autofix re-run.
    ///
    /// * `active_model` – the model to start with; updated by failover.
    /// * `specs`        – tool specs to advertise (pre-built by the caller).
    /// * `decision`     – `Some(d)` for the primary turn (enables failover, step-0 routing
    ///   record, quota-hint persistence); `None` for autofix re-runs (no failover, no records).
    /// * `max_steps`    – step cap (runaway guard).
    /// * `stream_idle`  – idle-stream timeout forwarded to every `complete_with` call.
    pub(crate) async fn run_model_loop(
        &mut self,
        mut active_model: String,
        specs: &[ToolSpec],
        decision: Option<&forge_mesh::RoutingDecision>,
        reuse_response_chain: bool,
        max_steps: usize,
        stream_idle: std::time::Duration,
    ) -> Result<ModelLoopOutcome, CoreError> {
        let failover_enabled = decision.is_some() && self.config.mesh.failover;
        let default_cooldown =
            std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs);

        // Failover chain: only meaningful for the primary turn (decision is Some). The autofix
        // path passes None, so `chain` is immediately exhausted and failover never fires.
        let fallbacks: Vec<String> = decision.map(|d| d.fallbacks.clone()).unwrap_or_default();
        let mut chain = fallbacks.into_iter();
        let explicit_pin = self.pinned_model.is_some() || decision.is_some_and(|d| d.pinned);
        let mut last_resort_used = false;
        // Bounds the overflow self-heal (shrink + retry the SAME model) so a transcript that can't
        // be shrunk enough eventually falls through to normal failover instead of looping.
        let mut compact_retries = 0usize;
        // Fresh turn: drop any window cap left armed by a previous turn's overflow self-heal, so a
        // short new turn isn't stuck sending a needlessly-trimmed view.
        self.overflow_window_cap = None;
        // Bounds the same-model retry for transient errors (a 5xx / dropped connection that often
        // succeeds on a second attempt). Reset to 0 whenever we switch to a different model, so the
        // budget is per-model, not per-turn — "don't give up instantly" before failing over.
        let mut transient_retries = 0u32;
        // Bounds in-turn waits for a rate-limited model to RESET (per-minute free tiers). Per turn,
        // not per model: a few short waits total, so the turn can't block indefinitely.
        let mut rate_limit_waits = 0u32;
        // Pinned rate-limit backoff (harness-robustness wave 2, fix 1): whether this turn runs an
        // EXPLICITLY pinned model — the session `/model` pin, or a routing decision flagged as a
        // `--model` pin. A rate limit on a pinned model is waited out with exponential backoff on
        // the SAME model (see `pinned_backoff_delay`) instead of failing the turn.
        let pinned_turn = self.pinned_model.is_some() || decision.is_some_and(|d| d.pinned);
        let mut pinned_rl_attempts = 0u32;
        let mut pinned_rl_waited = std::time::Duration::ZERO;
        // Pinned outage backoff (pinned-outage-resilience §1): a SEPARATE attempt/budget pair so
        // an outage retry never eats into (or is eaten by) the rate-limit budget above — the two
        // failure modes can both occur in the same turn without starving each other. `warned` is
        // the one-shot latch for the 50%-of-budget Warning (below); a per-attempt Warning would
        // spam the scrollback the way the RL path's does, so outage retries only surface via the
        // status-bar ModelSearch event until the halfway point.
        let mut pinned_outage_attempts = 0u32;
        let mut pinned_outage_waited = std::time::Duration::ZERO;
        let mut pinned_outage_warned_halfway = false;

        let mut final_text = String::new();
        let mut has_prior_final = false;
        let mut context_tokens: u64 = 0;
        // Per-turn cumulative bridge input tokens (wave 5, fix 1). A CLI bridge runs its own tool
        // loop in a subprocess, so the direct-path cost guards never see it; this sums the input
        // reported by each bridge completion this turn so the token ceiling can end an unbounded
        // bridge turn at an observation boundary. Summing across re-drives may over-count if a
        // persistent bridge reports cumulative usage, but this is a backstop — tripping early is
        // safe. Only bridge completions feed it (direct turns leave it 0).
        let mut bridge_input_accum: u64 = 0;
        let mut hit_step_cap = true;
        let mut halted_by_loop_guard = false;
        // A plan a bridge model proposes via the out-of-band sink (StreamEvent::Plan). Captured by
        // the per-step stream closure and returned in the outcome for the turn's approval flow.
        // Only honored in planning mode (the bridge advertises present_plan unconditionally — it
        // can't see the parent's runtime temper — so the parent gates here): outside Plan mode a
        // stray plan is dropped, which also stops the post-approval build turn from re-proposing.
        let mut proposed_plan: Option<forge_types::PlanProposal> = None;
        let in_plan_mode = self.mode == PermissionMode::Plan;
        // Harness reliability guards. `empty_nudges`: bounded retries when the model returns nothing
        // (narrate-then-stall / transient empty) before giving up. `last_tool_sig`/`repeat_count`:
        // doom-loop detection — the same tool batch repeated DOOM_LOOP_THRESHOLD× halts the turn.
        let mut empty_nudges = 0usize;
        let mut last_tool_sig: Option<u64> = None;
        let mut repeat_count = 0usize;
        // `recent_sigs`: a short sliding window of recent tool-batch signatures. The consecutive
        // `repeat_count` above misses an A,B,A,B,… oscillation (every step differs from the one
        // before, so the counter keeps resetting) — e.g. a model alternating a failing/empty call
        // with a trivial successful one, which ALSO clears the failure-loop streak (a success on a
        // tool resets it). Counting how often a signature recurs in this window catches that.
        let mut recent_sigs: std::collections::VecDeque<u64> = std::collections::VecDeque::new();
        // `continue_nudges`: bounded retries when the model signs off with text but tracked tasks
        // are still unfinished (narrate-then-stall) — drive it to completion instead of ending the
        // turn mid-task. `doom_nudged`: the doom-loop fires a "change approach" nudge BEFORE it
        // ever hard-stops, so a repeated call doesn't kill an otherwise-recoverable turn.
        let mut continue_nudges = 0usize;
        // A model can mark every task done, pass the verification gate, then yield a sentence such
        // as "Let me verify runtime issues:" with no tool call. That is explicit unfinished intent,
        // not a final answer. Give it one bounded chance to perform the promised action.
        let mut followup_intent_nudges = 0usize;
        let mut doom_nudged = false;
        // Failure-loop guard (complements the identical-call doom-loop): counts tool failures by
        // (tool name, error kind) ACROSS the turn, so a model retrying the same KIND of error with
        // different args (edits that never match, reads of paths that don't exist) is caught even
        // though its call signature keeps changing. A success on a tool clears its streak.
        let mut failure_counts: std::collections::HashMap<(String, ErrorCategory), usize> =
            std::collections::HashMap::new();
        let mut failure_nudged = false;
        // `toolcall_repair_nudges`: bounded retries when a direct model writes a tool call as TEXT
        // (`<invoke>` / `default_api:` markup) that the provider couldn't decode AND the text
        // recovery pass missed — so nothing executed. Without this the narration is accepted as a
        // final answer and the turn "succeeds" having done nothing (the phantom-release bug).
        let mut toolcall_repair_nudges = 0usize;
        // `bridge_continue_nudges`: bounded RE-RUNS of a CLI bridge whose turn returned with tracked
        // tasks still unfinished. A bridge turn is otherwise terminal (it runs its own tool loop and
        // returns once), so a long multi-step plan stalls partway — the bridge does a few steps,
        // returns, and the turn ends with work pending (the half-finished release: merged + tagged
        // but brew-sha + verify never ran). This drives a clean re-run, exactly as the user typing
        // `continue` would.
        let mut bridge_continue_nudges = 0usize;
        // Verification gate: when a bridge reports every task Done, completion is NOT accepted on
        // its say-so. Fresh tool-grounded evidence newer than the last artifact mutation is enough;
        // otherwise Forge requests a verification turn. This is the completion AUTHORITY: "done"
        // means Forge observed proof, not merely that the model asserted it.
        // Verification attempts spent on the current "all done" claim. 0 = not yet verifying. The
        // gate forces the bridge to PROVE completion with a real inspection tool; a verification
        // turn that just re-marks `update_tasks` without inspecting doesn't count (the C8 hole — a
        // model told to lie re-confirmed done without checking). Bounded so it can't loop.
        let mut verify_attempts = 0usize;
        // One-shot guard for the opt-in completeness re-drive (`mesh.verify_completeness`): fired at
        // most once per turn so it can't loop. See the bridge-yield branch below.
        let mut completeness_checked = false;
        // Direct path only: the `inspect_ran` count at the moment the verify nudge was last issued.
        // An inspection that runs AFTER this point is the model responding to the request to verify
        // (on the direct path, tools run in separate steps from the text claim, so a step-local
        // signal can't see it). Carried across steps; reset implicitly by being re-stamped each nudge.
        let mut verification_at_last_verify: u64 = 0;
        // Completed-task count observed at the last bridge re-drive check — the other half of the
        // progress signal (a re-run that closes a task but happens to run no fresh tool still counts
        // as progress).
        let mut bridge_done_prev = self
            .tasks
            .iter()
            .filter(|t| matches!(t.status, forge_types::TodoStatus::Done))
            .count();
        // Counts tools that actually STARTED executing across the whole turn (bridge tools surface
        // here via the sink too). The bridge re-drive uses the per-step delta as its progress
        // signal: a re-run that completes no task AND runs no tool made no progress, so it's halted
        // rather than re-driven again (the anti-spiral guard the old bridge-nudge lacked).
        let tools_ran = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        // Counts build/provision tool STARTS a bridge surfaces via the sink across the whole turn
        // (wave 5, fix 2). Per-command success/failure isn't available from the sink, so this
        // approximates the direct-path env-fight spend cap with an invocation count; past
        // BRIDGE_BUILD_FIGHT_THRESHOLD it folds into the token-ceiling early-terminate.
        let bridge_build_fight = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        // Counts INSPECTION tools (anything except `update_tasks`/`present_plan`) — the verification
        // gate requires the bridge to actually CHECK real state, not just re-assert "done".
        let inspect_ran = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let verification_ledger =
            std::sync::Arc::new(std::sync::Mutex::new(VerificationLedger::default()));
        let bridge_observations =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
                String,
                std::collections::VecDeque<VerificationObservation>,
            >::new()));
        // Latched when a CLI-bridge completion reports `StreamEvent::ToolsUnavailable` — Forge's
        // `mcp-serve` tool server failed to start, so the model's write tools were never exposed
        // (wave 7). Read into the loop outcome so `run_turn` can classify + the harness can retry.
        let mcp_tools_unavailable = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // This turn's snapshot context, handed explicitly to each bridge completion so its
        // `forge mcp-serve` child snapshots edits into THIS turn's dir under the live temper — no
        // process-global env mutation. Computed once before the per-step borrows (the temper is
        // constant within a turn); non-bridge providers ignore it.
        let checkpoint_ctx = self.checkpoint_context();

        for step in 0..max_steps {
            // ── Timeout reconciliation window (quality guards wave 4, fix 2) ──────────────────
            // The caller's hard timeout (`bench swe`'s tokio kill) is invisible from inside the
            // turn, so without this the kill lands mid-verification and "submit partial work"
            // ships whatever risky state the tree is in. Past the soft deadline: stop launching
            // new completions, inject ONE revert instruction, allow one model turn to act on it
            // (its tool calls run in the same step), then end the loop normally. The latch is a
            // Session field so later loop re-entries this turn (nudges/guards) end immediately
            // instead of re-firing.
            if self.past_turn_deadline() {
                if self.deadline_reconciled {
                    hit_step_cap = false;
                    break;
                }
                self.deadline_reconciled = true;
                self.presenter.emit(PresenterEvent::Warning(
                    "turn deadline reached — asking the model to revert unverified changes and stop"
                        .to_string(),
                ));
                let dseq = self.next_seq();
                let _ = self.store.add_message(
                    &self.id,
                    dseq,
                    Role::System,
                    DEADLINE_RECONCILE_NUDGE,
                    None,
                );
                self.transcript
                    .push(Message::system(DEADLINE_RECONCILE_NUDGE));
            }
            let (tools_before, mut resp) = crate::model_request::request_provider_response(
                self,
                &mut active_model,
                specs,
                decision,
                reuse_response_chain,
                stream_idle,
                &checkpoint_ctx,
                verify_attempts,
                in_plan_mode,
                &mut proposed_plan,
                &tools_ran,
                &inspect_ran,
                &bridge_build_fight,
                &verification_ledger,
                &bridge_observations,
                &mcp_tools_unavailable,
                explicit_pin,
                failover_enabled,
                default_cooldown,
                &mut chain,
                &mut last_resort_used,
                &mut compact_retries,
                &mut transient_retries,
                &mut rate_limit_waits,
                pinned_turn,
                &mut pinned_rl_attempts,
                &mut pinned_rl_waited,
                &mut pinned_outage_attempts,
                &mut pinned_outage_waited,
                &mut pinned_outage_warned_halfway,
                step,
            )
            .await?;
            // Compute the real cost from token counts and the model's price (FR-5, A-7), pricing
            // cache-read tokens at the discounted rate so it tracks the provider's actual bill.
            resp.usage.cost_usd = self.pricing.cost_for_usage(&active_model, &resp.usage);
            // The last call's input size is the live context fill (tui-token-counter.md) — except a
            // subscription CLI bridge reports cumulative internal usage, so [`context_fill_tokens`]
            // substitutes the transcript estimate there (else the gauge reads a bogus 337% and trips
            // the phantom "auto-compact imminent" hint).
            context_tokens = context_fill_tokens(
                &active_model,
                self.estimated_transcript_tokens(),
                resp.usage.input_tokens,
            );

            let (msg_id, bridge_tool_progress) = self.record_model_response(
                step,
                &active_model,
                decision,
                &resp,
                tools_before,
                &tools_ran,
                &mut bridge_input_accum,
                &mut empty_nudges,
            )?;

            if !resp.wants_tools() {
                if !resp.content.trim().is_empty() {
                    empty_nudges = 0;
                    final_text = resp.content.clone();
                    has_prior_final = true;
                }
                // A response with neither text nor a tool call is a silent dead-end (model glitch,
                // narrate-then-stall, or a transient empty completion). Rather than just stopping,
                // nudge it to continue a bounded number of times — this recovers the common case
                // where the model would have made progress on a retry.
                if resp.content.trim().is_empty() && !bridge_tool_progress {
                    if completion_verification_empty_is_terminal(
                        verify_attempts,
                        &self.tasks,
                        has_prior_final,
                    ) {
                        self.presenter.emit(PresenterEvent::Warning(
                            "verification continuation returned no additional text — keeping the completed answer"
                                .to_string(),
                        ));
                        let accepted = final_text.clone();
                        self.publish_terminal_answer(&accepted)?;
                        hit_step_cap = false;
                        break;
                    }
                    const MAX_EMPTY_NUDGES: usize = 2;
                    if empty_nudges < MAX_EMPTY_NUDGES {
                        empty_nudges += 1;
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "model returned an empty response — nudging it to continue ({empty_nudges}/{MAX_EMPTY_NUDGES})"
                        )));
                        let nudge = "Your last response was empty. Continue with the task: call a \
                                     tool to make progress, or state your final answer. Do not reply \
                                     with an empty message.";
                        let nseq = self.next_seq();
                        // Provider chat APIs require a continuation request to end in a user
                        // message. Appending this as System after the empty Assistant response made
                        // the recovery request invalid (`last message role must be 'user'`).
                        let _ = self
                            .store
                            .add_message(&self.id, nseq, Role::User, nudge, None);
                        self.transcript.push(Message::user(nudge));
                        continue;
                    }
                    // Nudges exhausted. An empty-responding model (e.g. some NIM models that stream
                    // an empty final chunk, like kimi-k2.6 in the dogfooding run) is broken for this
                    // turn — BENCH it and FAIL OVER to the next chain model instead of dead-ending
                    // the turn short of a working model (the subscription bridge sat untried below).
                    if failover_enabled {
                        let _ = self.store.bench_for(
                            &active_model,
                            default_cooldown,
                            "empty response (no text, no tool call)",
                        );
                        let mut picked = None;
                        for next in chain.by_ref() {
                            match self.admit_failover_model(&next).await {
                                Ok(true) => {
                                    picked = Some(next);
                                    break;
                                }
                                Ok(false) => {}
                                Err(e) => return Err(e),
                            }
                        }
                        if let Some(next) = picked {
                            self.presenter.emit(PresenterEvent::Routing {
                                tier: decision
                                    .map(|d| d.tier.as_str().to_string())
                                    .unwrap_or_default(),
                                model: next.clone(),
                                rationale: format!("failover from {active_model} (empty response)"),
                            });
                            active_model = next;
                            transient_retries = 0;
                            empty_nudges = 0;
                            continue;
                        }
                    }
                    self.presenter.emit(PresenterEvent::Error(
                        "model returned an empty response (no text, no tool call) — stopping the turn"
                            .to_string(),
                    ));
                } else if completion_promises_followup(&resp.content) && followup_intent_nudges == 0
                {
                    followup_intent_nudges += 1;
                    self.presenter.emit(PresenterEvent::Warning(
                        "model promised another action but stopped before doing it — continuing once"
                            .to_string(),
                    ));
                    const FOLLOWUP_INTENT_NUDGE: &str = "Your last response explicitly promised a \
                        next action but ended before doing it. Do not narrate future work and stop. \
                        Call the required tool now and complete/check the action, or, if nothing \
                        remains, give a self-contained final answer stating exactly what already \
                        passed.";
                    let nseq = self.next_seq();
                    let _ = self.store.add_message(
                        &self.id,
                        nseq,
                        Role::System,
                        FOLLOWUP_INTENT_NUDGE,
                        None,
                    );
                    self.transcript.push(Message::system(FOLLOWUP_INTENT_NUDGE));
                    continue;
                } else if forge_provider::is_cli_bridge(&active_model) {
                    // Bridge cost ceiling (wave 5, fixes 1 + 2). This is the observation boundary a
                    // bridge turn actually has: its tools ran inside the subprocess and it has now
                    // yielded, so the direct-path cost guards (which key on `resp.tool_calls`) never
                    // saw any of it. Two backstops decide whether to keep re-driving:
                    //   * the accumulated input crossed the per-turn ceiling (fix 1), or
                    //   * the bridge kept re-issuing build/provision commands (fix 2 — the env/build
                    //     fight pattern the sink can see but can't attach pass/fail to).
                    // Either one ends the turn cleanly here — no further re-drive — submitting
                    // whatever verified diff exists. A tail-cost backstop, NOT a target; the common
                    // bridge turn finishes well under the cap and never trips this.
                    let over_budget = bridge_turn_over_budget(
                        bridge_input_accum,
                        self.config.mesh.bridge_turn_token_cap,
                    );
                    let build_fighting = bridge_build_fight
                        .load(std::sync::atomic::Ordering::Relaxed)
                        >= BRIDGE_BUILD_FIGHT_THRESHOLD;
                    if over_budget || build_fighting {
                        let why = if over_budget {
                            format!(
                                "bridge turn hit the {}M input-token ceiling",
                                self.config.mesh.bridge_turn_token_cap / 1_000_000
                            )
                        } else {
                            format!(
                                "bridge kept re-running build/provision commands \
                                 ({BRIDGE_BUILD_FIGHT_THRESHOLD}×)"
                            )
                        };
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "{why} — stopping the turn and submitting the current diff to cap \
                             tail cost"
                        )));
                        final_text = resp.content;
                        let accepted = final_text.clone();
                        self.publish_terminal_answer(&accepted)?;
                        hit_step_cap = false;
                        break;
                    }
                    // Loop-gated completeness (opt-in `mesh.verify_completeness`): the bridge yielded.
                    // Before accepting "done", fire ONE bounded final-diff review — the model worked
                    // the turn normally (no completeness pressure throughout, which is what tripled
                    // tokens in the always-on preamble form), and now does a single targeted re-check
                    // against every requirement. One-shot (`completeness_checked`) so it can't loop;
                    // gated on a turn that ran real tools (so there's an actual change to review).
                    if self.config.mesh.verify_completeness
                        && !completeness_checked
                        && inspect_ran.load(std::sync::atomic::Ordering::Relaxed) > 0
                    {
                        completeness_checked = true;
                        self.presenter.emit(PresenterEvent::Warning(
                            "completeness check — reviewing the change against every requirement before finishing"
                                .to_string(),
                        ));
                        const COMPLETENESS_NUDGE: &str = "Before finishing, do ONE final review (a \
                            single bounded pass — do NOT re-explore the codebase): run `git diff` once \
                            to see your COMPLETE change, re-read the original request and write the \
                            distinct requirements/cases it lists (issues routinely specify several, \
                            e.g. \"reject a dotted blueprint name AND a dotted endpoint\"), and for \
                            each confirm your diff already handles it. Only if the diff is MISSING a \
                            requirement, add that specific fix — otherwise finish. A change that \
                            handles only the first of several cases is INCOMPLETE.";
                        let nseq = self.next_seq();
                        let _ = self.store.add_message(
                            &self.id,
                            nseq,
                            Role::System,
                            COMPLETENESS_NUDGE,
                            None,
                        );
                        self.transcript.push(Message::system(COMPLETENESS_NUDGE));
                        continue;
                    }
                    // A CLI bridge is a ONE-SHOT subprocess: claude-cli/codex runs its own internal
                    // tool loop and EXITS, so forge can't keep a single invocation going. That let a
                    // long plan stop half-done — the bridge does a few steps (merge + tag), exits
                    // after launching the async release build, and the dependent steps (brew sha,
                    // verify) never run. Completion must be defined by the TASK LIST, not by the
                    // subprocess exiting: while tracked tasks remain unfinished, re-invoke the bridge
                    // with a continue instruction (a clean new process — exactly what the user typing
                    // `continue` does), so a turn can't "be done" while the work isn't.
                    //
                    // Anti-spiral (the guard the old bridge-nudge lacked): a re-run must make
                    // PROGRESS — start at least one tool OR close at least one task — or the turn
                    // HALTS loudly instead of re-driving. A bridge that just re-narrates without
                    // acting therefore cannot loop. Gated on a non-empty task list, so an ordinary
                    // bridge Q&A (no tracked tasks) stays terminal as before.
                    //
                    // Tasks live in the store (the bridge's `update_tasks` runs in the separate
                    // `mcp-serve` process), so reload before judging completion.
                    let persisted = match self.store.tasks(&self.id) {
                        Ok(tasks) => tasks,
                        Err(error) => {
                            tracing::warn!(session_id = %self.id, %error, "session task history could not be reloaded");
                            Vec::new()
                        }
                    };
                    if !persisted.is_empty() {
                        self.tasks = persisted;
                    }
                    let unfinished: Vec<String> = self
                        .tasks
                        .iter()
                        .filter(|t| !matches!(t.status, forge_types::TodoStatus::Done))
                        .map(|t| t.title.clone())
                        .collect();
                    let done_now = self.tasks.len().saturating_sub(unfinished.len());
                    let tools_this_turn =
                        tools_ran.load(std::sync::atomic::Ordering::Relaxed) - tools_before;
                    let made_progress = tools_this_turn > 0 || done_now > bridge_done_prev;
                    bridge_done_prev = done_now;
                    const MAX_BRIDGE_CONTINUE_NUDGES: usize = 8;
                    if !unfinished.is_empty() {
                        // Work is open again — any earlier "all done" verification is stale.
                        verify_attempts = 0;
                        if made_progress && bridge_continue_nudges < MAX_BRIDGE_CONTINUE_NUDGES {
                            bridge_continue_nudges += 1;
                            self.presenter.emit(PresenterEvent::Warning(format!(
                                "bridge yielded with {} task(s) unfinished — continuing the plan ({bridge_continue_nudges}/{MAX_BRIDGE_CONTINUE_NUDGES})",
                                unfinished.len()
                            )));
                            let nudge = format!(
                                "The plan is NOT finished — these tracked tasks are still open:\n- {}\n\n\
                                 Continue the plan now: carry out the next unfinished step and run it \
                                 to completion. If you launched an async job earlier (a release \
                                 build, CI), WAIT for it (poll it) and then do the steps that depend \
                                 on it — do not treat 'launched' as 'done'. Mark each task Done via \
                                 update_tasks as you finish it; if one is genuinely already complete \
                                 or impossible, mark it Done and say why. Do not stop until every \
                                 task is resolved.",
                                unfinished.join("\n- ")
                            );
                            let nseq = self.next_seq();
                            let _ =
                                self.store
                                    .add_message(&self.id, nseq, Role::System, &nudge, None);
                            self.transcript.push(Message::system(&nudge));
                            continue;
                        }
                        // No progress on the re-run (would spiral) or the re-drive budget is spent:
                        // stop LOUDLY with the work named, rather than silently reporting success.
                        let why = if made_progress {
                            "reached the continue limit"
                        } else {
                            "the last attempt made no progress (no task completed, no tool ran)"
                        };
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "bridge stopped with {} task(s) still unfinished — {why}. Send `continue` to resume.",
                            unfinished.len()
                        )));
                    } else if !self.tasks.is_empty() {
                        // The bridge reports every task Done — but a self-reported status is exactly
                        // what produced the phantom release (claimed merged + tagged; nothing ran).
                        // Require fresh tool-grounded evidence when work changed external state.
                        // A successful check newer than the last artifact mutation is accepted
                        // immediately; otherwise the gate requests a verification turn.
                        // A read-only completion is already evidenced by its inspection; a reasoned
                        // no-op is accepted without demanding a meaningless edit.
                        //   * If the turn did NO inspectable work (a pure reasoning/analysis plan —
                        //     the deliverable is the answer text, there is no external state to
                        //     check), requiring a tool inspection would over-fire. Accept with a
                        //     calm "not tool-verified" note instead.
                        // `did_real_work` is cumulative over the whole turn; `inspected_this_turn`
                        // is whether the turn just observed ran an inspection tool.
                        let did_real_work =
                            inspect_ran.load(std::sync::atomic::Ordering::Relaxed) > 0;
                        let (inspected_since_verify, unresolved_checks) = {
                            let ledger = verification_ledger.lock().unwrap();
                            (
                                ledger.verified_since(verification_at_last_verify),
                                ledger.unresolved_summary(),
                            )
                        };
                        if self.run_completion_gate(
                            &mut verify_attempts,
                            did_real_work,
                            completion_claims_no_change(&resp.content),
                            inspected_since_verify,
                            unresolved_checks.as_deref(),
                        ) == PostCheckDecision::RequestObservation
                        {
                            verification_at_last_verify =
                                verification_ledger.lock().unwrap().checkpoint();
                            continue;
                        }
                        // else: accepted (clean / no-artifacts / unverified) — fall through to terminal.
                    }
                } else {
                    // Honest-failure guard: a direct model wrote a tool call as TEXT (e.g.
                    // `<invoke>`/`default_api:` markup) instead of invoking it, and neither the
                    // provider nor the text-recovery pass turned it into a real call — so NOTHING
                    // ran. Accepting this as the final answer is how a turn "succeeds" while having
                    // merged no PR and pushed no tag. Detect it and nudge the model to actually
                    // call the tool (bounded); never silently accept narrated tool calls.
                    if forge_provider::looks_like_unexecuted_tool_call(&resp.content) {
                        const MAX_TOOLCALL_REPAIR_NUDGES: usize = 2;
                        if toolcall_repair_nudges < MAX_TOOLCALL_REPAIR_NUDGES {
                            toolcall_repair_nudges += 1;
                            self.presenter.emit(PresenterEvent::Warning(format!(
                                "model wrote a tool call as text instead of invoking it — nothing ran; asking it to retry ({toolcall_repair_nudges}/{MAX_TOOLCALL_REPAIR_NUDGES})"
                            )));
                            let nudge = "Your last message contained a tool call written as TEXT \
                                         (e.g. `<invoke …>` or `default_api:` syntax). That tool DID \
                                         NOT run — text is not a tool call. Make the call through the \
                                         function-calling interface instead. Do not paste tool-call \
                                         markup into your message.";
                            let nseq = self.next_seq();
                            let _ =
                                self.store
                                    .add_message(&self.id, nseq, Role::System, nudge, None);
                            self.transcript.push(Message::system(nudge));
                            continue;
                        }
                        // Retries exhausted: do NOT pretend it worked. Surface it loudly so the user
                        // knows the turn's actions never executed, then end (can't loop forever).
                        self.presenter.emit(PresenterEvent::Warning(
                            "model kept emitting tool calls as text that never executed — the turn did NOT complete its actions"
                                .to_string(),
                        ));
                    }
                    // Direct model, non-empty text, no tool call — usually the real final answer.
                    // But a weaker model often narrates its NEXT action ("now I'll edit X") without
                    // calling the tool, or signs off with tasks still open. If the tracked task list
                    // still has unfinished items, this is a premature stall: drive it onward
                    // (bounded) so the work completes instead of ending the turn mid-task.
                    let unfinished = self
                        .tasks
                        .iter()
                        .filter(|t| !matches!(t.status, forge_types::TodoStatus::Done))
                        .count();
                    const MAX_CONTINUE_NUDGES: usize = 4;
                    if unfinished > 0 {
                        // Work is still open — any earlier "all done" verification is stale.
                        verify_attempts = 0;
                        if continue_nudges < MAX_CONTINUE_NUDGES {
                            continue_nudges += 1;
                            self.presenter.emit(PresenterEvent::Warning(format!(
                                "model stopped with {unfinished} task(s) unfinished — continuing it ({continue_nudges}/{MAX_CONTINUE_NUDGES})"
                            )));
                            let nudge = "You ended your reply, but tasks on your list are NOT yet \
                                         Done. The turn is not over — do not stop. Continue now: call \
                                         the next tool to make progress on the remaining work. Only \
                                         finish once every task is resolved; if one is genuinely \
                                         complete or impossible, mark it Done via update_tasks and say \
                                         why. Do not reply again without either calling a tool or \
                                         marking a task Done.";
                            let nseq = self.next_seq();
                            let _ =
                                self.store
                                    .add_message(&self.id, nseq, Role::System, nudge, None);
                            self.transcript.push(Message::system(nudge));
                            continue;
                        }
                        // Nudge budget spent and work is STILL open — surface it. The bridge path
                        // emits an equivalent warning; the direct path used to fall through here
                        // silently, leaving the user to wonder why the turn stopped mid-plan.
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "model stopped with {unfinished} task(s) unfinished after \
                             {MAX_CONTINUE_NUDGES} continue nudge(s) — giving up. Send `continue` \
                             to resume."
                        )));
                    } else if !self.tasks.is_empty() {
                        // Every tracked task reported Done — same completion authority as the bridge:
                        // accept fresh evidence newer than the last mutation, otherwise request a
                        // tool-grounded state check. A self-reported "done" without an inspection is
                        // exactly the phantom-completion the bridge gate guards against.
                        let did_real_work =
                            inspect_ran.load(std::sync::atomic::Ordering::Relaxed) > 0;
                        // Unlike the bridge (which runs its whole tool loop INSIDE one `complete()`
                        // call, so an inspection lands in the same step as the final text), a direct
                        // model runs each tool in a SEPARATE step from the text "done" claim. So a
                        // step-local "did this step inspect?" is ALWAYS false at this gate, which would
                        // wrongly flag a genuinely-verified turn as UNVERIFIED. Instead ask: did an
                        // successful, outcome-aware evidence SINCE the last verification request.
                        // A generic read cannot clear an unresolved failed check family.
                        let (inspected_since_verify, unresolved_checks) = {
                            let ledger = verification_ledger.lock().unwrap();
                            (
                                ledger.verified_since(verification_at_last_verify),
                                ledger.unresolved_summary(),
                            )
                        };
                        if self.run_completion_gate(
                            &mut verify_attempts,
                            did_real_work,
                            completion_claims_no_change(&resp.content),
                            inspected_since_verify,
                            unresolved_checks.as_deref(),
                        ) == PostCheckDecision::RequestObservation
                        {
                            verification_at_last_verify =
                                verification_ledger.lock().unwrap().checkpoint();
                            continue;
                        }
                    }
                }
                final_text = resp.content;
                let accepted = final_text.clone();
                self.publish_terminal_answer(&accepted)?;
                hit_step_cap = false;
                break;
            }

            if self
                .execute_model_tool_step(
                    &msg_id,
                    &resp.tool_calls,
                    &tools_ran,
                    &inspect_ran,
                    &verification_ledger,
                    &mut last_tool_sig,
                    &mut repeat_count,
                    &mut recent_sigs,
                    &mut doom_nudged,
                    &mut failure_counts,
                    &mut failure_nudged,
                )
                .await?
            {
                halted_by_loop_guard = true;
                hit_step_cap = false;
                break;
            }
        }

        Ok(ModelLoopOutcome {
            final_text,
            context_tokens,
            hit_step_cap,
            halted_by_loop_guard,
            active_model,
            plan: proposed_plan,
            tools_ran: tools_ran.load(std::sync::atomic::Ordering::Relaxed),
            mcp_tools_unavailable: mcp_tools_unavailable.load(std::sync::atomic::Ordering::Relaxed),
        })
    }
}
