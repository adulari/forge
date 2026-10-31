//! The per-turn progress ledger: one place that decides whether a turn is getting anywhere.
//!
//! A step count says nothing about that. A hundred steps that edit, test and close tasks are a
//! healthy turn; ten steps that re-read the same file are not. The ledger is owned by the
//! [`crate::Session`] and lives for the whole turn, across every re-drive pass, and judges each
//! finished tool step from what it produced:
//!
//! * **Strong** progress resets everything: a successful file write with arguments not tried
//!   this turn, a tracked task closing, a failing check passing (for the first time that failure
//!   was fixed), the working tree reaching a state it was never in this turn, or the person
//!   steering.
//! * **Informative** steps showed at least a quarter of result lines not seen this turn, read a
//!   path for the first time, or ran a silent command that was new. They only break a run of
//!   futile steps.
//! * **Neutral** steps are not evidence either way: a wait for something outside the turn (a
//!   poll, a running job, a command cut by its time limit) and the first step after a harness
//!   re-drive (a verify or completeness gate made the model look again; that is the harness
//!   asking, not the model looping).
//! * Everything else is **stale** (nothing changed, nothing new) or **all-failed**.
//!
//! Seven lanes watch the evidence (narration, futile, failure, cycle, check, waiting and idle,
//! see the constants below) under one latch: one nudge per episode, whichever lane fires first,
//! and a stop only after that nudge, at least [`MIN_STEPS_AFTER_NUDGE`] steps later, once a
//! lane's own stop bar is reached. A turn that edits, closes tasks, changes the tree or passes
//! checks is never touched, and the idle and waiting lanes, the only ones that can stop a turn
//! whose every step looks new, stop only turns nobody is watching.
//!
//! Pure data in, verdict out; [`crate::progress_supervisor`] is the part that touches the
//! transcript and the presenter.

mod check;
mod lanes;
mod tree;

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
/// Relapsing steps tolerated after a narration nudge before the turn ends, counted over the
/// [`RELAPSE_WINDOW`] steps that follow it. A relapse is a stale step, or one that opens with the
/// same words again over output that only looked new; a step that read a file for the first time
/// is not one, however much the model repeats itself.
pub(crate) const RELAPSE_STEPS: u32 = 2;
const RELAPSE_WINDOW: u32 = 4;
/// Consecutive stale or all-failed steps before the nudge and before the stop.
pub(crate) const FUTILE_NUDGE_AT: u32 = 5;
pub(crate) const FUTILE_STOP_AT: u32 = 10;
/// Consecutive steps in which every call failed with the same error, before the nudge and before
/// the stop.
pub(crate) const FAILURE_NUDGE_AT: u32 = 3;
pub(crate) const FAILURE_STOP_AT: u32 = 6;
/// The same batch with the same result this many times in a row is a cycle; so is a window of
/// [`CYCLE_WINDOW`] steps with at most two distinct batches, one recurring this many times, each
/// with one result throughout.
const CYCLE_AT: u32 = 3;
const CYCLE_WINDOW: usize = 6;
/// Steps for which a cycle has held, counting the one that nudged, before the stop: the nudge,
/// then two further repeats.
pub(crate) const CYCLE_STOP_AFTER: u32 = 3;
/// Runs of one check failing with the same output, however much was edited between them, before
/// the nudge and before the stop. Generous: a long debugging session can take many tries at one
/// failure, and a check that fails identically after six different edits is not listening to them.
pub(crate) const CHECK_NUDGE_AT: u32 = 6;
pub(crate) const CHECK_STOP_AT: u32 = 12;
/// Waiting steps since the last real progress before the nudge, and before the stop of a turn
/// nobody is watching. A wait is a call that polls something external; one that never becomes
/// ready would otherwise go on for as long as there is budget.
pub(crate) const WAIT_NUDGE_AT: u32 = 24;
pub(crate) const WAIT_STOP_AT: u32 = 96;
/// A stop needs at least this many steps after the nudge to let the model react to it.
pub(crate) const MIN_STEPS_AFTER_NUDGE: u32 = 2;
/// Consecutive steps with every lane under its nudge line that close an episode.
const QUIET_STEPS_TO_CLOSE: u32 = 4;
/// Steps after a stop in which a turn that repeats the stopped call or opening is stopped again
/// at once, with no second nudge.
const RESUME_WATCH_STEPS: u32 = 4;
/// Identical harness notes of one kind admitted in a burst; the next copies are dropped until
/// [`NOTE_REMIND_AFTER_STEPS`] steps have passed, so a diagnostic that persists is not lost for
/// the rest of the turn.
const NOTE_REPEAT_LIMIT: u32 = 3;
const NOTE_REMIND_AFTER_STEPS: u32 = 25;
/// Seen-line set size past which it is dropped and rebuilt; bounds memory on a very long turn
/// without ever leaving the ledger unable to judge (it just re-learns).
const MAX_SEEN_LINES: usize = 200_000;
/// A result with at least this share (1 in `NEW_LINE_DIVISOR`) of lines unseen is informative.
const NEW_LINE_DIVISOR: usize = 4;
/// A failed call has to print at least this many lines (the shell's header counts) before what it
/// printed can be new information.
const MIN_FAILURE_LINES: usize = 3;
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
    /// It read a path for the first time.
    fresh_read: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Episode {
    Quiet,
    Nudged {
        lane: Lane,
        /// Steps since the nudge, and how many of the first [`RELAPSE_WINDOW`] were stale.
        since: u32,
        relapses: u32,
    },
}

