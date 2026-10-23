//! Hard guards that end a turn: the spend and token ceilings, an explicit step limit, and the
//! unfinished-task halts.
//!
//! There is no default step cap. A turn that is making progress is never stopped by a count, and
//! one that is not is caught by the progress supervisor (`progress_ledger.rs`), which stops it
//! cleanly. `mesh.max_steps` and `mesh.max_steps_unattended` are explicit limits the user opts
//! into (`--max-turns` sets both): an ATTENDED turn pauses at the limit for a human to type
//! `continue`, an UNATTENDED one ends with an ERROR naming the uncommitted work.
//! `mesh.max_turn_input_tokens` ends a turn on every surface, because a step count says nothing
//! about spend.

use super::*;

impl Session {
    /// Whether nobody is present to answer a prompt for this turn.
    pub(crate) fn turn_is_unattended(&self) -> bool {
        !self.presenter.is_attended() || self.mode == PermissionMode::Bypass
    }

    /// A hard guard already ended this turn, so every later re-drive (nudge, verification,
    /// autofix) must refuse to start a fresh loop rather than reset the counters that stopped it.
    pub(crate) fn hard_guard_short_circuit(
        &mut self,
        active_model: String,
    ) -> Option<ModelLoopOutcome> {
        // A run budget that ran out between model calls (or before the first one) ends the turn
        // here; one that runs out DURING a call is caught right after it, by `hard_guard_after_call`.
        if !self.turn_hard_guard_abort {
            if let Some(hit) = self.run_budget_tripped() {
                self.abort_for_run_budget(hit);
            }
        }
        // A stalled turn is not a hard abort (an attended one still gets a status written), but a
        // re-drive must not restart the loop whose evidence stopped it.
        if !self.turn_hard_guard_abort && self.turn_forced_stop.is_none() {
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

    pub(crate) fn turn_input_ceiling_hit(&self) -> bool {
        let cap = self.config.mesh.max_turn_input_tokens;
        cap != 0 && self.turn_billable_input_tokens >= cap
    }

    /// Preserve whatever the turn had reached before a hard guard ends it, and describe both the
    /// files and where they were saved.
    ///
    /// Naming the uncommitted files without saving them is what made these aborts expensive: the
    /// error said "work is uncommitted: … and 56 more" and then the run ended, leaving a human to
    /// work out what a 400-step turn had been in the middle of. The files are left in place; the
    /// snapshot is an extra copy, not a move.
    pub(crate) fn preserve_uncommitted_work(&self) -> String {
        let root = self.workspace.root();
        let work = uncommitted_work_message(root);
        match snapshot_uncommitted_work(root) {
            Some(sha) => format!(
                "{work} — files left in place; tracked edits also snapshotted to {sha}                  (refs/forge/aborted-turns; recover with `git stash apply {sha}`)"
            ),
            None => work,
        }
    }

    /// End the turn for the per-turn input-token ceiling. Returns the final text to adopt.
    pub(crate) fn abort_for_token_ceiling(&mut self) -> String {
        let cap = self.config.mesh.max_turn_input_tokens;
        let work = self.preserve_uncommitted_work();
        self.presenter.emit(PresenterEvent::Error(format!(
            "ERROR: turn input-token ceiling exceeded (cap {cap}, billable input {}, total input \
             {} incl. cache reads, output {}) — ending turn; raise `mesh.max_turn_input_tokens` \
             for longer turns; work is uncommitted: {work}",
            self.turn_billable_input_tokens, self.turn_input_tokens, self.turn_output_tokens
        )));
        self.turn_hard_guard_abort = true;
        self.turn_forced_stop = Some(StopReason::BudgetExhausted);
        format!("ERROR: turn input-token ceiling exceeded; work is uncommitted: {work}")
    }

    /// End an unattended turn that reached the explicit step limit the user set.
    pub(crate) fn abort_for_step_ceiling(&mut self, limit: usize) -> String {
        let work = self.preserve_uncommitted_work();
        // Print BOTH input figures. The one that looks alarming — a 48.5M turn — is the
        // cache-inclusive total, while `mesh.max_turn_input_tokens` watches the billable subtotal;
        // showing only the former makes a working token ceiling look broken.
        self.presenter.emit(PresenterEvent::Error(format!(
            "ERROR: unattended turn reached the {limit}-step limit (`mesh.max_steps_unattended` / \
             `--max-turns`; turn tokens: billable input={}, total input={} incl. cache reads, \
             output={}) — ending turn; work is uncommitted: {work}",
            self.turn_billable_input_tokens, self.turn_input_tokens, self.turn_output_tokens,
        )));
        self.turn_hard_guard_abort = true;
        format!(
            "ERROR: unattended turn reached the {limit}-step limit; work is uncommitted: {work}"
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
        let _ = self.push_note(forge_types::MessageKind::Continue, nudge);
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
            self.preserve_uncommitted_work()
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
