//! The per-turn progress ledger: one place that decides whether a turn is getting anywhere.
//!
//! A step count says nothing about that. A hundred steps that edit, test and close tasks are a
//! healthy turn; ten steps that re-read the same file are not. The ledger is owned by the
//! [`crate::Session`] and lives for the whole turn, across every re-drive pass, and judges each
//! finished tool step from what it produced:
//!
//! * **Strong** progress resets everything: a successful file write with arguments not tried
//!   this turn, a tracked task closing, a failing check passing, or the person steering.
//! * **Informative** steps showed at least a quarter of result lines not seen this turn. They
//!   only break a run of futile steps.
//! * **Neutral** steps are not evidence either way: a poll-mode wait that was not ready yet, and
//!   the first step after a harness re-drive (a verify or completeness gate made the model look
//!   again; that is the harness asking, not the model looping).
//! * Everything else is **stale** (nothing changed, nothing new) or **all-failed**.
//!
//! Five lanes watch the stale evidence (narration, futile, failure, cycle and idle, see the
//! constants below) under one latch: one nudge per episode, whichever lane fires first, and a
//! stop only after that nudge, at least [`MIN_STEPS_AFTER_NUDGE`] steps later, once a lane's own
//! stop bar is reached. A turn that edits, closes tasks or passes checks is never touched, and
//! the idle lane, the only one that can stop a read-only turn of novel output, is generous and
//! stops only unattended turns.
//!
//! Pure data in, verdict out; [`crate::progress_supervisor`] is the part that touches the
//! transcript and the presenter.

use crate::loop_progress::CallObservation;
use crate::progress_report::{Lane, StallReport};
use crate::stall_guard::{opening_fingerprint, NarrationTracker};
use forge_types::MessageKind;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};

/// Steps remembered for the narration judgement.
const WINDOW_STEPS: usize = 10;
/// Stale narration repeats (restatements on steps with nothing new) within the window before the
/// model is nudged to answer. The third repeat is the fourth time the sentence is said.
pub(crate) const NARRATION_NUDGE_AT: usize = 3;
/// Relapsing steps (nothing new, or the same opening again) tolerated after a narration nudge
/// before the turn ends, counted over the [`RELAPSE_WINDOW`] steps that follow it.
pub(crate) const RELAPSE_STEPS: u32 = 2;
const RELAPSE_WINDOW: u32 = 4;
/// Consecutive stale or all-failed steps before the nudge and before the stop.
pub(crate) const FUTILE_NUDGE_AT: u32 = 5;
pub(crate) const FUTILE_STOP_AT: u32 = 10;
/// Consecutive steps in which every call failed, before the nudge and before the stop.
pub(crate) const FAILURE_NUDGE_AT: u32 = 3;
pub(crate) const FAILURE_STOP_AT: u32 = 6;
/// The same batch with the same result this many times in a row is a cycle; so is a window of
/// [`CYCLE_WINDOW`] steps with at most two distinct batches, one recurring this many times.
const CYCLE_AT: u32 = 3;
const CYCLE_WINDOW: usize = 6;
/// Steps for which a cycle has held, counting the one that nudged, before the stop: the nudge,
/// then two further repeats.
pub(crate) const CYCLE_STOP_AFTER: u32 = 3;
/// A stop needs at least this many steps after the nudge to let the model react to it.
pub(crate) const MIN_STEPS_AFTER_NUDGE: u32 = 2;
/// Consecutive steps with every lane under its nudge line that close an episode.
const QUIET_STEPS_TO_CLOSE: u32 = 4;
/// Steps after a stop in which a turn that repeats the stopped call or opening is stopped again
/// at once, with no second nudge.
const RESUME_WATCH_STEPS: u32 = 4;
/// Identical harness notes of one kind admitted per turn; the next copies are dropped.
const NOTE_REPEAT_LIMIT: u32 = 3;
/// Seen-line set size past which it is dropped and rebuilt; bounds memory on a very long turn
/// without ever leaving the ledger unable to judge (it just re-learns).
const MAX_SEEN_LINES: usize = 200_000;
/// A result with at least this share (1 in `NEW_LINE_DIVISOR`) of lines unseen is informative.
const NEW_LINE_DIVISOR: usize = 4;
pub(crate) const DEFAULT_IDLE_NUDGE_STEPS: u32 = 60;
pub(crate) const DEFAULT_IDLE_STOP_STEPS: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Fine,
    /// Tell the model once; the episode is now latched.
    Nudge(StallReport),
    /// End the turn cleanly.
    Stop(StallReport),
}

