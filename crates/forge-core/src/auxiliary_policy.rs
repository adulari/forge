//! Best-effort session side-call policy.
//!
//! This owner keeps related session invariants together behind the Session program.

use super::*;

pub(crate) fn report_auxiliary_persistence_failure(
    session_id: &str,
    purpose: &str,
    error: &dyn std::fmt::Display,
    emit: impl FnOnce(String),
) {
    tracing::warn!(
        session_id,
        purpose,
        error = %error,
        "failed to persist auxiliary call result"
    );
    emit(format!(
        "auxiliary {purpose} result was not persisted: {error}"
    ));
}

impl Session {
    /// Mark this session as serving one invocation that exits once its turn is answered
    /// (`forge run`). Its post-turn side calls are skipped: nobody sees a recap or a suggestion
    /// after the process is gone, and a memory capture would hold the exit for a model round-trip
    /// (`mesh.auto_memory_one_shot = true` keeps the capture).
    pub fn set_one_shot(&mut self, one_shot: bool) {
        self.one_shot = one_shot;
    }

    /// Optional side calls after a turn whose result has already been emitted. A channel-backed
    /// surface detaches them (model choice included, see `post_turn_calls`), so input frees at
    /// once and their results land a moment later; a synchronous surface runs them inline after
    /// the answer is shown; a one-shot run skips them.
    pub(crate) async fn run_post_turn_side_calls(
        &mut self,
        prompt: &str,
        final_text: &str,
        tasks_before: &[forge_types::TodoItem],
    ) {
        // A tripped session spend cap ends the turn; optional side calls must not keep spending.
        let spend_ok = self.session_spend_refusal().is_none();
        let detached = self.presenter.recap_sink().is_some();
        if spend_ok && !self.one_shot {
            if detached {
                self.spawn_post_turn_calls(prompt, final_text, tasks_before);
            } else {
                self.generate_recap(prompt, final_text, tasks_before).await;
                self.generate_suggestion(prompt, final_text).await;
            }
        }
        // Continual Harness auto-refine gate (`harness.auto_refine = "turns"`, refinement.rs).
        // Inline, not detached: it mutates durable harness state and its own presenter note should
        // land in order with the turn it concluded, not race a later turn's output.
        self.auto_refine_after_turns().await;
        // The detached batch above already carries the memory capture.
        let captured_by_detached_batch = detached && !self.one_shot;
        if !spend_ok
            || captured_by_detached_batch
            || (self.one_shot && !self.config.mesh.auto_memory_one_shot)
        {
            return;
        }
        // Dropping the JoinHandle detaches (does not cancel) the capture; a synchronous surface
        // awaits it so persistence has finished when the turn returns.
        if let Some(handle) = self.capture_memories(prompt, final_text).await {
            if !detached {
                let _ = handle.await;
            }
        }
    }
}