/// Which lanes a step may fire. A wait changes nothing but the waiting and idle counters, so
/// only those can fire on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    Waiting,
}

/// What a step was, as the stop watch remembers it.
#[derive(Debug, Clone, Copy)]
struct StepFacts {
    batch_sig: u64,
    result_fp: u64,
    opening: Option<u64>,
}

#[derive(Debug, Clone)]
struct FailureRun {
    tool: String,
    /// The error, digits masked.
    cause: u64,
    head: String,
    run: u32,
}

#[derive(Debug)]
pub(crate) struct ProgressLedger {
    narration: NarrationTracker,
    seen_signatures: HashSet<u64>,
    seen_lines: HashSet<u64>,
    seen_paths: HashSet<u64>,
    window: VecDeque<StepRecord>,
    /// Tool steps observed this turn, and model calls begun (the explicit step limit counts
    /// those, across every re-drive pass).
    step_no: u32,
    model_calls: usize,
    futile_run: u32,
    failure: Option<FailureRun>,
    recent_batches: VecDeque<(u64, u64)>,
    cycle_run: u32,
    /// Whether the step being observed reached the repeat count at all. Waiting and gated steps
    /// return before it, and must not make an earlier step's `cycle_run` look current.
    cycle_counted: bool,
    cycle_hold: u32,
    idle_steps: u32,
    wait_steps: u32,
    last_progress: Option<(u32, String)>,
    /// The most repeated (call label, result) among the recent steps, for the report.
    recent_calls: VecDeque<(String, u64)>,
    episode: Episode,
    quiet_steps: u32,
    stop_suppressed: bool,
    trees: tree::TreeWatch,
    checks: check::CheckWatch,
    gate_pending: bool,
    steer_pending: bool,
    last_stop: Option<StepFacts>,
    stop_candidate: Option<StepFacts>,
    resume_steps: u32,
    /// Per (kind, content): copies admitted in the current burst and the step of the last one.
    note_counts: HashMap<(MessageKind, u64), (u32, u32)>,
    idle_nudge: u32,
    idle_stop: u32,
    wait_nudge: u32,
    wait_stop: u32,
    headless: bool,
    futile_nudge: u32,
    futile_stop: u32,
    /// Cycle-hold steps before the cycle lane stops the turn.
    cycle_stop_after: u32,
}

impl Default for ProgressLedger {
    fn default() -> Self {
        Self {
            narration: NarrationTracker::default(),
            seen_signatures: HashSet::new(),
            seen_lines: HashSet::new(),
            seen_paths: HashSet::new(),
            window: VecDeque::new(),
            step_no: 0,
            model_calls: 0,
            futile_run: 0,
            failure: None,
            recent_batches: VecDeque::new(),
            cycle_run: 0,
            cycle_counted: false,
            cycle_hold: 0,
            idle_steps: 0,
            wait_steps: 0,
            last_progress: None,
            recent_calls: VecDeque::new(),
            episode: Episode::Quiet,
            quiet_steps: 0,
            stop_suppressed: false,
            trees: tree::TreeWatch::default(),
            checks: check::CheckWatch::default(),
            gate_pending: false,
            steer_pending: false,
            last_stop: None,
            stop_candidate: None,
            resume_steps: 0,
            note_counts: HashMap::new(),
            idle_nudge: DEFAULT_IDLE_NUDGE_STEPS,
            idle_stop: DEFAULT_IDLE_STOP_STEPS,
            wait_nudge: WAIT_NUDGE_AT,
            wait_stop: WAIT_STOP_AT,
            headless: false,
            futile_nudge: FUTILE_NUDGE_AT,
            futile_stop: FUTILE_STOP_AT,
            cycle_stop_after: CYCLE_STOP_AFTER,
        }
    }
}