/// How a finished step is remembered in the sliding window.
#[derive(Debug, Clone, Copy)]
struct StepRecord {
    /// Nothing in it was new: no successful new mutation and under a quarter of the result lines
    /// unseen.
    stale: bool,
    /// Its opening restated one of the model's recent openings.
    restated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Episode {
    Quiet,
    Nudged {
        lane: Lane,
        /// Steps since the nudge, and how many of the first [`RELAPSE_WINDOW`] repeated
        /// themselves (stale or the same opening again).
        since: u32,
        relapses: u32,
    },
}

#[derive(Debug, Clone, Copy)]
struct LastStop {
    batch_sig: u64,
    opening: Option<u64>,
}

#[derive(Debug, Clone)]
struct FailureRun {
    tool: String,
    cause: String,
    run: u32,
}

#[derive(Debug)]
pub(crate) struct ProgressLedger {
    narration: NarrationTracker,
    seen_signatures: HashSet<u64>,
    seen_lines: HashSet<u64>,
    window: VecDeque<StepRecord>,
    /// Tool steps observed this turn, and model calls begun (the explicit step limit counts
    /// those, across every re-drive pass).
    step_no: u32,
    model_calls: usize,
    futile_run: u32,
    failure: Option<FailureRun>,
    recent_batches: VecDeque<(u64, u64)>,
    cycle_run: u32,
    cycle_hold: u32,
    idle_steps: u32,
    last_progress: Option<(u32, String)>,
    /// The most repeated (call label, result) among the recent steps, for the report.
    recent_calls: VecDeque<(String, u64)>,
    episode: Episode,
    quiet_steps: u32,
    stop_suppressed: bool,
    nudge_tree: Option<u64>,
    gate_pending: bool,
    steer_pending: bool,
    last_stop: Option<LastStop>,
    stop_candidate: Option<LastStop>,
    resume_steps: u32,
    note_counts: HashMap<(MessageKind, u64), u32>,
    idle_nudge: u32,
    idle_stop: u32,
    unattended: bool,
}

impl Default for ProgressLedger {
    fn default() -> Self {
        Self {
            narration: NarrationTracker::default(),
            seen_signatures: HashSet::new(),
            seen_lines: HashSet::new(),
            window: VecDeque::new(),
            step_no: 0,
            model_calls: 0,
            futile_run: 0,
            failure: None,
            recent_batches: VecDeque::new(),
            cycle_run: 0,
            cycle_hold: 0,
            idle_steps: 0,
            last_progress: None,
            recent_calls: VecDeque::new(),
            episode: Episode::Quiet,
            quiet_steps: 0,
            stop_suppressed: false,
            nudge_tree: None,
            gate_pending: false,
            steer_pending: false,
            last_stop: None,
            stop_candidate: None,
            resume_steps: 0,
            note_counts: HashMap::new(),
            idle_nudge: DEFAULT_IDLE_NUDGE_STEPS,
            idle_stop: DEFAULT_IDLE_STOP_STEPS,
            unattended: false,
        }
    }
}

impl ProgressLedger {
    /// The idle thresholds (`0` disables) and whether nobody is present to see the turn. Cheap,
    /// called before every judgement so a config change or a mode switch takes effect at once.
    pub(crate) fn configure(&mut self, idle_nudge: usize, idle_stop: usize, unattended: bool) {
        self.idle_nudge = u32::try_from(idle_nudge).unwrap_or(u32::MAX);
        self.idle_stop = u32::try_from(idle_stop).unwrap_or(u32::MAX);
        self.unattended = unattended;
    }

