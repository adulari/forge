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
//! [`advance_goal`] and [`launch_goal_step`], so they cannot drift apart.

use std::path::Path;
use std::sync::{Arc, Mutex};

use forge_core::{GoalEvalRequest, GoalVerdict};

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
    fn eval_request(&self) -> GoalEvalRequest {
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

/// What a goal run does after a done signal.
pub(crate) enum GoalNext {
    /// The run ended; any non-success reason was already noted.
    Stop,
    /// Run the evaluator on the turn that just finished.
    Evaluate(GoalEvalRequest),
    /// Run this prompt next (a gate failure — not displaced by a queued prompt).
    Turn(String),
    /// Run this prompt next unless the user queued a correction, which takes the turn instead.
    Continue(String),
}

/// Advance a goal run whose in-flight generation just finished: after a turn, evaluate it (when
/// the evaluator is on); after the evaluation (or with it off), decide.
pub(crate) async fn advance_goal(
    gs: &mut GoalState,
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    cwd: &Path,
) -> GoalNext {
    if !gs.eval.evaluating {
        gs.eval.pending_tokens = app.turn_in + app.turn_out;
        if session.lock().await.goal_evaluator_enabled() {
            gs.eval.evaluating = true;
            gs.eval.slot.lock().unwrap().take();
            return GoalNext::Evaluate(gs.eval_request());
        }
    }
    gs.eval.evaluating = false;
    let verdict = gs.eval.slot.lock().unwrap().take();
    let (done, total, said_complete) = {
        let s = session.lock().await;
        let tasks = s.tasks();
        (
            tasks
                .iter()
                .filter(|t| t.status == forge_types::TodoStatus::Done)
                .count(),
            tasks.len(),
            is_goal_complete_marker(s.last_assistant_text()),
        )
    };
    let tokens = std::mem::take(&mut gs.eval.pending_tokens);
    // Gates check the checkout the session is working in, which a `worktree enter` moves away
    // from the directory Forge was launched in.
    let root = {
        let s = session.lock().await;
        let root = s.workspace_root();
        if root.as_os_str().is_empty() {
            cwd.to_path_buf()
        } else {
            root.to_path_buf()
        }
    };
    match next_goal_decision(gs, said_complete, done, total, tokens, verdict, &root).await {
        GoalDecision::Stop(reason) => {
            if !is_goal_complete_reason(&reason) {
                app.note(&reason);
            }
            GoalNext::Stop
        }
        GoalDecision::GateFailed { prompt } => GoalNext::Turn(prompt),
        GoalDecision::Retry { prompt } => {
            GoalNext::Continue(prompt.unwrap_or_else(|| GOAL_CONTINUE_PROMPT.to_string()))
        }
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
        GoalNext::Stop => return None,
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
        GoalNext::Turn(prompt) => prompt,
        GoalNext::Continue(prompt) => steer(app).unwrap_or(prompt),
    };
    gs.iter += 1;
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
