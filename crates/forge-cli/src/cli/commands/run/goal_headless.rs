//! `/goal` for a local, headless `forge run` (no daemon, no TUI): the same turn -> evaluate ->
//! decide loop the daemon-hosted driver runs, minus the `App` it renders into. The decision
//! itself is [`settle_goal`], shared with the interactive and daemon drivers, so max iterations,
//! budgets, the evaluator, wedge detection and quality gates behave identically
//! (docs/features/goal-evaluator.md).

use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result};
use forge_core::Session;

use super::*;

/// A `/goal <objective> [--gate ...] [--max-tokens N] [--max-minutes N]` one-shot prompt.
pub(crate) struct GoalRunSpec {
    pub(crate) goal: String,
    pub(crate) gates: Vec<String>,
    pub(crate) max_tokens: Option<u64>,
    pub(crate) max_minutes: Option<u64>,
    pub(crate) budget: Option<String>,
    pub(crate) contract: bool,
}

/// `Some` when `prompt` is a `/goal …` command, `None` for any other prompt.
pub(crate) fn parse_goal_prompt(prompt: &str) -> Option<GoalRunSpec> {
    let trimmed = prompt.trim_start();
    let name_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    if &trimmed[..name_end] != "/goal" {
        return None;
    }
    match forge_tui::parse_command(trimmed) {
        forge_tui::CommandAction::Goal {
            objective,
            gates,
            max_tokens,
            max_minutes,
            budget,
            contract,
        } => Some(GoalRunSpec {
            goal: objective.trim().to_string(),
            gates,
            max_tokens,
            max_minutes,
            budget,
            contract,
        }),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GoalRunEnd {
    Met,
    NotMet(String),
}

impl GoalRunEnd {
    pub(crate) fn into_result(self) -> Result<()> {
        match self {
            GoalRunEnd::Met => Ok(()),
            GoalRunEnd::NotMet(reason) => anyhow::bail!("goal not met: {reason}"),
        }
    }
}

fn total_tokens(session: &Session) -> u64 {
    let usage = session.session_usage_detail();
    usage.input_tokens.saturating_add(usage.output_tokens)
}

/// Drive `spec` to completion on `session`. `interrupted` resolving (Ctrl-C) stops the run after
/// dropping the in-flight turn or evaluation, keeping whatever already streamed.
pub(crate) async fn run_goal_headless(
    session: &mut Session,
    spec: GoalRunSpec,
    guidance: &[String],
    cwd: &Path,
    interrupted: impl Future<Output = ()>,
) -> Result<GoalRunEnd> {
    if spec.goal.is_empty() {
        anyhow::bail!(
            "usage: forge run \"/goal <objective> [--gate \\\"<cmd>\\\"]... [--max-tokens N] \
             [--max-minutes N]\""
        );
    }
    if spec.budget.is_some() {
        return Ok(GoalRunEnd::NotMet(
            "--budget needs an interactive or daemon-hosted goal; use --max-tokens or \
             --max-minutes for a headless run (nothing was started)"
                .to_string(),
        ));
    }
    session
        .prime_guidance(&[goal_session_guidance(&spec.goal)])
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    session.set_active_goal(Some(&spec.goal));
    if spec.contract || session.goal_contract_default() {
        session.request_goal_contract(&spec.goal);
    }
    let mut prompt = goal_start_prompt(&spec.goal);
    let mut gs = GoalState::new(0, spec.goal, spec.gates, spec.max_tokens, spec.max_minutes);
    let mut turn_guidance = guidance.to_vec();
    turn_guidance.push(GOAL_GUIDANCE.to_string());
    tokio::pin!(interrupted);
    loop {
        let before = total_tokens(session);
        session.set_run_budget(gs.budget.remaining());
        let turn = session.run_turn_with(
            &prompt,
            &turn_guidance,
            Some(forge_types::TaskTier::Complex),
        );
        let outcome = tokio::select! {
            r = turn => match r {
                Ok(outcome) => outcome,
                // The provider stayed down for the whole outage allowance: the goal is paused,
                // not failed, and a re-run picks it up once the provider is back.
                Err(forge_core::CoreError::ProviderDown { .. }) => {
                    return Ok(GoalRunEnd::NotMet(
                        "⏸ goal paused — the provider stayed down; run it again once it is back"
                            .to_string(),
                    ));
                }
                Err(e) => return Err(e).context("running goal turn"),
            },
            _ = &mut interrupted => return Ok(interrupted_end()),
        };
        drop(outcome);
        gs.eval.pending_tokens = total_tokens(session).saturating_sub(before);
        gs.eval.slot.lock().unwrap().take();
        if gs.contract.is_none() {
            match adopt_contract_outcome(&mut gs, session.take_goal_contract_outcome()) {
                Some(Ok(contract)) => {
                    for line in contract.table_lines() {
                        eprintln!("{line}");
                    }
                }
                Some(Err(reason)) => return Ok(GoalRunEnd::NotMet(reason)),
                None => {}
            }
        }
        if let Some(contract) = gs.contract.clone() {
            if !session.run_budget_hit_last_turn() {
                let results = tokio::select! {
                    r = session.validate_goal_contract(&contract) => r,
                    _ = &mut interrupted => return Ok(interrupted_end()),
                };
                *gs.validation.slot.lock().unwrap() = Some(results);
            }
        } else if session.goal_evaluator_enabled() && !session.run_budget_hit_last_turn() {
            let request = gs.eval_request();
            let verdict = tokio::select! {
                v = session.evaluate_goal(&request) => v,
                _ = &mut interrupted => return Ok(interrupted_end()),
            };
            *gs.eval.slot.lock().unwrap() = verdict;
        }
        let facts = GoalTurnFacts::read(session, cwd).without_relay();
        match settle_goal(&mut gs, facts).await {
            GoalStep::Stop { met: true, .. } => return Ok(GoalRunEnd::Met),
            GoalStep::Stop { reason, .. } | GoalStep::Pause { reason } => {
                return Ok(GoalRunEnd::NotMet(reason))
            }
            GoalStep::Relay { detail, .. } => return Ok(GoalRunEnd::NotMet(detail)),
            GoalStep::Turn(next) | GoalStep::Continue(next) => {
                gs.iter += 1;
                prompt = next;
            }
        }
    }
}

fn interrupted_end() -> GoalRunEnd {
    eprintln!("\r\x1b[K\x1b[2m🎯 goal interrupted — stopping (partial output kept)\x1b[0m");
    GoalRunEnd::NotMet("🎯 goal stopped — interrupted".to_string())
}

#[cfg(test)]
#[path = "goal_headless_tests.rs"]
mod tests;
