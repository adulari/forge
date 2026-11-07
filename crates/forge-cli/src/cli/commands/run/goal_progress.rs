//! How a `/goal` run decides its next step after a turn (docs/features/goal-evaluator.md).
//!
//! A finished goal turn is followed by one evaluator side call ([`Session::evaluate_goal`]),
//! spawned like a turn so the UI keeps ticking and Esc still stops it. When that call finishes,
//! [`next_goal_decision`] folds its verdict into the existing policy: an achieved goal still runs
//! the quality gates, an unfinished one gets a short prompt built from what the evaluator says is
//! missing, and a run whose verdicts stop moving is stopped as wedged. No verdict (evaluator off
//! or failed) means exactly the old behaviour: the fixed [`GOAL_CONTINUE_PROMPT`] and the
//! task-plan stall counter.
//!
//! Both drivers of `/goal` — the interactive TUI loop and the daemon-hosted session driver behind
//! `forge serve` (which also runs `forge run "/goal …"` published to the fleet) — go through
//! [`advance_goal`] and [`launch_goal_step`], so they cannot drift apart. The headless local
//! `forge run` (goal_headless.rs) has no `App` to drive, so it calls the same [`settle_goal`] the
//! two of them end in.

use std::path::Path;
use std::sync::{Arc, Mutex};

use forge_core::goal_contract::ContractOutcome;
use forge_core::{GoalEvalRequest, GoalVerdict};
use forge_types::goal_contract::{failure_prompt, AssertionResult, GoalContract};
use forge_types::goal_envelope::EnvelopeCheck;

use forge_core::goal_relay::{RelayPolicy, RelayReason};

use super::goal_envelope::{self, pause_message, EnvelopeSources};
use super::goal_relay::{self, RelayPlan};

use super::*;

/// Consecutive verdicts with an unchanged remaining list (or an explicit "no progress") before a
/// goal is stopped as wedged.
pub(crate) const GOAL_EVAL_WEDGED_TURNS: usize = 3;
/// Times the agent may claim completion against the evaluator's "not achieved" before the run
/// stops instead of looping on the disagreement.
pub(crate) const GOAL_DISAGREE_MAX: usize = 3;
/// An "achieved" verdict below this confidence counts only when the agent also claims completion.
pub(crate) const GOAL_EVAL_MIN_CONFIDENCE: f64 = 0.5;
const PROMPT_ITEMS: usize = 5;

/// The first turn of a goal: decompose it into a tracked plan, then start.
pub(crate) fn goal_start_prompt(goal: &str) -> String {
    format!(
        "Break this goal into a concrete, ordered plan and record it with the \
         update_tasks tool, then start on the first step.\n\nGoal: {goal}"
    )
}

/// The guidance primed on the session once a goal is set.
pub(crate) fn goal_session_guidance(goal: &str) -> String {
    format!("Session goal: {goal}\nKeep every step aligned to this goal until it is fully met.")
}

/// Evaluator bookkeeping that lives for one `/goal` run.
#[derive(Debug, Default)]
pub(crate) struct GoalEvalTrack {
    /// The in-flight generation is the evaluator call for the turn before it, not a turn.
    pub(crate) evaluating: bool,
    /// Tokens of the turn being evaluated, charged to the budget when the decision is made.
    pub(crate) pending_tokens: u64,
    /// Where the evaluator task leaves its verdict for the done-signal handler.
    pub(crate) slot: Arc<Mutex<Option<GoalVerdict>>>,
    pub(crate) last: Option<GoalVerdict>,
    pub(crate) unchanged: usize,
    pub(crate) disagreements: usize,
    /// The last quality-gate failure (`cmd`: output), shown to the evaluator and the next prompt.
    pub(crate) last_gate_failure: Option<String>,
}

