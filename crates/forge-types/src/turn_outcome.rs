use serde::{Deserialize, Serialize};

/// Why a `run_turn_with` loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model produced a final answer without requesting more tools.
    FinalAnswer,
    /// Loop hit an explicit step limit (`mesh.max_steps`, `--max-turns`) while the model still
    /// wanted tools. Never a default: step limits are opt-in.
    MaxSteps,
    /// A spend or run budget ended the turn: the per-turn token or dollar ceiling, a session cap,
    /// or a goal/loop/`--max-budget-usd` budget.
    BudgetExhausted,
    /// Turn was aborted via `forge_interrupt` (or an equivalent signal).
    Interrupted,
    /// The hosting daemon shut down before the turn could finish.
    DaemonShutdown,
    /// The turn ended having produced NO assistant text and no successful mutating tool call —
    /// it burned a turn and changed nothing. Reported as a failure, never as a completed turn:
    /// from an orchestrator or the phone this was previously indistinguishable from real work.
    NoOutput,
    /// The turn ended with tracked tasks still unfinished — the model stopped (or kept narrating
    /// without acting) before the plan was complete. Never a success: an orchestrator polling the
    /// fleet must see half-done work as half-done.
    TasksUnfinished,
    /// The progress supervisor ended the turn: nothing had changed and nothing new had turned up
    /// for long enough, even after a nudge. Edits are kept and `continue` starts a fresh window.
    /// Never a success: the work the person asked for is not known to be done.
    Stalled,
}

impl StopReason {
    /// Stable wire name, identical to the serde representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FinalAnswer => "final_answer",
            Self::MaxSteps => "max_steps",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Interrupted => "interrupted",
            Self::DaemonShutdown => "daemon_shutdown",
            Self::NoOutput => "no_output",
            Self::TasksUnfinished => "tasks_unfinished",
            Self::Stalled => "stalled",
        }
    }

    /// The coarse outcome a remote client needs: did the turn finish the work (`success`) or fail
    /// to do so (`failed`)? Callers wanting the precise reason read [`StopReason::as_str`]
    /// alongside this.
    pub const fn outcome(self) -> &'static str {
        match self {
            Self::FinalAnswer => "success",
            Self::DaemonShutdown => "interrupted",
            Self::MaxSteps
            | Self::BudgetExhausted
            | Self::Interrupted
            | Self::NoOutput
            | Self::TasksUnfinished
            | Self::Stalled => "failed",
        }
    }

    /// Whether the turn is an honest success. A no-output turn is NOT.
    pub const fn is_success(self) -> bool {
        matches!(self, Self::FinalAnswer)
    }
}

/// The result of a completed (or interrupted) agent turn.
#[derive(Debug, Clone)]
pub struct LoopOutcome {
    /// The assistant's final text response.
    pub text: String,
    /// Why the turn ended.
    pub stop_reason: StopReason,
}

impl LoopOutcome {
    pub fn final_answer(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::FinalAnswer,
        }
    }

    pub fn max_steps(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::MaxSteps,
        }
    }

    pub fn budget_exhausted(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::BudgetExhausted,
        }
    }

    /// The progress supervisor stopped the turn.
    pub fn stalled(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::Stalled,
        }
    }

    /// The turn produced nothing — no assistant text, no successful mutating tool call.
    pub fn no_output(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::NoOutput,
        }
    }
}

impl std::ops::Deref for LoopOutcome {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Display for LoopOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl PartialEq for LoopOutcome {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.stop_reason == other.stop_reason
    }
}

impl PartialEq<str> for LoopOutcome {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for LoopOutcome {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<String> for LoopOutcome {
    fn eq(&self, other: &String) -> bool {
        &self.text == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_outcome_separates_success_failure_and_produced_nothing() {
        assert_eq!(StopReason::FinalAnswer.outcome(), "success");
        assert_eq!(StopReason::NoOutput.outcome(), "failed");
        for failed in [
            StopReason::MaxSteps,
            StopReason::BudgetExhausted,
            StopReason::Interrupted,
            StopReason::TasksUnfinished,
            StopReason::Stalled,
        ] {
            assert_eq!(failed.outcome(), "failed", "{failed:?} is not a success");
        }

        for reason in [
            StopReason::FinalAnswer,
            StopReason::MaxSteps,
            StopReason::BudgetExhausted,
            StopReason::Interrupted,
            StopReason::DaemonShutdown,
            StopReason::NoOutput,
            StopReason::TasksUnfinished,
            StopReason::Stalled,
        ] {
            assert_eq!(
                serde_json::to_string(&reason).unwrap(),
                format!("\"{}\"", reason.as_str())
            );
            assert_eq!(reason.is_success(), reason == StopReason::FinalAnswer);
        }
        assert_eq!(StopReason::DaemonShutdown.outcome(), "interrupted");
        assert_eq!(StopReason::DaemonShutdown.as_str(), "daemon_shutdown");
        assert_eq!(StopReason::Stalled.as_str(), "stalled");
    }
}