    /// A new user turn. What the model has read stays read, but the turn gets a fresh episode and
    /// fresh counters: carrying a pending nudge or a window full of stale steps across turns
    /// halted every later turn after its first step in long sessions, where nearly every read
    /// repeats something seen hours earlier. A loop that really survives the user's message is
    /// still caught within this turn. Argument signatures are forgotten too, so a deliberate redo
    /// of an earlier edit counts as a fresh mutation. What the last stop was on is kept, so a
    /// bare `continue` that resumes the same calls is stopped again at once.
    pub(crate) fn begin_turn(&mut self) {
        self.narration.begin_turn();
        self.seen_signatures.clear();
        self.window.clear();
        self.step_no = 0;
        self.model_calls = 0;
        self.futile_run = 0;
        self.failure = None;
        self.recent_batches.clear();
        self.cycle_run = 0;
        self.cycle_hold = 0;
        self.idle_steps = 0;
        self.last_progress = None;
        self.recent_calls.clear();
        self.episode = Episode::Quiet;
        self.quiet_steps = 0;
        self.stop_suppressed = false;
        self.nudge_tree = None;
        self.gate_pending = false;
        self.steer_pending = false;
        self.stop_candidate = None;
        self.resume_steps = 0;
        self.note_counts.clear();
    }

    /// Count a model call about to start; the number made before it. The explicit step limit
    /// (`mesh.max_steps`, `--max-turns`) is checked against this, so it bounds the turn rather
    /// than each re-drive pass of it.
    pub(crate) fn begin_model_call(&mut self) -> usize {
        let before = self.model_calls;
        self.model_calls += 1;
        before
    }

    pub(crate) fn model_calls(&self) -> usize {
        self.model_calls
    }

    /// A harness gate (verify, completeness, continue) just sent the model back to look again:
    /// the step that follows is the harness's doing, not evidence of a loop.
    pub(crate) fn note_gate(&mut self) {
        self.gate_pending = true;
    }

    /// The person (or a finished background job) added to the conversation mid-turn: the next
    /// step answers new instructions.
    pub(crate) fn note_steer(&mut self) {
        self.steer_pending = true;
    }

    /// The transcript was just compacted: the model no longer remembers what it read, so reading
    /// it again is legitimate. Forget what was seen, but keep the counters and any pending nudge:
    /// a loop that survives a compaction is exactly the case this exists to catch.
    pub(crate) fn note_compaction(&mut self) {
        self.seen_signatures.clear();
        self.seen_lines.clear();
    }

    /// Whether an identical harness note of this kind may be added again. A harness note that
    /// repeats verbatim is the harness talking to itself: the fourth copy is dropped.
    pub(crate) fn admit_note(&mut self, kind: MessageKind, content: &str) -> bool {
        if !matches!(
            kind,
            MessageKind::Hint
                | MessageKind::LoopGuard
                | MessageKind::Continue
                | MessageKind::Silence
        ) {
            return true;
        }
        let count = self
            .note_counts
            .entry((kind, crate::loop_progress::hash_str(content)))
            .or_insert(0);
        *count += 1;
        *count <= NOTE_REPEAT_LIMIT
    }

