//! Structural progress judgement for the direct tool loop.
//!
//! Text similarity alone cannot tell a stuck model from a busy one: seven consecutive successful
//! edit/test steps that all open `Now let me update the …` repeat themselves exactly as much as
//! thirty steps of `You're right — I looped` that re-read the same file. What separates them is
//! whether the tool calls got the model anywhere. A step makes STRONG progress when it
//! successfully ran a mutating tool (`write_file`/`edit`/…) with arguments not tried before this
//! turn: the workspace changed, so everything resets. It is merely INFORMATIVE when a successful
//! result carries at least a quarter of lines the model has not already seen this turn (so
//! re-reading a file with a slightly different `limit`, or re-running a check that prints the
//! same thing, is stale, while reading a different file or watching a screen change is not).
//! Failed calls are never progress (the failure-loop guard owns them).
//!
//! A repeated opening only counts against the model on stale steps, over a sliding window of
//! recent steps. Once [`NARRATION_NUDGE_AT`] such steps accumulate the model is told to answer;
//! if [`RELAPSE_STEPS`] more stale steps follow, it is halted — the apology-relapse pattern
//! (`You're right, no more loops` followed by more of the same) gets no third chance.
//! Stagnation without any repeated wording (the model rephrases every step while reading the same
//! things, or alternates A,B,A,B with identical results) is caught by the same window at a
//! higher bar, [`STAGNANT_NUDGE_AT`].
//!
//! Pure data in, verdict out; [`Session::judge_loop_progress`] is the only part that touches the
//! transcript.

use crate::stall_guard::NarrationTracker;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

/// Steps remembered for the judgement.
const WINDOW_STEPS: usize = 10;
/// Stale narration repeats (restatements on steps with nothing new) within the window before the
/// model is nudged to answer. The third repeat is the fourth time the sentence is said.
pub(crate) const NARRATION_NUDGE_AT: usize = 3;
/// Steps with nothing new within the window, regardless of wording, before the same nudge. Higher
/// than the narration bar because a model that varies its words may simply be exploring:
/// legitimately waiting on a job produces unchanged output for a while, and the identical-call
/// doom-loop guard separately handles byte-identical polling.
pub(crate) const STAGNANT_NUDGE_AT: usize = 8;
/// Relapsing steps (nothing new, or the same opening again) tolerated after a nudge before the
/// turn ends.
pub(crate) const RELAPSE_STEPS: usize = 2;
/// Steps after a nudge in which [`RELAPSE_STEPS`] stale ones halt the turn; surviving the window
/// without them counts as the model having taken the hint.
const RELAPSE_WINDOW: usize = 4;

/// Seen-line set size past which it is dropped and rebuilt; bounds memory on a very long turn
/// without ever leaving the tracker unable to judge (it just re-learns).
const MAX_SEEN_LINES: usize = 200_000;
/// A result with at least this share (1 in `NEW_LINE_DIVISOR`) of lines unseen is informative.
const NEW_LINE_DIVISOR: usize = 4;
/// One-line results at most this long are treated as status messages (`no matches for 'x'`,
/// `ok`): their quoted parts are masked so varying only the query does not read as new.
const SHORT_RESULT_CHARS: usize = 160;

/// What one executed tool call looked like, reduced to what the judgement needs. Built where the
/// result is in hand so the (possibly huge) text is hashed once and never copied.
#[derive(Debug, Clone)]
pub(crate) struct CallObservation {
    signature: u64,
    ok: bool,
    mutating: bool,
    lines: Vec<u64>,
}

impl CallObservation {
    pub(crate) fn new(signature: u64, ok: bool, mutating: bool, result: &str) -> Self {
        Self {
            signature,
            ok,
            mutating,
            lines: line_fingerprints(result),
        }
    }
}

/// Stable hash of one call's tool name and JSON arguments (object keys are ordered, so equal
/// arguments hash equal).
pub(crate) fn call_signature(call: &forge_types::ToolCall) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    call.name.hash(&mut h);
    call.args.to_string().hash(&mut h);
    h.finish()
}