impl ProgressLedger {
    /// The ledger for one bridged turn, judged call by call from `forge mcp-serve`: each call is
    /// one step, so the futile and waiting bars double, and with no prose to compare and no idle
    /// clock the narration and idle lanes stay off.
    pub(crate) fn for_bridge() -> Self {
        Self {
            futile_nudge: FUTILE_NUDGE_AT * 2,
            futile_stop: FUTILE_STOP_AT * 2,
            wait_nudge: WAIT_NUDGE_AT * 2,
            wait_stop: WAIT_STOP_AT * 2,
            idle_nudge: 0,
            idle_stop: 0,
            ..Self::default()
        }
    }

    /// `[harness] repeat_interrupt` lets the model hear about a repeated call two steps before
    /// the cycle lane stops the turn.
    pub(crate) fn set_repeat_interrupt(&mut self, on: bool) {
        self.cycle_stop_after = if on {
            CYCLE_STOP_AFTER + 2
        } else {
            CYCLE_STOP_AFTER
        };
    }

    /// The label of the call just repeated when it is the `at`-th identical call (same batch, same
    /// result) in a row. Exactly `at` and only from the step that counted it, so the interrupt is sent
    /// once per run, not again by the waiting or gated steps that follow.
    pub(crate) fn repeat_label(&self, at: u32) -> Option<String> {
        if !self.cycle_counted || self.cycle_run != at {
            return None;
        }
        self.recent_calls
            .back()
            .map(|(label, _)| label.clone())
            .filter(|l| !l.is_empty())
    }

    /// The idle thresholds (`0` disables) and whether no person is watching the turn. Cheap,
    /// called before every judgement so a config change or a mode switch takes effect at once.
    /// `idle_stop` also gates the waiting stop: `0` means a turn nobody watches is never stopped
    /// for taking long.
    pub(crate) fn configure(&mut self, idle_nudge: usize, idle_stop: usize, headless: bool) {
        self.idle_nudge = u32::try_from(idle_nudge).unwrap_or(u32::MAX);
        self.idle_stop = u32::try_from(idle_stop).unwrap_or(u32::MAX);
        self.headless = headless;
    }

    /// A new user turn. What the model has read stays read, but the turn gets a fresh episode and
    /// fresh counters: carrying a pending nudge or a window full of stale steps across turns
    /// halted every later turn after its first step in long sessions, where nearly every read
    /// repeats something seen hours earlier. A loop that really survives the user's message is
    /// still caught within this turn. Argument signatures are forgotten too, so a deliberate redo
    /// of an earlier edit counts as a fresh mutation, and so are the tree states and what the
    /// checks did. What the last stop was on is kept, so a bare `continue` that resumes the same
    /// calls is stopped again at once.
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
        self.wait_steps = 0;
        self.last_progress = None;
        self.recent_calls.clear();
        self.episode = Episode::Quiet;
        self.quiet_steps = 0;
        self.stop_suppressed = false;
        self.trees = tree::TreeWatch::default();
        self.checks = check::CheckWatch::default();
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

    /// The working tree as a pass of the turn starts, with nothing in flight.
    pub(crate) fn rebase_tree(&mut self, state: Option<u64>) {
        self.trees.rebase(state);
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
    /// it again is legitimate, and a note it was told is worth telling again. Forget what was
    /// seen, but keep the counters and any pending nudge: a loop that survives a compaction is
    /// exactly the case this exists to catch.
    pub(crate) fn note_compaction(&mut self) {
        self.seen_signatures.clear();
        self.seen_lines.clear();
        self.seen_paths.clear();
        self.note_counts.clear();
    }

    /// Whether a harness note of this kind may be added again. One that repeats verbatim is the
    /// harness talking to itself: the fourth copy in a burst is dropped. A copy that comes
    /// [`NOTE_REMIND_AFTER_STEPS`] steps after the last one is admitted, so a diagnostic that
    /// persists is surfaced again instead of going silent for the rest of the turn.
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
        let step = self.step_no;
        let (count, last) = self
            .note_counts
            .entry((kind, crate::loop_progress::hash_str(content)))
            .or_insert((0, step));
        if *count >= NOTE_REPEAT_LIMIT && step.saturating_sub(*last) < NOTE_REMIND_AFTER_STEPS {
            return false;
        }
        if *count >= NOTE_REPEAT_LIMIT {
            *count = 0;
        }
        *count += 1;
        *last = step;
        true
    }

