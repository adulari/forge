//! The `/goal` run as every surface sees it: one status record on the app, written by the run
//! loop and read by the composer chip, `/status`, `/goal status` and the remote snapshot.
//!
//! The loop owns the real run state (`GoalState` in forge-cli); this record is the part the
//! surfaces need, plus the requests a `/goal pause|clear|edit` typed mid-run leaves for the loop
//! to apply at the next turn boundary. Commands never take the session lock, so a turn that is
//! still running cannot block them.

use crate::app::App;
use forge_types::goal_contract::{AssertionResult, GoalContract};
use forge_types::goal_envelope::{EnvelopeSpec, EnvelopeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalPhase {
    Active,
    Paused,
    BudgetLimit,
    Complete,
    Failed,
    Cancelled,
    /// The goal moved to a fresh worker session; this session no longer runs it.
    HandedOff,
}

/// One session of a goal's relay chain (docs/features/goal-contract.md, "Fresh-worker relay").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalRelayLink {
    pub session_id: String,
    /// Why this session handed the goal on (`wedged`, `stalled`, `worn`); `None` for the session
    /// that holds the goal now.
    pub handed_off: Option<String>,
}

impl GoalRelayLink {
    /// `S1 handed off (wedged)` / `S2 running`.
    pub fn line(&self, number: usize, tip_state: &str) -> String {
        match &self.handed_off {
            Some(reason) => format!("S{number} handed off ({reason})"),
            None => format!("S{number} {tip_state}"),
        }
    }
}

impl GoalPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::BudgetLimit => "budget limit",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::HandedOff => "handed off",
        }
    }

    /// The inverse of [`GoalPhase::wire`].
    pub fn from_wire(tag: &str) -> Option<Self> {
        Some(match tag {
            "active" => Self::Active,
            "paused" => Self::Paused,
            "budget_limit" => Self::BudgetLimit,
            "complete" => Self::Complete,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "handed_off" => Self::HandedOff,
            _ => return None,
        })
    }

    /// The wire tag remote clients switch on.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::BudgetLimit => "budget_limit",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::HandedOff => "handed_off",
        }
    }
}

/// What a command asked of a goal that is still running; the loop applies it between turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalRequest {
    Pause,
    Clear,
}