    /// How hard this turn is currently repeating itself, as a rung on the sampling ladder
    /// (0 = nothing, 1 = restated once, 2 = persistently). Reports the evidence EARLY, while the
    /// turn is still salvageable, so the next request can be shaped differently (a higher
    /// temperature, a repetition penalty) instead of being sent again identically. Only stale
    /// steps count: a productive turn never runs hotter, and neither does a long investigation
    /// that was merely asked to report.
    pub(crate) fn pressure(&self) -> u8 {
        let verbatim = self.narration.max_verbatim();
        let repeats = self.narration_repeats();
        let latched = matches!(self.episode, Episode::Nudged { lane, .. } if lane != Lane::Idle);
        if verbatim >= 3 || latched || repeats >= 2 {
            return 2;
        }
        if verbatim >= 2 || repeats >= 1 {
            return 1;
        }
        0
    }

    /// The working tree when the episode's nudge went out (`None` outside git): a stop is vetoed
    /// when it changed since, because shell-driven work (a silent `sed -i`, a `git commit`) looks
    /// stale to the ledger but is not.
    pub(crate) fn record_nudge_tree(&mut self, fingerprint: Option<u64>) {
        self.nudge_tree = fingerprint;
    }

    pub(crate) fn has_nudge_tree(&self) -> bool {
        self.nudge_tree.is_some()
    }

    pub(crate) fn tree_changed_since_nudge(&self, now: Option<u64>) -> bool {
        matches!((self.nudge_tree, now), (Some(then), Some(now)) if then != now)
    }

    /// The working tree changed after the nudge: that is progress, so start over.
    pub(crate) fn veto_stop(&mut self, what: &str) {
        self.stop_candidate = None;
        self.reset_on_progress(what.to_string());
    }

    /// The turn really ended on the verdict just returned: remember what it was stopped on.
    pub(crate) fn stopped(&mut self) {
        self.last_stop = self.stop_candidate.take();
    }

    /// The stop was reported but not enforced (`progress_guard = warn`/`off`): do not ask again
    /// in this episode.
    pub(crate) fn suppress_stop(&mut self) {
        self.stop_suppressed = true;
        self.stop_candidate = None;
    }

    /// Judge one finished tool step: the narration the model emitted with it, what its calls
    /// returned, and whether a tracked task closed during it.
    pub(crate) fn observe_step(
        &mut self,
        narration: &str,
        calls: &[CallObservation],
        task_closed: bool,
    ) -> Verdict {
        self.step_no += 1;
        let (new_write, informative) = self.assess(calls);
        let steered = std::mem::take(&mut self.steer_pending);
        let flipped = calls.iter().any(CallObservation::check_flip);
        let gated = std::mem::take(&mut self.gate_pending);
        let strong = new_write || flipped || task_closed || steered;
        let waiting_only = !calls.is_empty() && calls.iter().all(CallObservation::waiting);
        if !strong && (gated || waiting_only) {
            return Verdict::Fine;
        }
        let restated = self.narration.observe(narration, strong);
        if strong {
            let what = if new_write {
                calls
                    .iter()
                    .find(|c| c.wrote())
                    .map_or_else(|| "a file change".to_string(), |c| c.label().to_string())
            } else if task_closed {
                "a task closed".to_string()
            } else if flipped {
                "a failing check passed".to_string()
            } else {
                "your message".to_string()
            };
            self.reset_on_progress(what);
            return Verdict::Fine;
        }
        let batch_sig = batch_signature(calls);
        if let Some(last) = self.last_stop {
            self.resume_steps += 1;
            let same_opening =
                last.opening.is_some() && opening_fingerprint(narration) == last.opening;
            if batch_sig == last.batch_sig || same_opening {
                self.stop_candidate = Some(LastStop {
                    batch_sig,
                    opening: opening_fingerprint(narration),
                });
                return Verdict::Stop(self.report(Lane::Resumed));
            }
            if self.resume_steps >= RESUME_WATCH_STEPS {
                self.last_stop = None;
            }
        }
        self.count_stale(calls, informative, restated, batch_sig);
        self.decide(narration, batch_sig, restated)
    }

