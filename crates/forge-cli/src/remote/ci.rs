//! The local CI plan on the wire (docs/features/ci-parity.md): a compact summary per session so a
//! client can draw a chip and say which step blocked a goal or a push without fetching the plan.
//! The full plan, env and output tails come from `GET /api/sessions/{id}/ci`.

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SnapCi {
    /// Steps in the plan / skips listed with a reason.
    pub runnable: usize,
    pub skipped: usize,
    /// The step in flight while a run is going.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<SnapCiRunning>,
    /// Results of the last (or current) run, in plan order.
    pub steps: Vec<SnapCiStep>,
    /// When the last run finished, Unix seconds; absent while the first run is going.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_unix: Option<i64>,
    /// The last finished run passed (or only skipped tools that are not installed here).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// The step that is holding a goal or a push back right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<SnapCiBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SnapCiRunning {
    pub name: String,
    pub index: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SnapCiStep {
    pub key: String,
    pub name: String,
    /// `pass`, `fail`, `skip` (tool not installed here) or `not_run`.
    pub status: &'static str,
    pub secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SnapCiBlock {
    pub key: String,
    pub name: String,
    /// `goal` (the finish gate) or `push` (ship and the merge queue).
    pub via: &'static str,
}