impl GoalRequest {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Clear => "clear",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GoalStatus {
    pub objective: String,
    pub phase: GoalPhase,
    pub iteration: usize,
    pub tokens_used: u64,
    pub max_tokens: Option<u64>,
    pub max_minutes: Option<u64>,
    /// Run time up to the last sync; an active goal adds the time since `synced_at`.
    pub elapsed_secs: u64,
    pub gates: Vec<String>,
    /// Why a goal that did not complete stopped.
    pub reason: Option<String>,
    pub request: Option<GoalRequest>,
    pub edit: Option<String>,
    /// The spend envelope (`/goal --budget`), when the goal has one.
    pub envelope: Option<EnvelopeState>,
    /// A new envelope a `/goal budget` typed mid-run left for the loop to apply.
    pub envelope_edit: Option<EnvelopeSpec>,
    /// The approved validation contract and the validator's latest results
    /// (docs/features/goal-contract.md); `None` for a goal without one.
    pub contract: Option<GoalContract>,
    /// The sessions the goal has run in, oldest first, the one holding it last; empty until the
    /// goal has been handed to a fresh worker.
    pub relay: Vec<GoalRelayLink>,
    /// Wall clock of the last sync, Unix ms, so a client can keep the elapsed time ticking.
    pub synced_at_ms: u64,
    carry: bool,
}

/// What a resumed goal keeps from the run it continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalCarry {
    pub iteration: usize,
    pub tokens_used: u64,
    pub elapsed_secs: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl GoalStatus {
    pub fn is_live(&self) -> bool {
        self.phase == GoalPhase::Active
    }

    /// `2/5` once a plan exists, else the iteration, then the state and the token budget.
    pub fn chip(&self, done: usize, total: usize) -> String {
        let progress = if total == 0 {
            format!("iteration {}", self.iteration)
        } else {
            format!("{done}/{total}")
        };
        let budget = self
            .max_tokens
            .map(|max| format!(" · {}/{} tok", compact(self.tokens_used), compact(max)));
        let budget = budget.unwrap_or_default();
        let budget = match &self.envelope {
            Some(envelope) => format!("{budget} · {}", envelope.meter()),
            None => budget,
        };
        let budget = match self.contract.as_ref().filter(|c| !c.results.is_empty()) {
            Some(contract) => format!("{budget} · contract {}", contract.headline()),
            None => budget,
        };
        match self.phase {
            GoalPhase::Active => match self.request {
                Some(GoalRequest::Pause) => format!("{progress} · pausing{budget}"),
                Some(GoalRequest::Clear) => format!("{progress} · stopping{budget}"),
                None => format!("{progress}{budget}"),
            },
            GoalPhase::Paused => format!("paused · {progress}{budget}{}", self.resume_hint()),
            phase => format!("{} · {progress}{budget}", phase.label()),
        }
    }

    /// ` · resumes at 04:00` while the goal waits for its window to reset and will continue by
    /// itself.
    fn resume_hint(&self) -> String {
        match &self.envelope {
            Some(e) if e.auto_resume_at().is_some() => e
                .resume_label
                .as_ref()
                .map_or_else(String::new, |at| format!(" · resumes at {at}")),
            _ => String::new(),
        }
    }

    /// `S1 handed off (wedged) -> S2 running`; `None` for a goal that never changed worker.
    pub fn relay_line(&self) -> Option<String> {
        if self.relay.is_empty() {
            return None;
        }
        let tip = match self.phase {
            GoalPhase::Active => "running",
            GoalPhase::Paused => "paused",
            GoalPhase::Complete => "complete",
            GoalPhase::HandedOff => "starting",
            _ => "stopped",
        };
        let parts: Vec<String> = self
            .relay
            .iter()
            .enumerate()
            .map(|(i, link)| link.line(i + 1, tip))
            .collect();
        Some(parts.join(" -> "))
    }

    /// The `/goal status` lines.
    pub fn summary(&self) -> String {
        let mut out = format!("🎯 goal {} — {}", self.phase.label(), self.objective);
        out.push_str(&format!(
            "\n   iteration {} · {} tokens",
            self.iteration,
            compact(self.tokens_used)
        ));
        if let Some(max) = self.max_tokens {
            out.push_str(&format!(" of {}", compact(max)));
        }
        out.push_str(&format!(" · {}s", self.elapsed_secs));
        if let Some(max) = self.max_minutes {
            out.push_str(&format!(" of {max}m"));
        }
        if let Some(envelope) = &self.envelope {
            out.push_str(&format!("\n   {}", envelope.meter()));
            if self.phase == GoalPhase::Paused {
                if let (Some(_), Some(at)) = (envelope.auto_resume_at(), &envelope.resume_label) {
                    out.push_str(&format!("\n   resumes at {at} when the window resets"));
                }
            }
        }
        if let Some(chain) = self.relay_line() {
            out.push_str(&format!("\n   {chain}"));
        }
        if !self.gates.is_empty() {
            out.push_str(&format!("\n   gates: {}", self.gates.join(", ")));
        }
        if let Some(reason) = &self.reason {
            out.push_str(&format!("\n   {reason}"));
        }
        if let Some(contract) = &self.contract {
            out.push_str(&format!("\n   📜 contract — {}", contract.headline()));
            for line in contract.table_lines() {
                out.push_str(&format!("\n   {line}"));
            }
        }
        out
    }
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0).replace(".0k", "k"),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0).replace(".0M", "M"),
    }
}