/// Validator bookkeeping that lives for one `/goal --contract` run.
#[derive(Debug, Default)]
pub(crate) struct ValidationTrack {
    /// Where the validator task leaves its results for the done-signal handler.
    pub(crate) slot: Arc<Mutex<Option<Vec<AssertionResult>>>>,
    /// A validator run finished since the last decision.
    pub(crate) fresh: bool,
    /// The failing assertion numbers of the previous run, and how many runs in a row repeated them.
    pub(crate) last_failing: Vec<u32>,
    pub(crate) unchanged: usize,
}

fn normalized(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = items
        .iter()
        .map(|item| {
            item.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    out.sort();
    out
}

impl GoalEvalTrack {
    /// Record a verdict; `true` once the goal has been stuck for [`GOAL_EVAL_WEDGED_TURNS`].
    pub(crate) fn observe(&mut self, verdict: &GoalVerdict) -> bool {
        let same = self
            .last
            .as_ref()
            .is_some_and(|prev| normalized(&prev.remaining) == normalized(&verdict.remaining));
        let stuck = !verdict.achieved && (same || !verdict.progressed);
        self.unchanged = if stuck { self.unchanged + 1 } else { 0 };
        self.last = Some(verdict.clone());
        self.unchanged >= GOAL_EVAL_WEDGED_TURNS
    }
}

impl GoalState {
    pub(crate) fn eval_request(&self) -> GoalEvalRequest {
        GoalEvalRequest {
            goal: self.goal.clone(),
            started_unix: self.started_unix,
            iteration: self.iter,
            previous_remaining: self
                .eval
                .last
                .as_ref()
                .map(|v| v.remaining.clone())
                .unwrap_or_default(),
            last_gate_failure: self.eval.last_gate_failure.clone(),
        }
    }
}

/// The re-drive prompt built from a verdict: what is missing, the next step, any gate failure.
pub(crate) fn verdict_prompt(
    goal: &str,
    verdict: &GoalVerdict,
    claimed_done: bool,
    gate_failure: Option<&str>,
) -> String {
    let mut out = format!("Goal: {goal}\n");
    if claimed_done {
        out.push_str("You reported the goal complete, but the goal evaluator says it is not");
        if !verdict.reason.is_empty() {
            out.push_str(&format!(": {}", verdict.reason));
        }
        out.push('\n');
    } else if !verdict.reason.is_empty() {
        out.push_str(&format!("Goal evaluator: {}\n", verdict.reason));
    }
    if !verdict.remaining.is_empty() {
        out.push_str("Still missing:\n");
        for item in verdict.remaining.iter().take(PROMPT_ITEMS) {
            out.push_str(&format!("- {item}\n"));
        }
    }
    let next = if verdict.next_step.is_empty() {
        "confirm every part of the goal is met, with evidence"
    } else {
        verdict.next_step.as_str()
    };
    out.push_str(&format!("Next step: {next}\n"));
    if let Some(gate) = gate_failure.and_then(|g| g.lines().next()) {
        out.push_str(&format!("Last quality gate failure: {gate}\n"));
    }
    out.push_str(
        "Do that step end to end, verify it, commit/push finished work, and keep update_tasks \
         current.",
    );
    out
}

/// Like [`gate_failure_prompt`] but keeps the goal text in view — a `/goal` re-drive turn only
/// sees a generated prompt otherwise, so a gate-failure detour must restate what it's working
/// toward.
fn goal_gate_failure_prompt(goal: &str, cmd: &str, bounded_output: &str) -> String {
    format!(
        "Goal: {goal}\n\n{}",
        gate_failure_prompt(cmd, bounded_output)
    )
}

fn goal_budget_check(state: &GoalState) -> Option<GoalDecision> {
    goal_budget_stop_reason(
        state.budget.tokens_used,
        state.budget.max_tokens,
        state.budget.started_at.elapsed(),
        state.budget.max_elapsed,
    )
    .map(|reason| GoalDecision::Stop(reason.to_string()))
}

/// A completion (claimed or judged): the quality gates decide whether the run really ends.
async fn finish_with_gates(state: &mut GoalState, reason: &str, cwd: &Path) -> GoalDecision {
    let fingerprint = workspace_fingerprint(cwd).await;
    match run_gates(&mut state.gates, &state.gate_cfg, &fingerprint, cwd).await {
        GateOutcome::AllPassed => {
            state.eval.last_gate_failure = None;
            GoalDecision::Stop(reason.to_string())
        }
        GateOutcome::Failed {
            gate_index,
            bounded_output,
        } => {
            let cmd = state.gates[gate_index].spec.cmd.clone();
            state.eval.last_gate_failure = Some(format!("`{cmd}` failed: {bounded_output}"));
            goal_budget_check(state).unwrap_or_else(|| GoalDecision::GateFailed {
                prompt: goal_gate_failure_prompt(&state.goal, &cmd, &bounded_output),
            })
        }
        GateOutcome::Exhausted { gate_index } => GoalDecision::Stop(format!(
            "🎯 goal stopped — quality gate exhausted: {}",
            state.gates[gate_index].spec.cmd
        )),
    }
}

/// Decide the next action for a finished `/goal` turn. With a `verdict`, the evaluator decides
/// completion (gates still run), drives the prompt, and detects a wedged run; without one, the
/// task-plan/stall policy ([`goal_stop_reason`]) applies unchanged. Gates and budgets apply
/// either way. Mutates `state` in place: counters, gate attempts and token usage persist across
/// the whole run.
pub(crate) async fn next_goal_decision(
    state: &mut GoalState,
    said_complete: bool,
    done: usize,
    total: usize,
    tokens_this_turn: u64,
    verdict: Option<GoalVerdict>,
    cwd: &Path,
) -> GoalDecision {
    state.budget.tokens_used = state.budget.tokens_used.saturating_add(tokens_this_turn);
    let progressed = done > state.prev_done;
    state.prev_done = done;
    if state.contract.is_some() {
        return contract_decision(state, cwd).await;
    }
    let Some(verdict) = verdict else {
        state.no_progress = if progressed { 0 } else { state.no_progress + 1 };
        return match goal_stop_reason(said_complete, done, total, state.iter, state.no_progress) {
            Some(reason) if is_goal_complete_reason(reason) => {
                finish_with_gates(state, reason, cwd).await
            }
            Some(reason) => GoalDecision::Stop(reason.to_string()),
            None => goal_budget_check(state).unwrap_or(GoalDecision::Retry { prompt: None }),
        };
    };
    // The evaluator now owns stall detection; the task-plan counter only restarts on progress so
    // a later fallback turn does not inherit a stale streak.
    if progressed {
        state.no_progress = 0;
    }
    let claimed_done = said_complete || (total > 0 && done == total);
    let wedged = state.eval.observe(&verdict);
    if verdict.achieved && (verdict.confidence >= GOAL_EVAL_MIN_CONFIDENCE || claimed_done) {
        return finish_with_gates(state, GOAL_COMPLETE_REASON, cwd).await;
    }
    if state.iter >= GOAL_MAX_ITERS {
        return GoalDecision::Stop("🎯 goal stopped — iteration ceiling".to_string());
    }
    if claimed_done {
        state.eval.disagreements += 1;
        if state.eval.disagreements > GOAL_DISAGREE_MAX {
            return GoalDecision::Stop(format!(
                "🎯 goal stopped — the agent reports done but the evaluator disagrees: {}",
                verdict.reason
            ));
        }
    }
    if wedged {
        return GoalDecision::Stop(format!(
            "🎯 goal wedged — no progress across {GOAL_EVAL_WEDGED_TURNS} evaluations: {}",
            verdict.reason
        ));
    }
    goal_budget_check(state).unwrap_or_else(|| GoalDecision::Retry {
        prompt: Some(verdict_prompt(
            &state.goal,
            &verdict,
            claimed_done,
            state.eval.last_gate_failure.as_deref(),
        )),
    })
}

/// The decision for a goal with a contract: the validator's results are the only judge of "done".
/// An all-pass run still goes through the quality gates; a failing one becomes the next prompt;
/// the same failing set three runs in a row stops the goal as wedged. A turn the validator did not
/// get to report on (interrupted) re-runs it next time rather than falling back to the task plan
/// or the model's own claim.
async fn contract_decision(state: &mut GoalState, cwd: &Path) -> GoalDecision {
    let fresh = std::mem::take(&mut state.validation.fresh);
    let Some(contract) = state.contract.clone() else {
        return GoalDecision::Retry { prompt: None };
    };
    if fresh && contract.all_passed() {
        return finish_with_gates(state, GOAL_COMPLETE_REASON, cwd).await;
    }
    if state.iter >= GOAL_MAX_ITERS {
        return GoalDecision::Stop("🎯 goal stopped — iteration ceiling".to_string());
    }
    let prompt = if fresh {
        let failing = contract.failing_ids();
        state.validation.unchanged = if failing == state.validation.last_failing {
            state.validation.unchanged + 1
        } else {
            0
        };
        state.validation.last_failing = failing.clone();
        if state.validation.unchanged + 1 >= GOAL_EVAL_WEDGED_TURNS {
            let ids: Vec<String> = failing.iter().map(u32::to_string).collect();
            return GoalDecision::Stop(format!(
                "🎯 goal wedged — assertions {} still fail after {GOAL_EVAL_WEDGED_TURNS} validations",
                ids.join(", ")
            ));
        }
        failure_prompt(&state.goal, &contract)
    } else {
        GOAL_CONTINUE_PROMPT.to_string()
    };
    goal_budget_check(state).unwrap_or(GoalDecision::Retry {
        prompt: Some(prompt),
    })
}

/// What the contract flow left after the first turn of a `--contract` goal: adopt an approved
/// contract, or end the goal that must not start. `None` when there is nothing to settle.
pub(crate) fn adopt_contract_outcome(
    gs: &mut GoalState,
    outcome: Option<ContractOutcome>,
) -> Option<Result<GoalContract, String>> {
    match outcome? {
        ContractOutcome::Approved(contract) => {
            gs.contract = Some(contract.clone());
            Some(Ok(contract))
        }
        ContractOutcome::Declined => Some(Err(
            "🎯 goal not started — the contract was declined".to_string()
        )),
        ContractOutcome::Failed(why) => Some(Err(format!(
            "🎯 goal not started — no contract could be drafted: {why}"
        ))),
    }
}

/// Fold a finished validator run into the goal's contract.
pub(crate) fn record_validation(gs: &mut GoalState) {
    let Some(results) = gs.validation.slot.lock().unwrap().take() else {
        return;
    };
    if let Some(contract) = gs.contract.as_mut() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        contract.record(results, now);
        gs.validation.fresh = true;
    }
}

/// What a goal run does after a done signal.
pub(crate) enum GoalNext {
    /// The run ended; any non-success reason was already noted.
    Stop,
    /// Run the evaluator on the turn that just finished.
    Evaluate(GoalEvalRequest),
    /// Run the contract validator on the turn that just finished.
    Validate(GoalContract),
    /// Run this prompt next (a gate failure — not displaced by a queued prompt).
    Turn(String),
    /// Run this prompt next unless the user queued a correction, which takes the turn instead.
    Continue(String),
    /// The goal moved to a fresh worker session: the driver brings it up and ends this run.
    Relay(RelayPlan),
}

/// What a finished turn (and its evaluation) looks like to the goal policy, read off the session.
pub(crate) struct GoalTurnFacts {
    done: usize,
    total: usize,
    said_complete: bool,
    /// The turn was refused by a spend cap: re-driving it can only be refused again.
    budget_refused: bool,
    /// The run's own `--max-tokens`/`--max-minutes` ended the turn mid-flight.
    run_budget_hit: bool,
    /// Gates check the checkout the session is working in, which a `worktree enter` moves away
    /// from the directory Forge was launched in.
    root: std::path::PathBuf,
    /// What the spend envelope is measured against: subscription readings and session cost.
    sources: EnvelopeSources,
    /// The fresh-worker relay settings; `None` where a run cannot hand its goal on (the headless
    /// local run has no driver to start the successor).
    relay: Option<RelayPolicy>,
}

impl GoalTurnFacts {
    pub(crate) fn read(s: &Session, cwd: &Path) -> Self {
        let tasks = s.tasks();
        let root = s.workspace_root();
        Self {
            done: tasks
                .iter()
                .filter(|t| t.status == forge_types::TodoStatus::Done)
                .count(),
            total: tasks.len(),
            said_complete: is_goal_complete_marker(s.last_assistant_text()),
            budget_refused: s.budget_refused_last_turn(),
            run_budget_hit: s.run_budget_hit_last_turn(),
            root: if root.as_os_str().is_empty() {
                cwd.to_path_buf()
            } else {
                root.to_path_buf()
            },
            sources: EnvelopeSources::read(s),
            relay: Some(s.relay_policy()),
        }
    }

    /// For a run that has no driver to start a successor session.
    pub(crate) fn without_relay(mut self) -> Self {
        self.relay = None;
        self
    }
}

/// The decision after a goal turn has been evaluated (or the evaluator is off).
pub(crate) enum GoalStep {
    /// The run ended; `met` is false for every reason but a genuine completion.
    Stop {
        reason: String,
        met: bool,
    },
    Turn(String),
    Continue(String),
    /// The spend envelope is spent (or cannot be verified): the goal pauses, resumable, instead
    /// of starting another turn.
    Pause {
        reason: String,
    },
    /// The goal wedged, stalled or wore its session out: it continues in a fresh worker session.
    Relay {
        reason: RelayReason,
        detail: String,
    },
}

/// Fold the evaluation left in `gs.eval.slot` (if any) into the next step. Shared by the
/// interactive/daemon drivers and the headless local run, so the stop conditions cannot drift.
pub(crate) async fn settle_goal(gs: &mut GoalState, facts: GoalTurnFacts) -> GoalStep {
    gs.eval.evaluating = false;
    record_validation(gs);
    let verdict = gs.eval.slot.lock().unwrap().take();
    let tokens = std::mem::take(&mut gs.eval.pending_tokens);
    if facts.budget_refused {
        return GoalStep::Stop {
            reason: "🎯 goal stopped — budget cap reached".to_string(),
            met: false,
        };
    }
    let envelope_check = gs.envelope.as_mut().map(|envelope| {
        let mut check = facts.sources.check(envelope);
        if facts.run_budget_hit && envelope.is_usd() && check == EnvelopeCheck::Within {
            envelope.spent = true;
            check = EnvelopeCheck::Spent;
        }
        check
    });
    gs.sync_envelope_budget();
    let envelope_hit = matches!(
        envelope_check,
        Some(EnvelopeCheck::Spent | EnvelopeCheck::Unverified)
    );
    if facts.run_budget_hit && !envelope_hit {
        let reason = if gs
            .budget
            .max_elapsed
            .is_some_and(|max| gs.budget.started_at.elapsed() >= max)
        {
            "🎯 goal stopped — time budget exhausted"
        } else {
            "🎯 goal stopped — token budget exhausted"
        };
        return GoalStep::Stop {
            reason: reason.to_string(),
            met: false,
        };
    }
    let decision = next_goal_decision(
        gs,
        facts.said_complete,
        facts.done,
        facts.total,
        tokens,
        verdict,
        &facts.root,
    )
    .await;
    if let (true, Some(envelope), Some(check)) =
        (envelope_hit, gs.envelope.as_ref(), envelope_check)
    {
        if !matches!(decision, GoalDecision::Stop(_)) {
            return GoalStep::Pause {
                reason: pause_message(envelope, check),
            };
        }
    }
    if let (Some(policy), false) = (facts.relay.filter(RelayPolicy::allowed), envelope_hit) {
        match &decision {
            GoalDecision::Stop(stop) => {
                if let Some(reason) = RelayReason::of_stop(stop) {
                    return GoalStep::Relay {
                        reason,
                        detail: goal_relay::stop_detail(stop),
                    };
                }
            }
            GoalDecision::GateFailed { .. } | GoalDecision::Retry { .. } if policy.worn() => {
                return GoalStep::Relay {
                    reason: RelayReason::Worn,
                    detail: format!("compacted {} times", policy.compactions),
                };
            }
            _ => {}
        }
    }
    match decision {
        GoalDecision::Stop(reason) => GoalStep::Stop {
            met: is_goal_complete_reason(&reason),
            reason,
        },
        GoalDecision::GateFailed { prompt } => GoalStep::Turn(prompt),
        GoalDecision::Retry { prompt } => {
            GoalStep::Continue(prompt.unwrap_or_else(|| GOAL_CONTINUE_PROMPT.to_string()))
        }
    }
}

/// Why a goal pauses when the provider stayed down for the whole outage allowance.
const PROVIDER_DOWN_PAUSE: &str =
    "goal paused — the provider stayed down; /goal resume continues it once it is back";

/// Pause the goal, resumably, when the turn that just ended gave up on a down provider. Nothing
/// about the goal went wrong and the turn did no work, so it must not be counted as a turn
/// without progress (six of those stop the goal as stalled). True when it paused.
pub(crate) fn pause_if_provider_down(app: &mut forge_tui::App) -> bool {
    if app.last_stop_reason != Some(forge_types::StopReason::ProviderDown) {
        return false;
    }
    app.note(&format!("⏸ {PROVIDER_DOWN_PAUSE}"));
    app.goal_paused_by_envelope(PROVIDER_DOWN_PAUSE);
    true
}

/// Advance a goal run whose in-flight generation just finished: after a turn, evaluate it (when
/// the evaluator is on); after the evaluation (or with it off), decide.
pub(crate) async fn advance_goal(
    gs: &mut GoalState,
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    cwd: &Path,
) -> GoalNext {
    if let Some(stop) = apply_goal_requests(gs, session, app).await {
        return stop;
    }
    if !gs.eval.evaluating && pause_if_provider_down(app) {
        session.lock().await.set_active_goal(None);
        return GoalNext::Stop;
    }
    if !gs.eval.evaluating && gs.contract.is_none() {
        let outcome = session.lock().await.take_goal_contract_outcome();
        match adopt_contract_outcome(gs, outcome) {
            Some(Ok(contract)) => app.goal_set_contract(contract),
            Some(Err(reason)) => {
                app.note(&reason);
                app.goal_ended(false, &reason);
                session.lock().await.set_active_goal(None);
                return GoalNext::Stop;
            }
            None => {}
        }
    }
    if !gs.eval.evaluating {
        gs.eval.pending_tokens = app.turn_in + app.turn_out;
        if let Some(contract) = gs.contract.clone() {
            if !session.lock().await.run_budget_hit_last_turn() {
                gs.eval.evaluating = true;
                gs.validation.slot.lock().unwrap().take();
                return GoalNext::Validate(contract);
            }
        }
        let s = session.lock().await;
        if s.goal_evaluator_enabled() && !s.run_budget_hit_last_turn() {
            drop(s);
            gs.eval.evaluating = true;
            gs.eval.slot.lock().unwrap().take();
            return GoalNext::Evaluate(gs.eval_request());
        }
    }
    if let Some(envelope) = &gs.envelope {
        goal_envelope::refresh_quota(session, &envelope.spec).await;
    }
    let facts = GoalTurnFacts::read(&*session.lock().await, cwd);
    let step = settle_goal(gs, facts).await;
    if let Some(contract) = &gs.contract {
        app.goal_set_contract(contract.clone());
    }
    if let Some(envelope) = &gs.envelope {
        app.goal_envelope_sync(envelope);
    }
    match step {
        GoalStep::Pause { reason } => {
            app.note(&format!("⏸ {reason}"));
            app.goal_paused_by_envelope(&reason);
            if let Some(envelope) = app.goal.as_mut().and_then(|g| g.envelope.as_mut()) {
                goal_envelope::mark_waiting(envelope, chrono::Utc::now().timestamp());
            }
            session.lock().await.set_active_goal(None);
            GoalNext::Stop
        }
        GoalStep::Stop { reason, met } => {
            if !met {
                app.note(&reason);
            }
            app.goal_ended(met, &reason);
            session.lock().await.set_active_goal(None);
            GoalNext::Stop
        }
        GoalStep::Relay { reason, detail } => {
            match goal_relay::perform(gs, session, app, reason, &detail).await {
                Ok(plan) => GoalNext::Relay(plan),
                Err(why) => {
                    let stop =
                        format!("🎯 goal stopped — could not hand it to a fresh worker: {why}");
                    app.note(&stop);
                    app.goal_ended(false, &stop);
                    session.lock().await.set_active_goal(None);
                    GoalNext::Stop
                }
            }
        }
        GoalStep::Turn(prompt) => GoalNext::Turn(prompt),
        GoalStep::Continue(prompt) => GoalNext::Continue(prompt),
    }
}

/// A goal step that was started: its task, and the prompt it runs (`None` for an evaluation).
pub(crate) struct LaunchedGoalStep {
    pub(crate) handle: tokio::task::JoinHandle<()>,
    pub(crate) prompt: Option<String>,
}

/// Start what [`advance_goal`] asked for under generation `gen`. `steer` yields a queued user
/// prompt, which replaces a [`GoalNext::Continue`] prompt. The caller owns the generation
/// counter: on `Some` it must adopt `gen` and keep the goal state; on `None` the run is over.
#[allow(clippy::too_many_arguments)]
pub(crate) fn launch_goal_step(
    next: GoalNext,
    gs: &mut GoalState,
    steer: impl FnOnce(&mut forge_tui::App) -> Option<String>,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> Option<LaunchedGoalStep> {
    gs.gen = gen;
    let prompt = match next {
        GoalNext::Stop | GoalNext::Relay(_) => return None,
        GoalNext::Evaluate(request) => {
            app.done = false;
            app.tick = 0;
            *busy = true;
            *busy_since = std::time::Instant::now();
            let s = session.clone();
            let dt = done_tx.clone();
            let slot = Arc::clone(&gs.eval.slot);
            let handle = tokio::spawn(async move {
                let _done = DoneGuard(dt, gen);
                let verdict = s.lock().await.evaluate_goal(&request).await;
                *slot.lock().unwrap() = verdict;
            });
            return Some(LaunchedGoalStep {
                handle,
                prompt: None,
            });
        }
        GoalNext::Validate(contract) => {
            app.done = false;
            app.tick = 0;
            *busy = true;
            *busy_since = std::time::Instant::now();
            let s = session.clone();
            let dt = done_tx.clone();
            let slot = Arc::clone(&gs.validation.slot);
            let handle = tokio::spawn(async move {
                let _done = DoneGuard(dt, gen);
                let results = s.lock().await.validate_goal_contract(&contract).await;
                *slot.lock().unwrap() = Some(results);
            });
            return Some(LaunchedGoalStep {
                handle,
                prompt: None,
            });
        }
        GoalNext::Turn(prompt) => prompt,
        GoalNext::Continue(prompt) => steer(app).unwrap_or(prompt),
    };
    gs.iter += 1;
    app.goal_sync(
        gs.iter,
        gs.budget.tokens_used,
        gs.budget.started_at.elapsed().as_secs(),
    );
    arm_run_budget(session, Some(&gs.budget));
    let handle = spawn_turn_with(
        prompt.clone(),
        vec![GOAL_GUIDANCE.to_string()],
        Some(forge_types::TaskTier::Complex),
        session,
        done_tx,
        gen,
        app,
        busy,
        busy_since,
    );
    Some(LaunchedGoalStep {
        handle,
        prompt: Some(prompt),
    })
}

#[cfg(test)]
#[path = "goal_progress_tests.rs"]
mod tests;
