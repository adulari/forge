//! When the ledger speaks: which lane fires, when a nudge becomes a stop, and what the report
//! says. Split from the bookkeeping in the parent module, which feeds it the counters.

use super::*;

impl ProgressLedger {
    pub(super) fn waiting_nudge_due(&self) -> bool {
        self.wait_nudge > 0 && self.wait_steps >= self.wait_nudge
    }

    pub(super) fn nudge_lane(&self, scope: Scope) -> Option<Lane> {
        if scope == Scope::Waiting {
            return self.waiting_nudge_due().then_some(Lane::Waiting);
        }
        if self.cycle_hold >= 1 {
            Some(Lane::Cycle)
        } else if self
            .failure
            .as_ref()
            .is_some_and(|f| f.run >= FAILURE_NUDGE_AT)
        {
            Some(Lane::Failure)
        } else if self
            .checks
            .stall()
            .is_some_and(|s| s.runs >= CHECK_NUDGE_AT)
        {
            Some(Lane::Check)
        } else if self.narration_repeats() >= NARRATION_NUDGE_AT {
            Some(Lane::Narration)
        } else if self.futile_run >= self.futile_nudge {
            Some(Lane::Futile)
        } else if self.waiting_nudge_due() {
            Some(Lane::Waiting)
        } else if self.idle_nudge > 0 && self.idle_steps >= self.idle_nudge {
            Some(Lane::Idle)
        } else {
            None
        }
    }

    pub(super) fn waiting_stop_due(&self) -> bool {
        self.headless
            && self.idle_stop > 0
            && self.wait_stop > 0
            && self.wait_steps >= self.wait_stop
    }

    pub(super) fn stop_lane(&self, scope: Scope) -> Option<Lane> {
        if scope == Scope::Waiting {
            return self.waiting_stop_due().then_some(Lane::Waiting);
        }
        if self.cycle_hold >= self.cycle_stop_after {
            Some(Lane::Cycle)
        } else if self
            .failure
            .as_ref()
            .is_some_and(|f| f.run >= FAILURE_STOP_AT)
        {
            Some(Lane::Failure)
        } else if self.checks.stall().is_some_and(|s| s.runs >= CHECK_STOP_AT) {
            Some(Lane::Check)
        } else if matches!(self.episode, Episode::Nudged { lane: Lane::Narration, relapses, .. } if relapses >= RELAPSE_STEPS)
        {
            Some(Lane::Narration)
        } else if self.futile_run >= self.futile_stop {
            Some(Lane::Futile)
        } else if self.waiting_stop_due() {
            Some(Lane::Waiting)
        } else if self.headless && self.idle_stop > 0 && self.idle_steps >= self.idle_stop {
            Some(Lane::Idle)
        } else {
            None
        }
    }

    pub(super) fn decide(&mut self, facts: StepFacts, scope: Scope) -> Verdict {
        let relapse_now = self
            .window
            .back()
            .is_some_and(|r| r.stale || (r.restated && !r.fresh_read));
        let nudge_lane = self.nudge_lane(scope);
        match self.episode {
            Episode::Quiet => match nudge_lane {
                Some(lane) => {
                    self.episode = Episode::Nudged {
                        lane,
                        since: 0,
                        relapses: 0,
                    };
                    self.quiet_steps = 0;
                    Verdict::Nudge(self.report(lane))
                }
                None => Verdict::Fine,
            },
            Episode::Nudged {
                lane,
                since,
                relapses,
            } => {
                let since = since + 1;
                let relapses = if lane == Lane::Narration
                    && since <= RELAPSE_WINDOW
                    && relapse_now
                    && scope == Scope::All
                {
                    relapses + 1
                } else {
                    relapses
                };
                self.episode = Episode::Nudged {
                    lane,
                    since,
                    relapses,
                };
                if lane == Lane::Narration && since >= RELAPSE_WINDOW && relapses < RELAPSE_STEPS {
                    // It took the hint: new information kept arriving. Judge afresh from here.
                    self.window.clear();
                }
                if since >= MIN_STEPS_AFTER_NUDGE && !self.stop_suppressed {
                    if let Some(stop) = self.stop_lane(scope) {
                        self.stop_candidate = Some(facts);
                        return Verdict::Stop(self.report(stop));
                    }
                }
                if scope == Scope::All {
                    if self.nudge_lane(scope).is_none() {
                        self.quiet_steps += 1;
                        if self.quiet_steps >= QUIET_STEPS_TO_CLOSE {
                            self.episode = Episode::Quiet;
                            self.quiet_steps = 0;
                            self.stop_suppressed = false;
                        }
                    } else {
                        self.quiet_steps = 0;
                    }
                }
                Verdict::Fine
            }
        }
    }

    pub(super) fn report(&self, lane: Lane) -> StallReport {
        let steps = match lane {
            Lane::Narration => self.window.iter().filter(|r| r.stale).count() as u32,
            Lane::Futile => self.futile_run,
            Lane::Failure => self.failure.as_ref().map_or(0, |f| f.run),
            Lane::Cycle => self.cycle_run.max(self.recent_batches.len() as u32),
            Lane::Check => self.checks.stall().map_or(0, |s| s.runs),
            Lane::Waiting => self.wait_steps,
            Lane::Idle => self.idle_steps,
            Lane::Resumed => self.resume_steps,
        };
        let failing = self
            .failure
            .as_ref()
            .filter(|f| f.run >= 2)
            .map(|f| (f.tool.clone(), f.head.clone(), f.run));
        StallReport {
            lane,
            steps,
            last_progress: self.last_progress.clone(),
            repeated: self.most_repeated(),
            failing,
            check: self.checks.stall().map(|s| (s.family, s.runs)),
        }
    }

    /// The call that came back with the same result most often lately, if it did so at least
    /// three times.
    pub(super) fn most_repeated(&self) -> Option<(String, u32)> {
        let mut counts: HashMap<&(String, u64), u32> = HashMap::new();
        for entry in &self.recent_calls {
            *counts.entry(entry).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .filter(|((label, _), times)| !label.is_empty() && *times >= 3)
            .max_by_key(|((label, _), times)| (*times, std::cmp::Reverse(label.clone())))
            .map(|((label, _), times)| (label.clone(), times))
    }
}
