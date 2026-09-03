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
        cap != 0 && self.turn_billable_input_tokens >= cap
    }

    /// End the turn for the per-turn input-token ceiling. Returns the final text to adopt.
    pub(crate) fn abort_for_token_ceiling(&mut self) -> String {
        let cap = self.config.mesh.max_turn_input_tokens;
        let work = uncommitted_work_message(self.workspace.root());
        self.presenter.emit(PresenterEvent::Error(format!(
            "ERROR: turn input-token ceiling exceeded (cap {cap}, billable input {}, total input \
             {} incl. cache reads, output {}) — ending turn; raise `mesh.max_turn_input_tokens` \
             for longer turns; work is uncommitted: {work}",
            self.turn_billable_input_tokens, self.turn_input_tokens, self.turn_output_tokens
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

    /// One stronger push-back for a bridge that answered with prose while tracked tasks stayed
    /// open: it ran no tool and closed no task, so accepting the reply as an attempt (and then
    /// giving up on the nudge budget) throws the run away. Forbids another restatement and names
    /// the task the next tool call must advance.
    pub(crate) fn escalate_bridge_stall(&mut self, unfinished: &[String]) {
        self.presenter.emit(PresenterEvent::Warning(format!(
            "bridge replied without calling any tool while {} task(s) are open — demanding the \
             next concrete action (1/1)",
            unfinished.len()
        )));
        let nudge = format!(
            "You replied with prose only: you called NO tool and closed NO task, so nothing \
             changed. Do NOT restate, summarize, re-review or re-justify your previous answer — a \
             self-review is not work. Your next message MUST start with a tool call that advances \
             this exact task:\n\n  {first}\n\nRead or edit the specific files it needs, or run the \
             command it needs, and then mark it Done via update_tasks. These tasks are still \
             open:\n- {all}",
            first = unfinished.first().map(String::as_str).unwrap_or(""),
            all = unfinished.join("\n- ")
        );
        let seq = self.next_seq();
        let _ = self
            .store
            .add_message(&self.id, seq, Role::System, &nudge, None);
        self.transcript.push(Message::system(&nudge));
    }

    /// Stop a bridge turn that yielded with tracked tasks open. The turn is always recorded as
    /// incomplete; an ATTENDED session pauses with the resume prompt, while an UNATTENDED one has
    /// nobody to type `continue` and gets back the ERROR text to fail the turn with.
    pub(crate) fn halt_for_unfinished_tasks(
        &mut self,
        unfinished: Vec<String>,
        made_progress: bool,
        unattended: bool,
    ) -> Option<String> {
        let why = if made_progress {
            "reached the continue limit"
        } else {
            "the last attempt made no progress (no task completed, no tool ran)"
        };
        self.turn_unfinished_tasks = unfinished;
        let open = self.turn_unfinished_tasks.clone();
        if unattended {
            return Some(self.abort_for_unfinished_tasks(&open, why));
        }
        self.presenter.emit(PresenterEvent::Warning(format!(
            "bridge stopped with {} task(s) still unfinished — {why}. Send `continue` to resume.",
            open.len()
        )));
        None
    }

    /// End an unattended turn that stopped with tracked tasks still unfinished. Nobody is attached
    /// to type `continue`, so the turn must fail loudly — naming the open work and whether the
    /// worktree was actually touched — rather than exiting 0 as if the plan had been carried out.
    fn abort_for_unfinished_tasks(&mut self, unfinished: &[String], why: &str) -> String {
        let changed = working_tree_status(Some(self.workspace.root()))
            .is_some_and(|status| !status.is_empty());
        let files = if changed {
            uncommitted_work_message(self.workspace.root())
        } else {
            "no files were changed".to_string()
        };
        let message = format!(
            "ERROR: unattended turn ended with {} task(s) unfinished — {why}; open: {}; {files}",
            unfinished.len(),
            unfinished.join("; ")
        );
        self.presenter.emit(PresenterEvent::Error(message.clone()));
        self.turn_hard_guard_abort = true;
        message
    }

    /// The pacing verdict the primary pick was made under, applied to the failover chain as well:
    /// a hop must not walk rank order into a subscription the pacing engine is holding back (two
    /// unattended sessions failed over onto an over-pace ChatGPT plan and burned ~6M input tokens
    /// each, 2026-09-02). Held models stay in the chain but are deferred to last resort, so a turn
    /// still completes when nothing else is left. Empty when failover is off (autofix re-runs).
    pub(crate) fn pacing_held_in_chain(
        &self,
        decision: Option<&forge_mesh::RoutingDecision>,
        active_model: &str,
        fallbacks: &[String],
    ) -> Vec<String> {
        let Some(d) = decision else {
            return Vec::new();
        };
        let mut scope = vec![active_model.to_string()];
        scope.extend(fallbacks.iter().cloned());
        let quota = crate::readiness::ProviderReadiness::snapshot(&self.config, &self.store).quota;
        self.router.pacing_held(d.tier, &scope, &quota)
    }
}
