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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_baseline_the_first_sample_only_sets_one() {
        let mut watch = TreeWatch::default();
        assert!(!watch.is_new_state(&mut || Some(7)));
        assert!(watch.is_new_state(&mut || Some(8)));
        assert!(!watch.is_new_state(&mut || Some(7)), "a state already seen");
    }

    #[test]
    fn a_write_the_model_made_explains_the_next_sample_and_only_that_one() {
        let mut watch = TreeWatch::default();
        watch.rebase(Some(1));
        watch.wrote();
        assert!(!watch.is_new_state(&mut || Some(2)));
        assert!(watch.is_new_state(&mut || Some(3)));
    }

    #[test]
    fn a_slow_sample_makes_the_next_few_skip_and_a_missing_tree_is_no_evidence() {
        let mut watch = TreeWatch::default();
        watch.rebase(Some(1));
        let slow = &mut || {
            std::thread::sleep(SLOW_SAMPLE + Duration::from_millis(50));
            Some(2)
        };
        assert!(watch.is_new_state(slow));
        for _ in 0..SKIP_AFTER_SLOW {
            assert!(!watch
                .is_new_state(&mut || -> Option<u64> { panic!("skipped samples never probe") }));
        }
        assert!(watch.is_new_state(&mut || Some(3)));
        assert!(!watch.is_new_state(&mut || None));
    }

    #[test]
    fn a_very_long_turn_forgets_old_states_rather_than_growing_without_bound() {
        let mut watch = TreeWatch::default();
        watch.rebase(Some(0));
        for state in 1..=(MAX_STATES as u64 * 2) {
            watch.is_new_state(&mut || Some(state));
        }
        assert!(watch.seen.len() <= MAX_STATES);
    }
}
