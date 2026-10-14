//! `[mesh] harness_profile` in the loop (docs/features/harness-profile.md).
//!
//! `minimal` silences the advisory nudges: the silence guard's progress ask (the user still sees
//! the progress line), the stall/stagnation nudges (the halt that follows a relapse still fires),
//! git-hygiene commit/push reminders, the auto-orchestrate prompt, the mid-intent nudge and the
//! direct-path continue nudge. It never touches the permission broker, the doom-loop and
//! failure-loop halts, the phantom-edit gate, the completion gate, the step cap or the spend
//! guards. The profile follows the model actually driving the turn, so a failover onto a model
//! with a different profile changes it at the next step.

impl crate::Session {
    pub(crate) fn note_harness_model(&mut self, model: &str) {
        self.harness_profile = self.config.mesh.harness_profile_for(model);
    }

    pub(crate) fn advisory_nudges_on(&self) -> bool {
        self.harness_profile.advisory_nudges()
    }

    pub(crate) fn commit_nudges_on(&self) -> bool {
        self.config.git.commit_nudge
            && self.config.git.commit_policy.nudges()
            && self.advisory_nudges_on()
    }
}
