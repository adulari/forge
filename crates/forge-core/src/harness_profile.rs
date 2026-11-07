//! `[mesh] harness_profile` in the loop (docs/features/harness-profile.md).
//!
//! `minimal` silences the advisory nudges: the silence guard's progress ask (the user still sees
//! the progress line), the stall/stagnation nudges (the halt that follows a relapse still fires),
//! git-hygiene commit/push reminders and the auto-orchestrate prompt. It never touches the
//! permission broker, the doom-loop and failure-loop halts, the phantom-edit gate, the completion
//! gate, the step cap or the spend guards, and it never silences the stop-integrity gates: a final
//! reply that ends mid-intent (2 re-drives) or leaves tasks open (the continue re-drive) is driven
//! onward under every profile. `[harness] task_nudges = false` is the one switch for the latter. The profile follows the model actually driving the turn, so a failover onto a model
//! with a different profile changes it at the next step.

impl crate::Session {
    pub(crate) fn note_harness_model(&mut self, model: &str) {
        self.harness_profile = self.config.mesh.harness_profile_for(model);
        self.harness_flags = self.config.harness.flags_for(model);
        self.harness_model = model.to_string();
    }

    pub(crate) fn advisory_nudges_on(&self) -> bool {
        self.harness_profile.advisory_nudges()
    }

    /// Whether open tasks drive the model onward (the continue re-drive, the stale-task escalation
    /// and question, the turn-start list note). `[harness] task_nudges` decides; unset keeps them
    /// on in every profile, because ending a turn with the plan half done is a stop-integrity
    /// failure, not advisory chatter.
    pub(crate) fn task_nudges_on(&self) -> bool {
        self.config.harness.task_nudges.unwrap_or(true)
    }

    /// Whether the model is given the `update_tasks` tool (`[harness] task_tool`).
    pub(crate) fn task_tool_on(&self) -> bool {
        self.config.harness.task_tool
    }

    pub(crate) fn commit_nudges_on(&self) -> bool {
        self.config.git.commit_nudge && self.commit_policy().nudges() && self.advisory_nudges_on()
    }
}
