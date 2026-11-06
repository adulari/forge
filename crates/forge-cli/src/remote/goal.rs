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
    /// The spend envelope (`/goal --budget`); absent when the goal has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<SnapEnvelope>,
}

/// A goal's spend envelope: how much of its slice is used, ready to draw as a fill bar.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapEnvelope {
    /// `percent` of a subscription window, or `usd`.
    pub unit: &'static str,
    pub used: f64,
    pub slice: f64,
    /// `claude weekly`, `codex 5h` or `spend`.
    pub label: String,
    /// The ready-to-show line, e.g. `envelope 12/20% claude weekly`.
    pub meter: String,
    pub spent: bool,
    /// The last check found no fresh reading to measure against.
    pub unverified: bool,
    /// When the measured window resets, Unix seconds.
    pub resets_at: Option<i64>,
    /// When a goal paused on this envelope continues by itself (the window reset), Unix seconds.
    /// Absent when it will not: the user paused it, it is not spent, or the reset time is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumes_at: Option<i64>,
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
            envelope: goal.envelope.as_ref().map(|e| SnapEnvelope {
                unit: if e.is_usd() { "usd" } else { "percent" },
                used: e.used,
                slice: e.spec.slice,
                label: e.spec.label(),
                meter: e.meter(),
                spent: e.spent,
                unverified: e.unverified,
                resets_at: e.resets_at,
                resumes_at: e.auto_resume_at(),
            }),
        }
    }
}
