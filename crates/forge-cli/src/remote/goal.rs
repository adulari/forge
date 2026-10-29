//! The `/goal` run on the wire: state, budget and the request a command left pending, so a
//! client can show a chip and offer pause, resume and stop without parsing transcript notes.

use forge_tui::goal_status::GoalStatus;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapGoal {
    pub objective: String,
    /// `active`, `paused`, `budget_limit`, `complete`, `failed` or `cancelled`.
    pub state: &'static str,
    pub iteration: u64,
    pub tokens_used: u64,
    pub max_tokens: Option<u64>,
    pub max_minutes: Option<u64>,
    /// Run time as of `synced_at`; an `active` goal keeps running past it.
    pub elapsed_secs: u64,
    pub synced_at: u64,
    pub gates: Vec<String>,
    /// Why a goal that did not complete stopped.
    pub reason: Option<String>,
    /// `pause` or `clear`, applied when the turn in flight ends.
    pub pending: Option<&'static str>,
}

impl From<&GoalStatus> for SnapGoal {
    fn from(goal: &GoalStatus) -> Self {
        Self {
            objective: goal.objective.clone(),
            state: goal.phase.wire(),
            iteration: goal.iteration as u64,
            tokens_used: goal.tokens_used,
            max_tokens: goal.max_tokens,
            max_minutes: goal.max_minutes,
            elapsed_secs: goal.elapsed_secs,
            synced_at: goal.synced_at_ms,
            gates: goal.gates.clone(),
            reason: goal.reason.clone(),
            pending: goal.request.map(|request| request.wire()),
        }
    }
}
