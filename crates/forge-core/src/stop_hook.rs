//! Policy for `Stop` hooks that keep a turn going (Claude-Code parity: exit 2 or
//! `{"decision":"block","reason":…}` means "don't stop yet; here is your next instruction").
//!
//! The decision is pure so it can be tested without a model: [`StopHookGate`] counts consecutive
//! blocks, resets the count when a continuation did real work (a tool ran, as Claude Code does),
//! and caps a hook that never approves so it cannot wedge the turn.

use crate::hooks::LifecycleOutcome;

/// What to do with one `Stop` hook outcome.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StopVerdict {
    /// End the turn.
    Stop,
    /// Re-drive the model with this reason as the next instruction (`n` of `max`).
    Continue { reason: String, n: u32, max: u32 },
    /// The hook blocked again but the cap is reached: end the turn anyway.
    Capped { reason: String, max: u32 },
}

#[derive(Debug)]
pub(crate) struct StopHookGate {
    blocks: u32,
    max: u32,
}

impl StopHookGate {
    pub(crate) fn new(max: u32) -> Self {
        Self { blocks: 0, max }
    }

    /// `stop_hook_active` for the hook's stdin: true once this turn is already a continuation.
    pub(crate) fn active(&self) -> bool {
        self.blocks > 0
    }

    /// Judge a hook outcome. `can_continue` is false when the turn is ending for a reason a hook
    /// must not override (loop guard, hard guard, turn deadline).
    pub(crate) fn judge(&mut self, outcome: &LifecycleOutcome, can_continue: bool) -> StopVerdict {
        let Some(reason) = outcome.blocked.clone() else {
            return StopVerdict::Stop;
        };
        if outcome.halt || !can_continue {
            return StopVerdict::Stop;
        }
        if self.blocks >= self.max {
            return StopVerdict::Capped {
                reason,
                max: self.max,
            };
        }
        self.blocks += 1;
        StopVerdict::Continue {
            reason,
            n: self.blocks,
            max: self.max,
        }
    }

    /// A continuation made progress (tools ran): a later block is a fresh request, not a spin.
    pub(crate) fn note_progress(&mut self, tools_ran: u64) {
        if tools_ran > 0 {
            self.blocks = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(reason: &str) -> LifecycleOutcome {
        LifecycleOutcome {
            blocked: Some(reason.into()),
            ..Default::default()
        }
    }

    #[test]
    fn approval_stops() {
        let mut g = StopHookGate::new(3);
        assert_eq!(
            g.judge(&LifecycleOutcome::default(), true),
            StopVerdict::Stop
        );
    }

    #[test]
    fn blocks_continue_until_the_cap_then_force_stop() {
        let mut g = StopHookGate::new(2);
        assert!(!g.active());
        assert!(matches!(
            g.judge(&blocked("a"), true),
            StopVerdict::Continue { n: 1, .. }
        ));
        assert!(g.active());
        assert!(matches!(
            g.judge(&blocked("a"), true),
            StopVerdict::Continue { n: 2, .. }
        ));
        assert_eq!(
            g.judge(&blocked("a"), true),
            StopVerdict::Capped {
                reason: "a".into(),
                max: 2
            }
        );
    }

    #[test]
    fn tool_progress_resets_the_count() {
        let mut g = StopHookGate::new(1);
        assert!(matches!(
            g.judge(&blocked("a"), true),
            StopVerdict::Continue { .. }
        ));
        g.note_progress(0);
        assert!(matches!(
            g.judge(&blocked("a"), true),
            StopVerdict::Capped { .. }
        ));
        g.note_progress(2);
        assert!(matches!(
            g.judge(&blocked("a"), true),
            StopVerdict::Continue { n: 1, .. }
        ));
    }

    #[test]
    fn halt_and_uncontinuable_turns_override_a_block() {
        let mut g = StopHookGate::new(8);
        let mut o = blocked("a");
        o.halt = true;
        assert_eq!(g.judge(&o, true), StopVerdict::Stop);
        assert_eq!(g.judge(&blocked("a"), false), StopVerdict::Stop);
    }
}