impl App {
    /// A goal run begins, or resumes the paused goal it matches. Returns what the run carries
    /// over from the paused one; its iteration is the next turn, not the one that paused.
    pub fn goal_begin(
        &mut self,
        objective: &str,
        gates: &[String],
        max_tokens: Option<u64>,
        max_minutes: Option<u64>,
    ) -> Option<GoalCarry> {
        let kept_envelope = self
            .goal
            .as_ref()
            .filter(|g| g.carry && g.objective == objective)
            .and_then(|g| g.envelope.clone());
        let envelope = self.pending_envelope.take().or(kept_envelope);
        let kept_contract = self
            .goal
            .as_ref()
            .filter(|g| g.carry && g.objective == objective)
            .and_then(|g| g.contract.clone());
        let carry = self
            .goal
            .as_ref()
            .filter(|g| g.carry && g.objective == objective)
            .map(|g| GoalCarry {
                iteration: g.iteration + 1,
                tokens_used: g.tokens_used,
                elapsed_secs: g.elapsed_secs,
            });
        let base = carry.unwrap_or(GoalCarry {
            iteration: 1,
            tokens_used: 0,
            elapsed_secs: 0,
        });
        self.goal = Some(GoalStatus {
            objective: objective.to_string(),
            phase: GoalPhase::Active,
            iteration: base.iteration,
            tokens_used: base.tokens_used,
            max_tokens,
            max_minutes,
            elapsed_secs: base.elapsed_secs,
            gates: gates.to_vec(),
            reason: None,
            request: None,
            edit: None,
            envelope,
            envelope_edit: None,
            contract: kept_contract,
            relay: Vec::new(),
            synced_at_ms: now_ms(),
            carry: false,
        });
        carry
    }

    /// The sessions the goal has run in.
    pub fn goal_set_relay(&mut self, chain: Vec<GoalRelayLink>) {
        if let Some(goal) = self.goal.as_mut() {
            goal.relay = chain;
        }
    }

    /// The goal moved to a fresh worker session: this one no longer runs it.
    pub fn goal_handed_off(&mut self, reason: &str) {
        if let Some(goal) = self.goal.as_mut() {
            goal.phase = GoalPhase::HandedOff;
            goal.request = None;
            goal.reason = Some(reason.to_string());
        }
    }

    /// The approved contract for the running goal.
    pub fn goal_set_contract(&mut self, contract: GoalContract) {
        if let Some(goal) = self.goal.as_mut() {
            goal.contract = Some(contract);
        }
    }

