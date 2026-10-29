//! `/goal pause|resume|clear|status|edit` — steering a goal that is running or paused.
//!
//! A command typed while a goal runs may not take the session lock (the turn holds it), so
//! pause, clear and edit only leave a request on the app's [`GoalStatus`]. The goal loop applies
//! it in [`apply_goal_requests`] when the turn in flight ends, before the evaluator is paid for
//! or another turn is started. Resume runs while idle and restarts the goal through the same
//! [`DispatchOutcome::StartGoal`] a new goal uses, carrying the counters over.
//!
//! [`GoalStatus`]: forge_tui::goal_status::GoalStatus

use std::sync::Arc;

use forge_tui::goal_status::{GoalPhase, GoalRequest};
use forge_tui::GoalControl;

use super::dispatch::DispatchOutcome;
use super::*;

const NO_GOAL: &str = "no goal — /goal <objective> sets one";

pub(crate) async fn goal_control_outcome(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    control: GoalControl,
) -> anyhow::Result<DispatchOutcome> {
    match control {
        GoalControl::Status => match &app.goal {
            Some(goal) => app.note(&goal.summary()),
            None => app.note(NO_GOAL),
        },
        GoalControl::Pause => {
            if app.goal_request(GoalRequest::Pause) {
                app.note("⏸ goal pauses after this turn — /goal resume continues it");
            } else if app
                .goal
                .as_ref()
                .is_some_and(|g| g.phase == GoalPhase::Paused)
            {
                app.note("goal is already paused — /goal resume continues it");
            } else {
                app.note("no running goal to pause");
            }
        }
        GoalControl::Clear => {
            if app.goal_request(GoalRequest::Clear) {
                app.note("🎯 goal stops after this turn and is cleared");
            } else if app.goal.is_some() {
                app.goal_clear();
                app.note("🎯 goal cleared");
            } else {
                app.note(NO_GOAL);
            }
        }
        GoalControl::Edit(text) => {
            if app.goal_request_edit(text.clone()) {
                app.note("✎ goal reworded — a running goal uses it from its next turn");
            } else {
                return gates::goal_outcome(
                    session,
                    app,
                    format!("edit {text}"),
                    Vec::new(),
                    None,
                    None,
                )
                .await;
            }
        }
        GoalControl::Resume => {
            let Some(goal) = app.goal_prepare_resume() else {
                app.note("no paused goal to resume");
                return Ok(DispatchOutcome::Handled);
            };
            {
                let mut s = session.lock().await;
                s.prime_guidance(&[goal_session_guidance(&goal.objective)])
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                s.set_active_goal(Some(&goal.objective));
            }
            app.note(&format!("▶ goal resumed — {}", goal.objective));
            return Ok(DispatchOutcome::StartGoal {
                prompt: format!("Goal: {}\n\n{GOAL_CONTINUE_PROMPT}", goal.objective),
                goal: goal.objective,
                gates: goal.gates,
                max_tokens: goal.max_tokens,
                max_minutes: goal.max_minutes,
            });
        }
    }
    Ok(DispatchOutcome::Handled)
}

/// Apply what `/goal edit|pause|clear` left for the loop. `Some(GoalNext::Stop)` ends the run:
/// the goal is paused or cleared and nothing else is started.
pub(crate) async fn apply_goal_requests(
    gs: &mut GoalState,
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
) -> Option<GoalNext> {
    let turn_tokens = if gs.eval.evaluating {
        gs.eval.pending_tokens
    } else {
        app.turn_in + app.turn_out
    };
    app.goal_sync(
        gs.iter,
        gs.budget.tokens_used.saturating_add(turn_tokens),
        gs.budget.started_at.elapsed().as_secs(),
    );
    if let Some(text) = app.goal_take_edit() {
        gs.goal = text.clone();
        gs.no_progress = 0;
        gs.eval.last = None;
        gs.eval.unchanged = 0;
        gs.eval.disagreements = 0;
        session.lock().await.set_active_goal(Some(&text));
    }
    match app.goal_take_request()? {
        GoalRequest::Pause => {
            app.goal_paused();
            session.lock().await.set_active_goal(None);
            app.note("⏸ goal paused — /goal resume continues it");
        }
        GoalRequest::Clear => {
            app.goal_clear();
            session.lock().await.set_active_goal(None);
            app.note("🎯 goal stopped and cleared");
        }
    }
    Some(GoalNext::Stop)
}

/// The run was interrupted (Esc, a model switch mid-turn): the goal is over, not paused.
pub(crate) fn abandon_goal(goal_state: &mut Option<GoalState>, app: &mut forge_tui::App) {
    if goal_state.take().is_some() {
        app.goal_interrupted();
    }
}

#[cfg(test)]
#[path = "goal_control_tests.rs"]
mod tests;
