//! Wall-clock and token ceiling for one autonomous `/goal` or `/loop` run, enforced INSIDE a turn.
//!
//! The run drivers used to check `--max-tokens` / `--max-minutes` only between turns, so a single
//! long turn (hundreds of steps) could overshoot either ceiling arbitrarily. The driver now hands
//! the session what is LEFT of the run's budget before each turn; the step loop checks it before
//! every model call and after every response and, once spent, ends the turn through the same hard
//! guard latch the spend and token guards use (so no re-drive restarts the loop). The turn reports
//! [`StopReason::BudgetExhausted`], which the driver turns into its "budget exhausted" stop.

use super::*;

/// What remains of a run's budget at the start of a turn. `None` fields are unbounded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunBudget {
    /// Tokens (input + output, the same sum the driver accumulates) this turn may still use.
    pub tokens_remaining: Option<u64>,
    /// Wall-clock instant after which no further model call may start.
    pub deadline: Option<std::time::Instant>,
}

/// Which ceiling tripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunBudgetHit {
    Tokens,
    Time,
}

/// Pure check, shared by the live guard and the tests.
pub(crate) fn run_budget_hit(
    budget: &RunBudget,
    turn_tokens: u64,
    now: std::time::Instant,
) -> Option<RunBudgetHit> {
    if budget
        .tokens_remaining
        .is_some_and(|left| turn_tokens >= left)
    {
        Some(RunBudgetHit::Tokens)
    } else if budget.deadline.is_some_and(|d| now >= d) {
        Some(RunBudgetHit::Time)
    } else {
        None
    }
}

impl Session {
    /// Arm the run budget for the NEXT turn only. The turn consumes it when it starts, so a budget
    /// armed for a goal iteration that never ran (or an interrupted one) cannot bound some later
    /// unrelated turn: the driver re-arms before every iteration of a bounded run.
    pub fn set_run_budget(&mut self, budget: Option<RunBudget>) {
        self.run_budget_next = budget;
    }

    /// Whether the run budget ended the most recent turn mid-flight.
    pub fn run_budget_hit_last_turn(&self) -> bool {
        self.turn_run_budget_hit
    }

    pub(crate) fn run_budget_tripped(&self) -> Option<RunBudgetHit> {
        let budget = self.run_budget.as_ref()?;
        run_budget_hit(
            budget,
            self.turn_input_tokens
                .saturating_add(self.turn_output_tokens),
            std::time::Instant::now(),
        )
    }

    /// End the turn because the run budget is spent. Returns the final text to adopt. Work is
    /// preserved exactly as for the other hard guards.
    pub(crate) fn abort_for_run_budget(&mut self, hit: RunBudgetHit) -> String {
        let which = match hit {
            RunBudgetHit::Tokens => "token budget",
            RunBudgetHit::Time => "time budget",
        };
        let work = self.preserve_uncommitted_work();
        self.presenter.emit(PresenterEvent::Warning(format!(
            "run {which} exhausted mid-turn — stopping the turn here; work is uncommitted: {work}"
        )));
        self.turn_hard_guard_abort = true;
        self.turn_run_budget_hit = true;
        format!("Stopped: the run's {which} was exhausted mid-turn; work is uncommitted: {work}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn unbounded_budget_never_trips() {
        assert_eq!(
            run_budget_hit(&RunBudget::default(), u64::MAX, Instant::now()),
            None
        );
    }

    #[test]
    fn tokens_trip_at_the_remaining_amount() {
        let b = RunBudget {
            tokens_remaining: Some(100),
            deadline: None,
        };
        assert_eq!(run_budget_hit(&b, 99, Instant::now()), None);
        assert_eq!(
            run_budget_hit(&b, 100, Instant::now()),
            Some(RunBudgetHit::Tokens)
        );
    }

    #[test]
    fn deadline_trips_once_passed() {
        let now = Instant::now();
        let b = RunBudget {
            tokens_remaining: None,
            deadline: Some(now + Duration::from_secs(5)),
        };
        assert_eq!(run_budget_hit(&b, 0, now), None);
        assert_eq!(
            run_budget_hit(&b, 0, now + Duration::from_secs(5)),
            Some(RunBudgetHit::Time)
        );
    }
}