    /// How hard this turn is currently repeating itself, as a rung on the sampling ladder
    /// (0 = nothing, 1 = restated once, 2 = persistently). Reports the evidence EARLY, while the
    /// turn is still salvageable, so the next request can be shaped differently (a higher
    /// temperature, a repetition penalty) instead of being sent again identically. Only stale
    /// steps count: a productive turn never runs hotter, and neither does a long investigation
    /// that was merely asked to report, or a long wait.
    pub(crate) fn pressure(&self) -> u8 {
        let verbatim = self.narration.max_verbatim();
        let repeats = self.narration_repeats();
        let latched = matches!(
            self.episode,
            Episode::Nudged { lane, .. } if !matches!(lane, Lane::Idle | Lane::Waiting)
        );
        if verbatim >= 3 || latched || repeats >= 2 {
            return 2;
        }
        if verbatim >= 2 || repeats >= 1 {
            return 1;
        }
        0
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
    /// returned, whether a tracked task closed during it, and (called only when a step could have
    /// changed files unseen) the state of the working tree.
    pub(crate) fn observe_step(
        &mut self,
        narration: &str,
        calls: &[CallObservation],
        task_closed: bool,
        tree: &mut dyn FnMut() -> Option<u64>,
    ) -> Verdict {
        self.step_no += 1;
        self.cycle_counted = false;
        let (new_write, informative, fresh_read) = self.assess(calls);
        let steered = std::mem::take(&mut self.steer_pending);
        let gated = std::mem::take(&mut self.gate_pending);
        let fresh_flip = self.checks.observe(calls);
        let waiting = calls.iter().any(CallObservation::waiting);
        if calls.iter().any(CallObservation::wrote) {
            self.trees.wrote();
        }
        let mut strong = new_write || fresh_flip || task_closed || steered;
        let mut tree_moved = false;
        if !strong && !waiting && calls.iter().any(CallObservation::opaque) {
            tree_moved = self.trees.is_new_state(tree);
            strong = tree_moved;
        }
        if !strong && waiting {
            if informative {
                self.futile_run = 0;
            }
            self.idle_steps += 1;
            self.wait_steps += 1;
            return self.decide(StepFacts::of(narration, calls), Scope::Waiting);
        }
        if !strong && gated {
            return Verdict::Fine;
        }
        let restated = self.narration.observe(narration, strong);
        let facts = StepFacts::of(narration, calls);
        if strong {
            let what = if new_write {
                calls
                    .iter()
                    .find(|c| c.wrote())
                    .map_or_else(|| "a file change".to_string(), |c| c.label().to_string())
            } else if task_closed {
                "a task closed".to_string()
            } else if fresh_flip {
                "a failing check passed".to_string()
            } else if tree_moved {
                "the working tree changed".to_string()
            } else {
                "your message".to_string()
            };
            self.reset_on_progress(what);
            return self.decide(facts, Scope::All);
        }
        if let Some(last) = self.last_stop {
            self.resume_steps += 1;
            let same_call = facts.batch_sig == last.batch_sig && facts.result_fp == last.result_fp;
            let same_opening = last.opening.is_some() && facts.opening == last.opening;
            // The same call that comes back differently, or the same opening over new findings,
            // is the person's `continue` working: whatever was blocking has moved.
            if same_call || (same_opening && !informative) {
                self.stop_candidate = Some(facts);
                return Verdict::Stop(self.report(Lane::Resumed));
            }
            if self.resume_steps >= RESUME_WATCH_STEPS {
                self.last_stop = None;
            }
        }
        self.count_stale(calls, informative, (restated, fresh_read), &facts);
        self.decide(facts, Scope::All)
    }

    /// What a step's calls added: a new write, anything new at all, and a path read for the first
    /// time.
    fn assess(&mut self, calls: &[CallObservation]) -> (bool, bool, bool) {
        let (mut new_write, mut informative, mut fresh_read) = (false, false, false);
        for call in calls {
            let new_signature = call.ok() && self.seen_signatures.insert(call.signature());
            if call.mutating() && new_signature {
                new_write = true;
            }
            // Every target is recorded, not only up to the first new one.
            let mut first_read = false;
            for target in call.read_targets() {
                first_read |= self.seen_paths.insert(*target);
            }
            if call.ok() && first_read {
                fresh_read = true;
            }
            if call.ok() && (first_read || (call.silent() && new_signature)) {
                informative = true;
            }
            if self.seen_lines.len() > MAX_SEEN_LINES {
                self.seen_lines.clear();
            }
            let total = call.lines().len();
            // A short error is a cause, not information: its novelty is a different path or
            // number in the same sentence. A listing of failures, a stack trace, compiler output
            // is.
            if !call.ok() && total < MIN_FAILURE_LINES {
                continue;
            }
            let new = call
                .lines()
                .iter()
                .filter(|l| self.seen_lines.insert(**l))
                .count();
            if new > 0 && new * NEW_LINE_DIVISOR >= total {
                informative = true;
            }
        }
        (new_write, informative, fresh_read)
    }

    fn reset_on_progress(&mut self, what: String) {
        self.window.clear();
        self.futile_run = 0;
        self.failure = None;
        self.recent_batches.clear();
        self.cycle_run = 0;
        self.cycle_hold = 0;
        self.idle_steps = 0;
        self.wait_steps = 0;
        self.recent_calls.clear();
        // Edits do not end a check's streak of identical failures, so neither do they end the
        // episode that streak started.
        if !matches!(
            self.episode,
            Episode::Nudged {
                lane: Lane::Check,
                ..
            }
        ) {
            self.episode = Episode::Quiet;
            self.quiet_steps = 0;
            self.stop_suppressed = false;
        }
        self.last_stop = None;
        self.resume_steps = 0;
        self.last_progress = Some((self.step_no, what));
    }

    fn count_stale(
        &mut self,
        calls: &[CallObservation],
        informative: bool,
        (restated, fresh_read): (bool, bool),
        facts: &StepFacts,
    ) {
        let stale = !informative;
        self.window.push_back(StepRecord {
            stale,
            restated,
            fresh_read,
        });
        if self.window.len() > WINDOW_STEPS {
            self.window.pop_front();
        }
        self.futile_run = if informative { 0 } else { self.futile_run + 1 };
        self.idle_steps += 1;

        // The same failure again, not failing calls in general: a search that finds nothing
        // exits non-zero without a word, and a triage of six failing scripts fails six ways.
        let failed_with_error =
            !calls.is_empty() && calls.iter().all(|c| !c.ok() && c.cause().is_some());
        self.failure = match calls.last().filter(|_| failed_with_error) {
            Some(last) => {
                let tool = single_tool(calls);
                let cause = last.cause().unwrap_or_default();
                let run = match &self.failure {
                    Some(f) if f.tool == tool && f.cause == cause && !informative => f.run + 1,
                    _ => 1,
                };
                Some(FailureRun {
                    tool,
                    cause,
                    head: last.error_head().unwrap_or("").to_string(),
                    run,
                })
            }
            None => None,
        };

        let key = (facts.batch_sig, facts.result_fp);
        self.cycle_run = if self.recent_batches.back() == Some(&key) {
            self.cycle_run + 1
        } else {
            1
        };
        self.cycle_counted = true;
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

    /// At most two distinct batches in the window, one recurring [`CYCLE_AT`] times, and each
    /// batch with the same result every time it ran: A,B,A,B,A,B. Requiring at most two keeps
    /// act-then-observe workflows out (tap, screen, tap, screen: the screen repeats while every
    /// action differs), and requiring both to be constant keeps a wait out (`sleep`, then a log
    /// that grows: the sleep repeats while the log shows something new each time).
    fn oscillating(&self) -> bool {
        let mut results: HashMap<u64, (u64, u32, bool)> = HashMap::new();
        for (sig, fp) in &self.recent_batches {
            let entry = results.entry(*sig).or_insert((*fp, 0, true));
            entry.1 += 1;
            entry.2 &= entry.0 == *fp;
        }
        results.len() <= 2
            && results.values().all(|(_, _, constant)| *constant)
            && results.values().any(|(_, times, _)| *times >= CYCLE_AT)
    }

    fn narration_repeats(&self) -> usize {
        self.window.iter().filter(|r| r.stale && r.restated).count()
    }
}

impl StepFacts {
    fn of(narration: &str, calls: &[CallObservation]) -> Self {
        Self {
            batch_sig: batch_signature(calls),
            result_fp: combined_result_fp(calls),
            opening: opening_fingerprint(narration),
        }
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
mod regressions;
#[cfg(test)]
mod tests;