    fn assess(&mut self, calls: &[CallObservation]) -> (bool, bool) {
        let (mut new_write, mut informative) = (false, false);
        for call in calls {
            let new_signature = self.seen_signatures.insert(call.signature());
            if !call.ok() {
                continue;
            }
            if call.mutating() && new_signature {
                new_write = true;
            }
            if self.seen_lines.len() > MAX_SEEN_LINES {
                self.seen_lines.clear();
            }
            let total = call.lines().len();
            let new = call
                .lines()
                .iter()
                .filter(|l| self.seen_lines.insert(**l))
                .count();
            if new > 0 && new * NEW_LINE_DIVISOR >= total {
                informative = true;
            }
        }
        (new_write, informative)
    }

    fn reset_on_progress(&mut self, what: String) {
        self.window.clear();
        self.futile_run = 0;
        self.failure = None;
        self.recent_batches.clear();
        self.cycle_run = 0;
        self.cycle_hold = 0;
        self.idle_steps = 0;
        self.recent_calls.clear();
        self.episode = Episode::Quiet;
        self.quiet_steps = 0;
        self.stop_suppressed = false;
        self.last_stop = None;
        self.resume_steps = 0;
        self.last_progress = Some((self.step_no, what));
    }

    fn count_stale(
        &mut self,
        calls: &[CallObservation],
        informative: bool,
        restated: bool,
        batch_sig: u64,
    ) {
        let stale = !informative;
        self.window.push_back(StepRecord { stale, restated });
        if self.window.len() > WINDOW_STEPS {
            self.window.pop_front();
        }
        self.futile_run = if informative { 0 } else { self.futile_run + 1 };
        self.idle_steps += 1;

        let all_failed = !calls.is_empty() && calls.iter().all(|c| !c.ok());
        self.failure = if all_failed {
            let tool = single_tool(calls);
            let cause = calls
                .last()
                .and_then(CallObservation::error_head)
                .unwrap_or("")
                .to_string();
            let run = match &self.failure {
                Some(f) if f.tool == tool => f.run + 1,
                _ => 1,
            };
            Some(FailureRun { tool, cause, run })
        } else {
            None
        };

        let key = (batch_sig, combined_result_fp(calls));
        self.cycle_run = if self.recent_batches.back() == Some(&key) {
            self.cycle_run + 1
        } else {
            1
        };
        self.recent_batches.push_back(key);
        if self.recent_batches.len() > CYCLE_WINDOW {
            self.recent_batches.pop_front();
        }
        self.cycle_hold = if self.cycle_run >= CYCLE_AT || self.oscillating() {
            self.cycle_hold + 1
        } else {
            0
        };

        self.recent_calls.push_back((
            calls
                .first()
                .map_or_else(String::new, |c| c.label().to_string()),
            key.1,
        ));
        if self.recent_calls.len() > WINDOW_STEPS {
            self.recent_calls.pop_front();
        }
    }

    /// At most two distinct batches in the window, one recurring [`CYCLE_AT`] times, all of its
    /// occurrences with the same result: A,B,A,B,A,B. Requiring at most two keeps act-then-observe
    /// workflows out (tap, screen, tap, screen: the screen repeats while every action differs).
    fn oscillating(&self) -> bool {
        let distinct: HashSet<u64> = self.recent_batches.iter().map(|(sig, _)| *sig).collect();
        if distinct.len() > 2 {
            return false;
        }
        self.recent_batches.iter().any(|(sig, fp)| {
            let same: Vec<_> = self
                .recent_batches
                .iter()
                .filter(|(other, _)| other == sig)
                .collect();
            same.len() >= CYCLE_AT as usize && same.iter().all(|(_, other_fp)| other_fp == fp)
        })
    }

    fn narration_repeats(&self) -> usize {
        self.window.iter().filter(|r| r.stale && r.restated).count()
    }

