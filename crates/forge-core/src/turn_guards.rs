//! Step and token guards that bound one turn.
//!
//! An ATTENDED turn treats `mesh.max_steps` as a checkpoint it can pause on — a human types
//! `continue`. An UNATTENDED turn (headless `forge run`, a detached daemon session, or
//! `--mode bypass`) has nobody to type it, so pausing there is a silent death with the work left
//! uncommitted. These guards make the unattended path warn at the soft cap and run on to
//! `mesh.max_steps_unattended`, then end the turn with an ERROR naming the uncommitted work.
//! `mesh.max_turn_input_tokens` ends a turn on every surface, because the step cap alone does not
//! bound spend.

use super::*;

impl Session {
    /// Whether nobody is present to answer a prompt for this turn.
    pub(crate) fn turn_is_unattended(&self) -> bool {
        !self.presenter.is_attended() || self.mode == PermissionMode::Bypass
    }

    /// A hard guard already ended this turn, so every later re-drive (nudge, verification,
    /// autofix) must refuse to start a fresh loop rather than reset the counters that stopped it.
    pub(crate) fn hard_guard_short_circuit(
        &self,
        active_model: String,
    ) -> Option<ModelLoopOutcome> {
        if !self.turn_hard_guard_abort {
            return None;
        }
        Some(ModelLoopOutcome {
            final_text: String::new(),
            context_tokens: 0,
            hit_step_cap: false,
            halted_by_loop_guard: true,
            active_model,
            plan: None,
            tools_ran: 0,
            mcp_tools_unavailable: false,
            hard_guard_abort: true,
        })
    }

    /// One warning as an unattended turn passes the soft cap. Names the spend so a runaway is
    /// visible in the log stream rather than only in the final bill.
    pub(crate) fn warn_soft_step_checkpoint(&mut self, soft_cap: usize, step: usize, hard: usize) {
        self.presenter.emit(PresenterEvent::Warning(format!(
            "reached soft step cap {soft_cap} at step {step}; unattended turn continuing toward \
             hard cap {hard} (turn tokens: input={}, output={})",
            self.turn_input_tokens, self.turn_output_tokens
        )));
    }

    pub(crate) fn turn_input_ceiling_hit(&self) -> bool {
        let cap = self.config.mesh.max_turn_input_tokens;
        cap != 0 && self.turn_input_tokens >= cap
    }

    /// End the turn for the per-turn input-token ceiling. Returns the final text to adopt.
    pub(crate) fn abort_for_token_ceiling(&mut self) -> String {
        let cap = self.config.mesh.max_turn_input_tokens;
        let work = uncommitted_work_message(self.workspace.root());
        self.presenter.emit(PresenterEvent::Error(format!(
            "ERROR: turn input-token ceiling exceeded (cap {cap}, input {}, output {}) — ending \
             turn; work is uncommitted: {work}",
            self.turn_input_tokens, self.turn_output_tokens
        )));
        self.turn_hard_guard_abort = true;
        format!("ERROR: turn input-token ceiling exceeded; work is uncommitted: {work}")
    }

    /// End an unattended turn that ran all the way to the hard step ceiling.
    pub(crate) fn abort_for_step_ceiling(&mut self, hard: usize, soft_cap: usize) -> String {
        let work = uncommitted_work_message(self.workspace.root());
        self.presenter.emit(PresenterEvent::Error(format!(
            "ERROR: unattended turn reached hard step ceiling {hard} (soft cap {soft_cap}; turn \
             tokens: input={}, output={}) — ending turn; work is uncommitted: {work}",
            self.turn_input_tokens, self.turn_output_tokens
        )));
        self.turn_hard_guard_abort = true;
        format!(
            "ERROR: unattended turn reached hard step ceiling {hard}; work is uncommitted: {work}"
        )
    }
}
