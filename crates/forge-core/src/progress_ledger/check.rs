//! Checks (tests, builds, linters) across a turn: when a failing one passing is progress, and
//! when failing the same way again and again is not.
//!
//! Edits between runs of a check say nothing about whether the turn is getting anywhere; the
//! check's verdict does. Ten different edits followed by ten identical failures is a model
//! guessing, however new each guess is. And a check that flips between failing and passing
//! because the model breaks and repairs the same thing is not progress each time it passes, only
//! the first time that failure was fixed.

use crate::loop_progress::CallObservation;
use std::collections::{HashMap, HashSet};

/// A check family failing with the same output, run after run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Stall {
    pub(super) family: &'static str,
    failure: u64,
    pub(super) runs: u32,
}

#[derive(Debug, Default)]
pub(super) struct CheckWatch {
    /// What each family's latest failure looked like.
    failing: HashMap<&'static str, u64>,
    /// (family, failure) pairs that were fixed once this turn.
    fixed: HashSet<(&'static str, u64)>,
    stall: Option<Stall>,
}

impl CheckWatch {
    /// Fold one step's calls in. Returns whether a check passed that had failed in a way not
    /// fixed before this turn: the only flip that is progress.
    pub(super) fn observe(&mut self, calls: &[CallObservation]) -> bool {
        let mut fresh = false;
        for call in calls {
            let Some(family) = call.check() else {
                // A check inside a batched command: the verification ledger saw it flip but
                // there is no output of its own to compare.
                fresh |= call.check_flip();
                continue;
            };
            if call.ok() {
                if let Some(failure) = self.failing.remove(family) {
                    fresh |= self.fixed.insert((family, failure));
                }
                if self.stall.is_some_and(|s| s.family == family) {
                    self.stall = None;
                }
                continue;
            }
            let failure = call.failure_fp();
            self.failing.insert(family, failure);
            self.stall = Some(match self.stall {
                Some(s) if s.family == family && s.failure == failure => Stall {
                    runs: s.runs + 1,
                    ..s
                },
                _ => Stall {
                    family,
                    failure,
                    runs: 1,
                },
            });
        }
        fresh
    }

    pub(super) fn stall(&self) -> Option<Stall> {
        self.stall
    }
}