    fn nudge_lane(&self) -> Option<Lane> {
        if self.cycle_hold >= 1 {
            Some(Lane::Cycle)
        } else if self
            .failure
            .as_ref()
            .is_some_and(|f| f.run >= FAILURE_NUDGE_AT)
        {
            Some(Lane::Failure)
        } else if self.narration_repeats() >= NARRATION_NUDGE_AT {
            Some(Lane::Narration)
        } else if self.futile_run >= FUTILE_NUDGE_AT {
            Some(Lane::Futile)
        } else if self.idle_nudge > 0 && self.idle_steps >= self.idle_nudge {
            Some(Lane::Idle)
        } else {
            None
        }
    }

    fn stop_lane(&self) -> Option<Lane> {
        if self.cycle_hold >= CYCLE_STOP_AFTER {
            Some(Lane::Cycle)
        } else if self
            .failure
            .as_ref()
            .is_some_and(|f| f.run >= FAILURE_STOP_AT)
        {
            Some(Lane::Failure)
        } else if matches!(self.episode, Episode::Nudged { lane: Lane::Narration, relapses, .. } if relapses >= RELAPSE_STEPS)
        {
            Some(Lane::Narration)
        } else if self.futile_run >= FUTILE_STOP_AT {
            Some(Lane::Futile)
        } else if self.unattended && self.idle_stop > 0 && self.idle_steps >= self.idle_stop {
            Some(Lane::Idle)
        } else {
            None
        }
    }

    fn decide(&mut self, narration: &str, batch_sig: u64, restated: bool) -> Verdict {
        let stale_now = self.window.back().is_some_and(|r| r.stale);
        let nudge_lane = self.nudge_lane();
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
                    && (stale_now || restated)
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
                    if let Some(stop) = self.stop_lane() {
                        self.stop_candidate = Some(LastStop {
                            batch_sig,
                            opening: opening_fingerprint(narration),
                        });
                        return Verdict::Stop(self.report(stop));
                    }
                }
                if self.nudge_lane().is_none() {
                    self.quiet_steps += 1;
                    if self.quiet_steps >= QUIET_STEPS_TO_CLOSE {
                        self.episode = Episode::Quiet;
                        self.quiet_steps = 0;
                        self.stop_suppressed = false;
                    }
                } else {
                    self.quiet_steps = 0;
                }
                Verdict::Fine
            }
        }
    }

    fn report(&self, lane: Lane) -> StallReport {
        let steps = match lane {
            Lane::Narration => self.window.iter().filter(|r| r.stale).count() as u32,
            Lane::Futile => self.futile_run,
            Lane::Failure => self.failure.as_ref().map_or(0, |f| f.run),
            Lane::Cycle => self.cycle_run.max(self.recent_batches.len() as u32),
            Lane::Idle => self.idle_steps,
            Lane::Resumed => self.resume_steps,
        };
        let failing = self
            .failure
            .as_ref()
            .filter(|f| f.run >= 2)
            .map(|f| (f.tool.clone(), f.cause.clone(), f.run));
        StallReport {
            lane,
            steps,
            last_progress: self.last_progress.clone(),
            repeated: self.most_repeated(),
            failing,
        }
    }

    /// The call that came back with the same result most often lately, if it did so at least
    /// three times.
    fn most_repeated(&self) -> Option<(String, u32)> {
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

/// One hash for the whole batch of calls a step made, in order.
fn batch_signature(calls: &[CallObservation]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for call in calls {
        call.signature().hash(&mut h);
    }
    h.finish()
}

fn combined_result_fp(calls: &[CallObservation]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for call in calls {
        call.result_fp().hash(&mut h);
    }
    h.finish()
}

/// The tool every call of a step used, or a placeholder when they differ.
fn single_tool(calls: &[CallObservation]) -> String {
    match calls.first() {
        Some(first) if calls.iter().all(|c| c.tool() == first.tool()) => first.tool().to_string(),
        _ => "several tools".to_string(),
    }
}

#[cfg(test)]
mod tests;
