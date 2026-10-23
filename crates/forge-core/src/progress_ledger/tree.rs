//! Whether shell-driven work changed the working tree in a way not seen before this turn.
//!
//! A silent `sed -i`, a `git commit` or a generator script looks like nothing to a ledger that
//! reads results: the result is the same `exit 0` every time. The working tree is where such work
//! shows. It is sampled after a step whose calls could have changed it unseen, and a state that no
//! earlier sample had is progress. A state seen before is not: a file that flips between two
//! contents is in a known state from the second visit on, however many times it moves, which is
//! what separates oscillating from working.

use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Distinct tree states remembered; past this the set is dropped and rebuilt.
const MAX_STATES: usize = 4096;
/// A sample slower than this makes the next few samples skip, so a huge repository costs the
/// turn a bounded share of its time.
const SLOW_SAMPLE: Duration = Duration::from_millis(400);
const SKIP_AFTER_SLOW: u32 = 8;

#[derive(Debug, Default)]
pub(super) struct TreeWatch {
    seen: HashSet<u64>,
    /// A recognised write ran since the last sample: the tree moved for a known reason, so the
    /// next sample only moves the baseline.
    wrote: bool,
    started: bool,
    skip: u32,
}

impl TreeWatch {
    /// The tree as it is now, with nothing in flight: a turn or a pass of it is starting.
    pub(super) fn rebase(&mut self, state: Option<u64>) {
        self.wrote = false;
        if let Some(state) = state {
            self.remember(state);
            self.started = true;
        }
    }

    /// A file tool changed a file.
    pub(super) fn wrote(&mut self) {
        self.wrote = true;
    }

    /// Sample the tree; whether it is in a state no earlier sample was in.
    pub(super) fn is_new_state(&mut self, probe: &mut dyn FnMut() -> Option<u64>) -> bool {
        if self.skip > 0 {
            self.skip -= 1;
            return false;
        }
        let began = Instant::now();
        let sampled = probe();
        if began.elapsed() > SLOW_SAMPLE {
            self.skip = SKIP_AFTER_SLOW;
        }
        let Some(state) = sampled else {
            return false;
        };
        let unseen = self.remember(state);
        // Without a baseline, or after a write the model made, the first sample explains nothing.
        let explained = std::mem::take(&mut self.wrote) || !self.started;
        self.started = true;
        unseen && !explained
    }

    fn remember(&mut self, state: u64) -> bool {
        if self.seen.len() >= MAX_STATES {
            self.seen.clear();
        }
        self.seen.insert(state)
    }
}