    /// Record a validator run on the goal's contract.
    pub fn goal_contract_results(&mut self, results: Vec<AssertionResult>) {
        if let Some(contract) = self.goal.as_mut().and_then(|g| g.contract.as_mut()) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            contract.record(results, now);
        }
    }

    /// Record the envelope's latest accounting so every surface shows the same meter.
    pub fn goal_envelope_sync(&mut self, envelope: &EnvelopeState) {
        if let Some(goal) = self.goal.as_mut() {
            goal.envelope = Some(envelope.clone());
        }
    }

    /// Ask a live goal to move its envelope; a paused goal takes it at once. `false` when there
    /// is no goal with an envelope to move.
    pub fn goal_request_envelope(&mut self, spec: EnvelopeSpec) -> bool {
        let Some(goal) = self.goal.as_mut().filter(|g| g.envelope.is_some()) else {
            return false;
        };
        match goal.phase {
            GoalPhase::Active => {
                goal.envelope_edit = Some(spec);
                true
            }
            GoalPhase::Paused => {
                if let Some(envelope) = goal.envelope.as_mut() {
                    envelope.retarget(spec);
                    if !envelope.spent {
                        goal.reason = None;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Bring back a goal a restart interrupted, as a paused goal that `/goal resume` continues
    /// (`phase` Paused), or a goal that had ended, kept for its contract table.
    #[allow(clippy::too_many_arguments)]
    pub fn goal_restore(
        &mut self,
        objective: &str,
        gates: &[String],
        max_tokens: Option<u64>,
        max_minutes: Option<u64>,
        carry: GoalCarry,
        reason: Option<String>,
        envelope: Option<EnvelopeState>,
        contract: Option<GoalContract>,
        phase: GoalPhase,
    ) {
        self.goal = Some(GoalStatus {
            objective: objective.to_string(),
            phase,
            iteration: carry.iteration,
            tokens_used: carry.tokens_used,
            max_tokens,
            max_minutes,
            elapsed_secs: carry.elapsed_secs,
            gates: gates.to_vec(),
            reason,
            request: None,
            edit: None,
            envelope,
            envelope_edit: None,
            contract,
            relay: Vec::new(),
            synced_at_ms: now_ms(),
            carry: false,
        });
    }

    pub fn goal_take_envelope_edit(&mut self) -> Option<EnvelopeSpec> {
        self.goal.as_mut()?.envelope_edit.take()
    }

    /// The goal stopped because its envelope is spent (or cannot be verified): paused, so it can
    /// be resumed, with the reason on the record.
    pub fn goal_paused_by_envelope(&mut self, reason: &str) {
        if let Some(goal) = self.goal.as_mut() {
            goal.phase = GoalPhase::Paused;
            goal.request = None;
            goal.reason = Some(reason.to_string());
        }
    }

    /// Record the run's progress; called whenever the loop's own counters move.
    pub fn goal_sync(&mut self, iteration: usize, tokens_used: u64, elapsed_secs: u64) {
        if let Some(goal) = self.goal.as_mut().filter(|g| g.is_live()) {
            goal.iteration = iteration;
            goal.tokens_used = tokens_used;
            goal.elapsed_secs = elapsed_secs;
            goal.synced_at_ms = now_ms();
        }
    }

    /// Change the live goal's wording, or the paused goal's. A running loop applies it at its
    /// next turn boundary; a paused goal takes it at once.
    pub fn goal_request_edit(&mut self, objective: String) -> bool {
        match self.goal.as_mut() {
            Some(goal) if goal.is_live() => {
                goal.edit = Some(objective);
                true
            }
            Some(goal) if goal.phase == GoalPhase::Paused => {
                goal.objective = objective;
                true
            }
            _ => false,
        }
    }

    pub fn goal_take_edit(&mut self) -> Option<String> {
        let edit = self.goal.as_mut()?.edit.take()?;
        self.goal.as_mut()?.objective = edit.clone();
        Some(edit)
    }

    pub fn goal_take_request(&mut self) -> Option<GoalRequest> {
        self.goal.as_mut()?.request.take()
    }

    /// Ask a live goal to pause or stop after the turn in flight; `false` when none is live.
    pub fn goal_request(&mut self, request: GoalRequest) -> bool {
        match self.goal.as_mut() {
            Some(goal) if goal.is_live() => {
                goal.request = Some(request);
                true
            }
            _ => false,
        }
    }

    pub fn goal_paused(&mut self) {
        if let Some(goal) = self.goal.as_mut() {
            goal.phase = GoalPhase::Paused;
            goal.request = None;
            goal.reason = None;
        }
    }

    /// The goal ended: `met` is a verified completion; otherwise `reason` says why it stopped.
    pub fn goal_ended(&mut self, met: bool, reason: &str) {
        let Some(goal) = self.goal.as_mut() else {
            return;
        };
        goal.request = None;
        if met {
            goal.phase = GoalPhase::Complete;
            goal.reason = None;
            return;
        }
        goal.phase = if reason.contains("budget") {
            GoalPhase::BudgetLimit
        } else {
            GoalPhase::Failed
        };
        goal.reason = Some(reason.to_string());
    }

    /// Esc, a model switch mid-turn, or a driver interrupt ended a live goal.
    pub fn goal_interrupted(&mut self) {
        if let Some(goal) = self.goal.as_mut().filter(|g| g.is_live()) {
            goal.phase = GoalPhase::Cancelled;
            goal.request = None;
        }
    }

    pub fn goal_clear(&mut self) {
        self.goal = None;
    }

    /// Mark the paused goal as continuing: the next `begin` for the same objective keeps its
    /// counters. `None` when there is no paused goal.
    pub fn goal_prepare_resume(&mut self) -> Option<GoalStatus> {
        let goal = self
            .goal
            .as_mut()
            .filter(|g| g.phase == GoalPhase::Paused)?;
        goal.carry = true;
        Some(goal.clone())
    }
}

#[cfg(test)]
#[path = "goal_status_tests.rs"]
mod tests;