fn line_fingerprints(result: &str) -> Vec<u64> {
    let mut lines = result.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next();
    let rest: Vec<&str> = lines.collect();
    let mut out = Vec::with_capacity(rest.len() + 1);
    let single_short = rest.is_empty() && first.is_some_and(|l| l.len() <= SHORT_RESULT_CHARS);
    if let Some(first) = first {
        out.push(hash_str(&if single_short {
            mask_quoted(first)
        } else if first.starts_with("shell:") {
            // `shell: exit 0 in 42ms` — the timing differs on every run of an identical command.
            first.replace(|c: char| c.is_ascii_digit(), "#")
        } else {
            first.to_string()
        }));
    }
    out.extend(rest.into_iter().map(hash_str));
    out
}

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Replace everything between matching quote characters with nothing.
fn mask_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut open: Option<char> = None;
    for c in s.chars() {
        match open {
            Some(q) if c == q => {
                open = None;
                out.push(c);
            }
            Some(_) => {}
            None => {
                if matches!(c, '\'' | '"' | '`') {
                    open = Some(c);
                }
                out.push(c);
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Fine,
    /// The model keeps restating itself while getting nothing new: tell it to answer.
    NudgeRepeating {
        stale_steps: usize,
    },
    /// Most recent steps found nothing new, though the wording varied: tell it to answer.
    NudgeStagnant {
        stale_steps: usize,
    },
    /// Still nothing new after the nudge: end the turn cleanly.
    Halt {
        stale_steps: usize,
    },
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

#[derive(Debug, Default)]
pub(crate) struct ProgressTracker {
    narration: NarrationTracker,
    seen_signatures: HashSet<u64>,
    seen_lines: HashSet<u64>,
    window: std::collections::VecDeque<StepRecord>,
    nudged: bool,
    /// Steps since the nudge, and how many of them were stale.
    steps_since_nudge: usize,
    stale_since_nudge: usize,
}

impl ProgressTracker {
    /// Judge one finished tool step: the narration the model emitted with it and what its calls
    /// returned.
    ///
    /// Two tiers of progress. A successful mutation with new arguments is STRONG: it wipes the
    /// slate, because the workspace actually changed. New information alone (a read of something
    /// not seen before, a screen that changed) is WEAK: it stops that one step counting against
    /// the model but does not erase the steps around it. The distinction matters because a stuck
    /// model rarely returns nothing new on every single step — live, it alternated a re-read of a
    /// big file (nothing new) with a small `git diff | grep` (a few new lines) for 25 steps — so
    /// a rule that any new line clears the count never fires on the loops that matter, while a
    /// window over recent steps does.
    pub(crate) fn observe_step(&mut self, narration: &str, calls: &[CallObservation]) -> Verdict {
        let (strong, informative) = self.assess(calls);
        let restated = self.narration.observe(narration, strong);
        if strong {
            self.window.clear();
            self.nudged = false;
            self.steps_since_nudge = 0;
            self.stale_since_nudge = 0;
            return Verdict::Fine;
        }
        let stale = !informative;
        self.window.push_back(StepRecord { stale, restated });
        if self.window.len() > WINDOW_STEPS {
            self.window.pop_front();
        }
        if self.nudged {
            self.steps_since_nudge += 1;
            // Told to stop repeating itself, a step that still repeats itself relapses even if it
            // happened to read something new: the live apology loop did exactly that.
            if stale || restated {
                self.stale_since_nudge += 1;
            }
            if self.stale_since_nudge >= RELAPSE_STEPS {
                return Verdict::Halt {
                    stale_steps: self.stale_steps(),
                };
            }
            if self.steps_since_nudge >= RELAPSE_WINDOW {
                // It took the hint: new information kept arriving. Judge afresh from here.
                self.nudged = false;
                self.window.clear();
            }
            return Verdict::Fine;
        }
        if self.stale_repeats() >= NARRATION_NUDGE_AT {
            self.nudged = true;
            self.steps_since_nudge = 0;
            self.stale_since_nudge = 0;
            return Verdict::NudgeRepeating {
                stale_steps: self.stale_steps(),
            };
        }
        if self.stale_steps() >= STAGNANT_NUDGE_AT {
            self.nudged = true;
            self.steps_since_nudge = 0;
            self.stale_since_nudge = 0;
            return Verdict::NudgeStagnant {
                stale_steps: self.stale_steps(),
            };
        }
        Verdict::Fine
    }

    fn stale_steps(&self) -> usize {
        self.window.iter().filter(|r| r.stale).count()
    }

    fn stale_repeats(&self) -> usize {
        self.window.iter().filter(|r| r.stale && r.restated).count()
    }

    /// `(strong, informative)` for this step; records everything seen either way.
    fn assess(&mut self, calls: &[CallObservation]) -> (bool, bool) {
        let (mut strong, mut informative) = (false, false);
        for call in calls {
            let new_signature = self.seen_signatures.insert(call.signature);
            if !call.ok {
                continue;
            }
            if call.mutating && new_signature {
                strong = true;
            }
            if self.seen_lines.len() > MAX_SEEN_LINES {
                self.seen_lines.clear();
            }
            let total = call.lines.len();
            let new = call
                .lines
                .iter()
                .filter(|l| self.seen_lines.insert(**l))
                .count();
            if new > 0 && new * NEW_LINE_DIVISOR >= total {
                informative = true;
            }
        }
        (strong, informative || strong)
    }

    /// A new user turn. What the model has read stays read and the recent window stays put: the
    /// user typing "you are stuck in a loop" starts a new turn but does not make the model any
    /// less stuck (live, the same loop survived three such messages because every message reset
    /// the guard). Only argument signatures are forgotten, so a deliberate redo of an earlier edit
    /// still counts as a fresh mutation.
    pub(crate) fn begin_turn(&mut self) {
        self.narration.begin_turn();
        self.seen_signatures.clear();
    }

    /// The transcript was just compacted: the model no longer remembers what it read, so reading
    /// it again is legitimate. Forget what was seen, but keep the window and any pending nudge —
    /// a loop that survives a compaction is exactly the case this exists to catch.
    pub(crate) fn note_compaction(&mut self) {
        self.seen_signatures.clear();
        self.seen_lines.clear();
    }

    /// How hard this turn is currently repeating itself, as a rung on the sampling ladder
    /// (0 = nothing, 1 = restated once, 2 = persistently). Reports the evidence EARLY, while the
    /// turn is still salvageable, so the next request can be shaped differently (a higher
    /// temperature, a repetition penalty) instead of being sent again identically. Only stale
    /// steps count: a productive turn never runs hotter.
    pub(crate) fn pressure(&self) -> u8 {
        let verbatim = self.narration.max_verbatim();
        let repeats = self.stale_repeats();
        if verbatim >= 3 || self.nudged || repeats >= 2 {
            return 2;
        }
        if verbatim >= 2 || repeats >= 1 {
            return 1;
        }
        0
    }
}

impl crate::Session {
    /// Called after the step's tools ran: novelty decides whether repeated wording is a loop,
    /// and a nudge must land after the tool results.
    /// Judge the step that just finished. Delivers a nudge into the transcript (after the tool
    /// results, so message ordering stays valid) or reports that the turn must end.
    pub(crate) fn judge_loop_progress(
        &mut self,
        narration: &str,
        observations: &[CallObservation],
    ) -> bool {
        let verdict = self.progress.observe_step(narration, observations);
        // Carried into the NEXT request's sampling (see `model_request`).
        self.repetition_pressure = self.progress.pressure();
        let (warning, nudge) = match verdict {
            Verdict::Fine => return false,
            Verdict::NudgeRepeating { stale_steps } => (
                format!(
                    "model has opened {} replies with the same sentence while {stale_steps} of \
                     its last {WINDOW_STEPS} steps found nothing new — nudging it to answer \
                     before stopping",
                    NARRATION_NUDGE_AT + 1
                ),
                crate::stall_guard::STALL_NUDGE,
            ),
            Verdict::NudgeStagnant { stale_steps } => (
                format!(
                    "{stale_steps} of the last {WINDOW_STEPS} steps found nothing new — nudging \
                     the model to answer before stopping"
                ),
                STAGNATION_NUDGE,
            ),
            Verdict::Halt { stale_steps } => {
                self.presenter
                    .emit(forge_types::PresenterEvent::Warning(format!(
                        "{stale_steps} of the model's last {WINDOW_STEPS} steps found nothing new \
                         and it kept going after a nudge — stopping to avoid a loop. Nothing is \
                         lost: edits so far are kept. Reply with what you want next, or \
                         `continue` to let it try again."
                    )));
                return true;
            }
        };
        self.presenter
            .emit(forge_types::PresenterEvent::Warning(warning));
        let seq = self.next_seq();
        let _ = self
            .store
            .add_message(&self.id, seq, forge_types::Role::System, nudge, None);
        self.transcript.push(forge_types::Message::system(nudge));
        false
    }
}

pub(crate) const STAGNATION_NUDGE: &str = "Your last several steps returned nothing you had not \
already seen, whatever order or wording you used. More of the same will not change that. Stop \
investigating: answer now from what you have, say what is still uncertain, or make a concrete \
edit or run a command that tests a NEW idea. If you are waiting on something, say so and stop \
polling.";

#[cfg(test)]
mod tests {
    use super::*;

    const REPEATED: &str =
        "You're right — I looped. Answering the question from evidence, then finishing the revert.";

    fn obs(sig: u64, ok: bool, mutating: bool, result: &str) -> CallObservation {
        CallObservation::new(sig, ok, mutating, result)
    }

    /// A read that shows `n` lines of a file, the first `n - 1` of which an earlier read of the
    /// same file already showed (`limit` creeping up by one).
    fn creeping_read(n: usize) -> CallObservation {
        let text: String = (0..n).map(|i| format!("line {i} of the file\n")).collect();
        obs(1000 + n as u64, true, false, &text)
    }

    /// Twenty openings with nothing in common, so wording never counts as a repeat.
    const DISTINCT: [&str; 20] = [
        "Checking whether the cache invalidates correctly under load",
        "Perhaps the scheduler drops the wakeup before the lock releases",
        "Switching to the config loader since parsing might be lenient",
        "Reconsidering how retries interact with idempotency keys here",
        "Tracing allocation sizes through the buffer pool next",
        "Comparing serialization output between both encoder versions",
        "Wondering if the migration ordering explains the missing column",
        "Looking into whether timeouts shadow the underlying socket error",
        "Revisiting the assumption about monotonic clock readings",
        "Auditing feature flags that gate the experimental path",
        "Examining log rotation settings for truncated entries",
        "Measuring queue latency percentiles around the incident window",
        "Questioning whether the proxy rewrites headers unexpectedly",
        "Mapping which crates depend on the shared error enum",
        "Validating the fixture data against the published schema",
        "Probing the watcher for missed filesystem events",
        "Studying how backpressure propagates through the pipeline",
        "Inspecting build script outputs for stale artifacts",
        "Reviewing locale handling inside the date formatter",
        "Estimating memory growth across repeated reconnects",
    ];

    fn run(steps: &[(&str, Vec<CallObservation>)]) -> Vec<Verdict> {
        let mut t = ProgressTracker::default();
        steps
            .iter()
            .map(|(text, calls)| t.observe_step(text, calls))
            .collect()
    }

    fn first(verdicts: &[Verdict], pred: impl Fn(&Verdict) -> bool) -> Option<usize> {
        verdicts.iter().position(pred).map(|i| i + 1)
    }

    #[test]
    fn a_productive_edit_streak_with_one_opening_never_nudges_or_halts() {
        let steps: Vec<_> = (0..30)
            .map(|i| {
                let calls = if i % 2 == 0 {
                    vec![obs(i, true, true, "edited src/lib.rs")]
                } else {
                    vec![obs(
                        500 + i,
                        true,
                        false,
                        &format!("running 3 tests\ntest case_{i} ... ok\ntest result: ok"),
                    )]
                };
                ("Now let me update the parser to cover the next case", calls)
            })
            .collect();
        let verdicts = run(&steps);
        assert!(verdicts.iter().all(|v| *v == Verdict::Fine), "{verdicts:?}");
    }

    #[test]
    fn rereading_with_a_creeping_limit_and_one_opening_halts_by_step_six() {
        let steps: Vec<_> = (20..40)
            .map(|n| (REPEATED, vec![creeping_read(n)]))
            .collect();
        let verdicts = run(&steps);
        assert_eq!(
            first(&verdicts, |v| matches!(v, Verdict::NudgeRepeating { .. })),
            Some(4),
            "{verdicts:?}"
        );
        assert_eq!(
            first(&verdicts, |v| matches!(v, Verdict::Halt { .. })),
            Some(6),
            "{verdicts:?}"
        );
    }

    #[test]
    fn a_changing_wording_still_halts_through_the_stagnation_counter() {
        let steps: Vec<_> = DISTINCT
            .iter()
            .enumerate()
            .map(|(i, t)| (*t, vec![creeping_read(20 + i)]))
            .collect();
        let verdicts = run(&steps);
        assert_eq!(
            first(&verdicts, |v| matches!(v, Verdict::NudgeStagnant { .. })),
            Some(STAGNANT_NUDGE_AT + 1),
            "{verdicts:?}"
        );
        assert_eq!(
            first(&verdicts, |v| matches!(v, Verdict::Halt { .. })),
            Some(STAGNANT_NUDGE_AT + RELAPSE_STEPS + 1),
            "{verdicts:?}"
        );
    }

    #[test]
    fn oscillation_with_identical_results_is_caught_even_with_new_prose_each_time() {
        let a = "contents of a.rs\nfn a() {}\nfn b() {}\nfn c() {}";
        let b = "contents of b.rs\nfn x() {}\nfn y() {}\nfn z() {}";
        let steps: Vec<_> = DISTINCT
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let call = if i % 2 == 0 {
                    obs(1, true, false, a)
                } else {
                    obs(2, true, false, b)
                };
                (*t, vec![call])
            })
            .collect();
        let verdicts = run(&steps);
        // Steps 1 and 2 are novel (first sight of each file); the stale streak starts at 3.
        assert!(
            first(&verdicts, |v| matches!(v, Verdict::Halt { .. })).is_some_and(|s| s <= 12),
            "{verdicts:?}"
        );
    }

    #[test]
    fn device_polling_with_changing_results_is_never_stopped() {
        let steps: Vec<_> = (0..40)
            .map(|i| {
                let calls = vec![
                    obs(10_000 + i, true, false, "tapped"),
                    obs(
                        1,
                        true,
                        false,
                        &format!("screen {i}\nbutton Next\nlabel Step {i} of 40\nprogress {i}%"),
                    ),
                ];
                ("Tapping the next control and checking the screen", calls)
            })
            .collect();
        let verdicts = run(&steps);
        assert!(verdicts.iter().all(|v| *v == Verdict::Fine), "{verdicts:?}");
    }

    #[test]
    fn device_polling_whose_screen_never_changes_is_still_a_loop() {
        let steps: Vec<_> = (0..20)
            .map(|i| {
                let calls = vec![
                    obs(10_000 + i, true, false, "tapped"),
                    obs(1, true, false, "screen home\nbutton Next\nlabel Welcome"),
                ];
                ("Tapping the next control and checking the screen", calls)
            })
            .collect();
        let verdicts = run(&steps);
        assert!(
            first(&verdicts, |v| matches!(v, Verdict::Halt { .. })).is_some(),
            "{verdicts:?}"
        );
    }

    #[test]
    fn varying_only_the_quoted_query_of_an_empty_search_is_not_new() {
        let steps: Vec<_> = (0..12)
            .map(|i| {
                (
                    REPEATED,
                    vec![obs(
                        i,
                        true,
                        false,
                        &format!("no matches for 'pattern_{i}'"),
                    )],
                )
            })
            .collect();
        let verdicts = run(&steps);
        assert!(
            first(&verdicts, |v| matches!(v, Verdict::Halt { .. })).is_some(),
            "{verdicts:?}"
        );
    }

    #[test]
    fn failed_calls_and_repeated_mutations_are_not_progress() {
        let mut t = ProgressTracker::default();
        let edit = || vec![obs(7, true, true, "edited src/lib.rs")];
        assert_eq!(t.observe_step(REPEATED, &edit()), Verdict::Fine);
        // The same edit again (a revert/re-apply cycle) and a failing edit add nothing.
        for _ in 0..2 {
            t.observe_step(REPEATED, &edit());
            t.observe_step(REPEATED, &[obs(8, false, true, "error: no match")]);
        }
        assert!(t.stale_steps() >= 4, "{}", t.stale_steps());
    }

    #[test]
    fn a_novel_step_after_the_nudge_clears_it_so_a_later_loop_gets_its_own() {
        let mut t = ProgressTracker::default();
        let mut verdicts = Vec::new();
        for n in 20..25 {
            verdicts.push(t.observe_step(REPEATED, &[creeping_read(n)]));
        }
        assert!(
            matches!(verdicts[3], Verdict::NudgeRepeating { .. }),
            "{verdicts:?}"
        );
        assert_eq!(
            t.observe_step("Found it", &[obs(99, true, true, "edited src/other.rs")]),
            Verdict::Fine
        );
        let mut again = Vec::new();
        for n in 30..36 {
            again.push(t.observe_step(REPEATED, &[creeping_read(n)]));
        }
        assert!(
            again
                .iter()
                .any(|v| matches!(v, Verdict::NudgeRepeating { .. })),
            "second loop was not nudged again: {again:?}"
        );
    }

    #[test]
    fn a_compaction_forgets_what_was_read_but_not_the_streak() {
        let mut t = ProgressTracker::default();
        for n in 20..24 {
            t.observe_step(REPEATED, &[creeping_read(n)]);
        }
        assert!(t.nudged);
        t.note_compaction();
        // The same file read again after compaction is new information for a model that lost it
        // (not stale, so not a halt)...
        assert_eq!(
            t.observe_step(REPEATED, &[creeping_read(23)]),
            Verdict::Fine
        );
        assert!(t.nudged, "the pending nudge survives the compaction");
        // ...but a loop that continues past it is caught again within a handful of steps.
        let verdicts: Vec<Verdict> = (24..34)
            .map(|n| t.observe_step(REPEATED, &[creeping_read(n)]))
            .collect();
        assert!(
            verdicts.iter().any(|v| matches!(v, Verdict::Halt { .. })),
            "{verdicts:?}"
        );
    }

    #[test]
    fn a_new_user_turn_does_not_reset_a_loop_in_progress() {
        let mut t = ProgressTracker::default();
        for n in 20..24 {
            t.observe_step(REPEATED, &[creeping_read(n)]);
        }
        assert!(t.nudged);
        t.begin_turn();
        let verdicts: Vec<Verdict> = (24..27)
            .map(|n| t.observe_step(REPEATED, &[creeping_read(n)]))
            .collect();
        assert!(
            verdicts.iter().any(|v| matches!(v, Verdict::Halt { .. })),
            "the user's complaint arrived as a new turn and must not buy the loop a fresh start: \
             {verdicts:?}"
        );
    }

    #[test]
    fn pressure_tracks_stale_repetition_only() {
        let mut t = ProgressTracker::default();
        for i in 0..6 {
            t.observe_step(
                "Now let me update the parser to cover the next case",
                &[obs(i, true, true, "edited src/lib.rs")],
            );
        }
        assert_eq!(t.pressure(), 0, "productive repetition never runs hotter");
        t.observe_step(REPEATED, &[creeping_read(20)]);
        t.observe_step(REPEATED, &[creeping_read(21)]);
        assert_eq!(t.pressure(), 1);
        t.observe_step(REPEATED, &[creeping_read(22)]);
        t.observe_step(REPEATED, &[creeping_read(23)]);
        assert_eq!(t.pressure(), 2);
    }

    #[test]
    fn shell_timing_in_the_header_is_not_new_information() {
        let a = obs(
            1,
            true,
            false,
            "shell: exit 0 in 42ms\n\nsame output\nmore output",
        );
        let b = obs(
            1,
            true,
            false,
            "shell: exit 0 in 19ms\n\nsame output\nmore output",
        );
        let mut t = ProgressTracker::default();
        assert!(t.assess(&[a]).1);
        assert!(!t.assess(&[b]).1);
    }
    /// Real looping stretches from the live store (2026-09-08 and 2026-09-17), reduced to the
    /// shape the tracker sees: narration openings, call signatures, success flags, and result
    /// lines interned to ids (one blob per distinct result). Anonymised: no file contents, paths
    /// or credentials survive.
    fn replay(name: &str) -> (Vec<Verdict>, usize) {
        let all: serde_json::Value =
            serde_json::from_str(include_str!("tests/fixtures/loop_replays.json")).unwrap();
        let seg = all
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap();
        let blobs: Vec<String> = seg["blobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|runs| {
                let mut text = String::new();
                for run in runs.as_array().unwrap() {
                    let (start, count) = (run[0].as_u64().unwrap(), run[1].as_u64().unwrap());
                    for id in start..start + count {
                        text.push_str(&format!("L{id}\n"));
                    }
                }
                text
            })
            .collect();
        let steps = seg["steps"].as_array().unwrap();
        let mut t = ProgressTracker::default();
        let verdicts = steps
            .iter()
            .map(|step| {
                let calls: Vec<CallObservation> = step["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        CallObservation::new(
                            c["sig"].as_u64().unwrap(),
                            c["ok"].as_bool().unwrap(),
                            c["mut"].as_bool().unwrap(),
                            &blobs[c["res"].as_u64().unwrap() as usize],
                        )
                    })
                    .collect();
                t.observe_step(step["text"].as_str().unwrap(), &calls)
            })
            .collect();
        (verdicts, steps.len())
    }

    /// The Sep 8 apology loop: 35 steps, no edit, the same `You're right — I looped` opening
    /// from step 6 on, alternating a re-read of a 1,349-line file with small `git`/`grep`
    /// queries. The old narration-only guard (5-word prefix, no notion of progress) nudged at
    /// step 14 and halted at step 18 when started fresh on this stretch; live it needed ~37 steps
    /// and three user messages because each message reset it.
    #[test]
    fn the_apology_loop_is_nudged_by_step_12_and_halted_by_step_14() {
        let (v, n) = replay("s1_looped_apology");
        assert_eq!(n, 35);
        assert_eq!(
            first(&v, |v| matches!(v, Verdict::NudgeRepeating { .. })),
            Some(12),
            "{v:?}"
        );
        assert_eq!(
            first(&v, |v| matches!(v, Verdict::Halt { .. })),
            Some(14),
            "{v:?}"
        );
    }

    /// Two stretches the old guard nudged although the model was working (it nudged at the last
    /// step of both): a sweep-and-push where every shell command printed something new, and a
    /// clippy cleanup reading fresh compiler output each time. Neither is a loop.
    #[test]
    fn working_stretches_the_old_guard_nudged_are_left_alone() {
        for name in ["s2_sweep_and_push", "s3_clippy_shell"] {
            let (v, _) = replay(name);
            assert!(v.iter().all(|v| *v == Verdict::Fine), "{name}: {v:?}");
        }
    }
}
