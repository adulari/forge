//! Adaptive reasoning effort (docs/features/adaptive-effort.md): after a tool round in which every
//! call was provably read-only and succeeded, the next step runs at low effort. Reading the next
//! file or re-running a search rarely needs deep reasoning. A failure, a write or a user turn
//! restores the configured effort.
//!
//! Lowering only happens once a response in this turn reported reasoning tokens (the
//! `usage.reasoning_tokens` split): a model that reports none either does not reason or cannot
//! show the saving, so there is nothing to cut and nothing to measure. The evidence holds for the
//! rest of the turn, so a low-effort reply that happens to report zero does not flip the next
//! request back up and down again (each flip is a request whose non-input fields changed, which
//! on codex-oauth's WebSocket chain means a full resend).

use forge_types::{EffortLevel, Usage};

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct AdaptiveEffort {
    /// The last tool round was all read-only and all succeeded.
    after_readonly_round: bool,
    /// A response in this turn reported reasoning tokens.
    reasoning_seen: bool,
}

impl AdaptiveEffort {
    pub(crate) fn observe_response(&mut self, usage: &Usage) {
        self.reasoning_seen |= usage.reasoning_tokens.is_some_and(|n| n > 0);
    }

    /// A new user turn: the configured effort, and the reasoning evidence starts over.
    pub(crate) fn begin_turn(&mut self) {
        *self = Self::default();
    }

    /// Record one executed tool round. `readonly` = every call was provably side-effect free,
    /// `ok` = every call succeeded. An empty round says nothing and restores.
    pub(crate) fn observe_round(&mut self, calls: usize, readonly: bool, ok: bool) {
        self.after_readonly_round = calls > 0 && readonly && ok;
    }

    /// A steer or a step with no tool call: back to the configured effort.
    pub(crate) fn restore(&mut self) {
        self.after_readonly_round = false;
    }

    /// The effort to send. `enabled` is `[mesh] adaptive_effort` for this model's provider and
    /// `exact` an `/effort exact` instruction, which is never overridden.
    pub(crate) fn effort_for(
        &self,
        enabled: bool,
        exact: bool,
        configured: Option<EffortLevel>,
    ) -> Option<EffortLevel> {
        let lower = enabled
            && !exact
            && self.after_readonly_round
            && self.reasoning_seen
            && configured != Some(EffortLevel::Low);
        if lower {
            Some(EffortLevel::Low)
        } else {
            configured
        }
    }
}

impl super::Session {
    /// The effort for the next request to `model`, given the effort routing chose.
    pub(crate) fn adaptive_effort_for(
        &self,
        model: &str,
        configured: Option<EffortLevel>,
    ) -> Option<EffortLevel> {
        let effort = self.adaptive_effort.effort_for(
            self.config.mesh.adaptive_effort_for(model),
            self.exact_effort.is_some(),
            configured,
        );
        if effort != configured {
            tracing::debug!(
                model,
                ?configured,
                "adaptive effort: low after a read-only round"
            );
        }
        effort
    }

    /// Fold one executed tool round into the adaptive-effort state.
    pub(crate) fn observe_adaptive_round(
        &mut self,
        calls: &[forge_types::ToolCall],
        observations: &[crate::loop_progress::CallObservation],
    ) {
        let readonly = calls.iter().all(|c| self.is_concurrent_readonly(c));
        let ok = observations.iter().all(|o| o.ok());
        self.adaptive_effort
            .observe_round(calls.len(), readonly, ok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reasoned(n: Option<u64>) -> AdaptiveEffort {
        let mut s = AdaptiveEffort::default();
        s.observe_response(&Usage {
            reasoning_tokens: n,
            ..Usage::default()
        });
        s
    }

    #[test]
    fn a_successful_readonly_round_drops_to_low() {
        let mut s = reasoned(Some(900));
        s.observe_round(2, true, true);
        assert_eq!(
            s.effort_for(true, false, Some(EffortLevel::High)),
            Some(EffortLevel::Low)
        );
        assert_eq!(s.effort_for(true, false, None), Some(EffortLevel::Low));
    }

    #[test]
    fn a_write_a_failure_or_a_user_turn_restores() {
        let high = Some(EffortLevel::High);
        let mut s = reasoned(Some(900));
        s.observe_round(1, false, true);
        assert_eq!(s.effort_for(true, false, high), high, "write");
        s.observe_round(1, true, false);
        assert_eq!(s.effort_for(true, false, high), high, "failure");
        s.observe_round(1, true, true);
        s.restore();
        assert_eq!(s.effort_for(true, false, high), high, "steer");
        s.observe_round(1, true, true);
        s.begin_turn();
        assert_eq!(s.effort_for(true, false, high), high, "user turn");
        s.observe_round(0, true, true);
        assert_eq!(s.effort_for(true, false, high), high, "no tools ran");
    }

    #[test]
    fn it_needs_the_switch_reported_reasoning_and_no_exact_pin() {
        let high = Some(EffortLevel::High);
        let mut s = reasoned(Some(900));
        s.observe_round(1, true, true);
        assert_eq!(
            s.effort_for(false, false, high),
            high,
            "provider not enabled"
        );
        assert_eq!(s.effort_for(true, true, high), high, "/effort exact");
        for silent in [None, Some(0)] {
            let mut s = reasoned(silent);
            s.observe_round(1, true, true);
            assert_eq!(s.effort_for(true, false, high), high, "{silent:?}");
        }
    }

    #[test]
    fn reasoning_evidence_holds_for_the_turn_and_resets_with_it() {
        let high = Some(EffortLevel::High);
        let mut s = reasoned(Some(900));
        s.observe_round(1, true, true);
        s.observe_response(&Usage::default());
        assert_eq!(
            s.effort_for(true, false, high),
            Some(EffortLevel::Low),
            "a silent low-effort reply does not flip back up"
        );
        s.begin_turn();
        s.observe_round(1, true, true);
        assert_eq!(
            s.effort_for(true, false, high),
            high,
            "a new turn needs new evidence"
        );
    }
}
