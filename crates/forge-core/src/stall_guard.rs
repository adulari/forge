//! Narration-stall guard: a model that opens step after step with the same sentence while its
//! tool calls keep changing is stuck, even though no single call repeats.
//!
//! Observed live (2026-09-09, a 27k-message session on muse-spark after a compaction): 23
//! consecutive steps of `You're right — I looped. Answering the 429 question from evidence, then
//! finishing the revert I left half-done.` followed by a *different* read of the same file each
//! time. The identical-call doom-loop guard never fired because the arguments differed; the
//! failure-loop guard never fired because every read succeeded; the step cap was the only thing
//! that would have ended it. The repeated sentence is the structural signal this guard uses.
//!
//! A second, live shape (2026-09-17, `meta::muse-spark-1.3-contributor`) defeats the consecutive
//! check above: the model cycles among ~5 phrasings instead of repeating one, e.g. (labelling each
//! distinct text with a letter, oldest to newest):
//! `A B A C A D E F F E D G C H A I A C C B A D I I G J D C A K B G B B L M M N E O`. Across 40
//! steps only 5 of the 39 adjacent pairs were identical, so `repeats` kept resetting to zero
//! before it ever reached `NUDGE_AFTER` — but one phrasing (`A`) recurred 7 times spanning 750s
//! and another (`C`, by frequency) recurred 5 times spanning 862s. `NarrationTracker` therefore
//! also tracks recurrence across a wider, non-consecutive window (see `FREQ_*` below), so the same
//! handful of phrasings coming back again and again — interleaved with other text — is caught even
//! though no run of consecutive steps repeats.

/// What the guard wants the loop to do after seeing this step's narration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stall {
    /// Nothing to do.
    Fine,
    /// The same statement `NUDGE_AFTER` times running: tell the model once to answer instead.
    Nudge,
    /// Still repeating after the nudge: end the turn.
    Halt,
}

/// Consecutive near-identical openings before the nudge (the fifth statement trips it). One
/// more than the identical-call doom-loop guard needs, so a model that repeats BOTH its sentence
/// and its call gets that guard's more specific diagnosis first.
const NUDGE_AFTER: usize = 4;
/// Further repeats tolerated after the nudge before halting.
const HALT_AFTER_NUDGE: usize = 2;

/// Openings remembered for the comparison. Live, the model alternated between two phrasings
/// ("Reverting the failed live-reuse path …" / "Cutting the failed live-reuse path …"), so
/// comparing with only the previous step saw a change every time.
const WINDOW: usize = 3;

/// Bounded history of recent narration fingerprints used for the frequency check, independent of
/// the consecutive-only `recent`/`repeats` state above. 20 steps comfortably spans the live
/// evidence (the worst phrasing recurred roughly every 5-6 steps over 40) while aging out narration
/// from far earlier in a long, healthy turn, so an incidental echo late in a productive session
/// doesn't outlive its relevance.
const FREQ_WINDOW: usize = 20;
/// Below this many observed steps the frequency check is disabled entirely. Short bursts of
/// mutually-similar text (an exact repeat plus a couple of fuzzy variants) are exactly what the
/// consecutive check above already handles at its own pace; without this floor the frequency check
/// — which scans the *whole* window, not just the last few steps — fires earlier than the
/// consecutive check on those same short bursts and changes when the first `Nudge` is reported.
/// Ten steps is the smallest sample the live evidence's "distinct-fingerprint collapse" signal
/// needs to mean anything, and it's comfortably above every existing consecutive-repeat scenario
/// (which resolves within 5-9 steps).
const FREQ_MIN_HISTORY: usize = 10;
/// A fingerprint (by `similar`) recurring this many times inside the trailing `FREQ_WINDOW` steps
/// is structural repetition rather than progress. Matches `NUDGE_AFTER` so both paths apply the
/// same "four is a pattern" bar — this one just doesn't require the four to be back-to-back. In the
/// live 40-step evidence the worst phrasing reached 4 recurrences by step 15 and the halt-worthy
/// 5th by step 17, both comfortably inside the 20-step window.
const FREQ_NUDGE_AFTER: usize = 4;
/// Further recurrences of the fingerprint that triggered the frequency nudge, tolerated before
/// halting. Matches `HALT_AFTER_NUDGE`.
const FREQ_HALT_AFTER_NUDGE: usize = 2;

#[derive(Debug, Default)]
pub(crate) struct NarrationTracker {
    recent: std::collections::VecDeque<Vec<String>>,
    repeats: usize,
    nudged: bool,
    repeats_since_nudge: usize,
    freq_history: std::collections::VecDeque<Vec<String>>,
    freq_trigger: Option<Vec<String>>,
    freq_repeats_since_nudge: usize,
}

impl NarrationTracker {
    /// Feed the text the model emitted alongside this step's tool calls.
    pub(crate) fn observe(&mut self, text: &str) -> Stall {
        let words = opening_words(text);
        if words.is_empty() {
            return Stall::Fine;
        }

        let freq_stall = self.observe_frequency(&words);
        let consecutive_stall = self.observe_consecutive(&words);

        match (consecutive_stall, freq_stall) {
            (Stall::Halt, _) | (_, Stall::Halt) => Stall::Halt,
            (Stall::Nudge, _) | (_, Stall::Nudge) => Stall::Nudge,
            _ => Stall::Fine,
        }
    }

    fn observe_consecutive(&mut self, words: &[String]) -> Stall {
        let same = self.recent.iter().any(|prev| similar(prev, words));
        self.recent.push_back(words.to_vec());
        if self.recent.len() > WINDOW {
            self.recent.pop_front();
        }
        if !same {
            self.repeats = 0;
            self.nudged = false;
            self.repeats_since_nudge = 0;
            return Stall::Fine;
        }
        self.repeats += 1;
        if self.nudged {
            self.repeats_since_nudge += 1;
            if self.repeats_since_nudge >= HALT_AFTER_NUDGE {
                return Stall::Halt;
            }
            return Stall::Fine;
        }
        if self.repeats >= NUDGE_AFTER {
            self.nudged = true;
            return Stall::Nudge;
        }
        Stall::Fine
    }

    /// Frequency-over-a-window check: unlike `observe_consecutive`, a non-matching step in between
    /// does not reset anything here — that's the whole point (see the module doc's second live
    /// shape). Once a fingerprint has tripped the nudge, only further recurrences of that *same*
    /// fingerprint count toward the halt, so an unrelated later repeat doesn't cash in on someone
    /// else's nudge.
    fn observe_frequency(&mut self, words: &[String]) -> Stall {
        self.freq_history.push_back(words.to_vec());
        if self.freq_history.len() > FREQ_WINDOW {
            self.freq_history.pop_front();
        }
        if self.freq_history.len() < FREQ_MIN_HISTORY {
            return Stall::Fine;
        }

        if let Some(trigger) = &self.freq_trigger {
            if similar(trigger, words) {
                self.freq_repeats_since_nudge += 1;
                if self.freq_repeats_since_nudge >= FREQ_HALT_AFTER_NUDGE {
                    return Stall::Halt;
                }
            }
            return Stall::Fine;
        }

        let count = self
            .freq_history
            .iter()
            .filter(|prev| similar(prev, words))
            .count();
        if count >= FREQ_NUDGE_AFTER {
            self.freq_trigger = Some(words.to_vec());
            self.freq_repeats_since_nudge = 0;
            return Stall::Nudge;
        }
        Stall::Fine
    }

    pub(crate) fn repeats(&self) -> usize {
        self.repeats
    }
}

/// The first sentence-ish of the reply as lowercase alphanumeric words. Only the opening is
/// compared: a stuck model repeats its preamble verbatim, while a working model's reply changes
/// from the first words.
fn opening_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .take(24)
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Near-identical openings. A one-liner like "Reading." never counts (too short to be a
/// preamble). Otherwise: the same first five words, or a word-set overlap of at least 60% — the
/// live loop rephrased its second half every step ("finishing the revert I left half-done",
/// "fixing the broken revert", "pinning the 429 source") around a fixed opening.
fn similar(a: &[String], b: &[String]) -> bool {
    const MIN_WORDS: usize = 5;
    if a.len() < MIN_WORDS || b.len() < MIN_WORDS {
        return false;
    }
    if a[..MIN_WORDS] == b[..MIN_WORDS] {
        return true;
    }
    let sa: std::collections::HashSet<&String> = a.iter().collect();
    let sb: std::collections::HashSet<&String> = b.iter().collect();
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    union > 0 && inter * 5 >= union * 3
}

impl crate::Session {
    /// Feed this step's narration to the guard; queue the nudge or report that the turn must
    /// halt. Lives here so the model loop stays a dispatcher.
    pub(crate) fn narration_stalled(&mut self, tracker: &mut NarrationTracker, text: &str) -> bool {
        match tracker.observe(text) {
            Stall::Fine => false,
            Stall::Nudge => {
                self.presenter
                    .emit(forge_types::PresenterEvent::Warning(format!(
                        "model has opened {} replies in a row with the same sentence without \
                         answering — nudging it to answer before stopping",
                        tracker.repeats() + 1
                    )));
                self.pending_hints.push(STALL_NUDGE.to_string());
                false
            }
            Stall::Halt => {
                self.presenter.emit(forge_types::PresenterEvent::Error(
                    "the model kept repeating the same statement without answering after a \
                     nudge — stopping to avoid a loop"
                        .to_string(),
                ));
                true
            }
        }
    }
}

pub(crate) const STALL_NUDGE: &str = "You have opened your last several replies with the same \
sentence while issuing yet another read, and you have not answered. Stop investigating. Reply \
NOW, in text, with the answer you can give from what you have already read (say what is \
uncertain), then carry on with the remaining work with a concrete next edit or command. Do not \
repeat that opening sentence again.";

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: [&str; 7] = [
        "You're right — I looped. Answering the 429 question from evidence, then finishing the revert I left half-done.",
        "You're right — I looped. Answering the 429 question from evidence, then finishing the revert I left half-done.",
        "You're right — I looped. Answering the 429 question, then finishing the half-done revert.",
        "You're right — I looped. Answering the 429 question from evidence, then fixing the broken revert.",
        "You're right — I looped. Pinning the 429 source, then fixing the broken revert.",
        "You're right — I looped. Answering the 429 question from evidence, then fixing the broken revert.",
        "You're right — I looped. Pinning the 429 source, then fixing the broken revert.",
    ];

    #[test]
    fn the_live_loop_is_nudged_on_the_fifth_statement_and_halted_two_later() {
        let mut t = NarrationTracker::default();
        let verdicts: Vec<Stall> = LIVE.iter().map(|s| t.observe(s)).collect();
        assert_eq!(
            verdicts,
            [
                Stall::Fine,
                Stall::Fine,
                Stall::Fine,
                Stall::Fine,
                Stall::Nudge,
                Stall::Fine,
                Stall::Halt
            ],
            "{verdicts:?}"
        );
    }

    #[test]
    fn two_alternating_phrasings_still_count_as_the_same_stall() {
        let a = "Reverting the failed live-reuse path — keeping the proven wins, restoring fresh-connection + finish PUT.";
        let b = "Cutting the failed live-reuse path and keeping the proven perf wins.";
        let mut t = NarrationTracker::default();
        let verdicts: Vec<Stall> = [a, b, a, b, a, b, a, b]
            .iter()
            .map(|s| t.observe(s))
            .collect();
        assert!(
            verdicts.contains(&Stall::Nudge) && verdicts.last() == Some(&Stall::Halt),
            "{verdicts:?}"
        );
    }

    #[test]
    fn a_changing_opening_resets_everything() {
        let mut t = NarrationTracker::default();
        for s in &LIVE[..5] {
            t.observe(s);
        }
        assert_eq!(
            t.observe("The 429 comes from herodotus: the worker retries without backoff."),
            Stall::Fine
        );
        assert_eq!(t.repeats(), 0);
        assert_eq!(
            t.observe(LIVE[0]),
            Stall::Fine,
            "a fresh streak starts over"
        );
    }

    /// 15 distinct, unrelated narration fingerprints. Deliberately different topics/wording so
    /// `similar` never groups two different letters together — only repeats of the same letter's
    /// exact text count as a recurrence, matching the metadata evidence's "15 distinct texts".
    const TEXTS: [(char, &str); 15] = [
        (
            'A',
            "Reviewing the retry logic before making any further changes here.",
        ),
        (
            'B',
            "Checking the queue depth to understand the current backlog size.",
        ),
        (
            'C',
            "Reading the config file to confirm the timeout value used.",
        ),
        ('D', "Looking at the worker thread to see where it blocks."),
        (
            'E',
            "Inspecting the response headers to find the rate limit source.",
        ),
        (
            'F',
            "Tracing the error path through the client before the retry.",
        ),
        (
            'G',
            "Comparing the two branches to spot the behavioral difference found.",
        ),
        (
            'H',
            "Verifying the schema migration applied correctly to the test database.",
        ),
        (
            'I',
            "Walking through the call stack to locate the actual failure.",
        ),
        (
            'J',
            "Auditing the recent commits for anything touching this shared module.",
        ),
        (
            'K',
            "Measuring the latency spread across the last few sample runs.",
        ),
        (
            'L',
            "Confirming the feature flag state before touching production traffic.",
        ),
        (
            'M',
            "Scanning the logs for any related warning near the crash.",
        ),
        (
            'N',
            "Profiling the hot path to rule out a performance regression.",
        ),
        (
            'O',
            "Summarizing findings so far before deciding on the next step.",
        ),
    ];

    fn text_for(c: char) -> &'static str {
        TEXTS.iter().find(|(k, _)| *k == c).unwrap().1
    }

    #[test]
    fn the_interleaved_live_pattern_eventually_nudges_then_halts() {
        // Ordered oldest -> newest, exactly the live distinct-text pattern (metadata evidence,
        // 2026-09-17): 'A' recurs 7 times, 'C' 5 times, cycling among ~5 phrasings overall, with
        // only 5 of the 39 adjacent pairs identical. The consecutive check alone never fires on
        // this; the frequency check must.
        const PATTERN: &str = "ABACADEFFEDGCHAIACCBADIIGJDCAKBGBBLMMNEO";
        assert_eq!(PATTERN.len(), 40);
        let mut t = NarrationTracker::default();
        let verdicts: Vec<Stall> = PATTERN.chars().map(|c| t.observe(text_for(c))).collect();
        let nudge_at = verdicts.iter().position(|s| *s == Stall::Nudge);
        let halt_at = verdicts.iter().position(|s| *s == Stall::Halt);
        assert!(nudge_at.is_some(), "never nudged: {verdicts:?}");
        assert!(
            halt_at.is_some(),
            "never halted, would run forever: {verdicts:?}"
        );
        assert!(
            halt_at.unwrap() > nudge_at.unwrap(),
            "halt must come after nudge: {verdicts:?}"
        );
    }

    #[test]
    fn a_strictly_consecutive_repeat_still_nudges_on_the_fourth() {
        let text = text_for('A');
        let mut t = NarrationTracker::default();
        let verdicts: Vec<Stall> = std::iter::repeat_n(text, 5).map(|s| t.observe(s)).collect();
        assert_eq!(
            verdicts,
            [
                Stall::Fine,
                Stall::Fine,
                Stall::Fine,
                Stall::Fine,
                Stall::Nudge
            ],
            "{verdicts:?}"
        );
    }

    #[test]
    fn a_genuine_multi_step_turn_with_different_narration_each_step_never_trips() {
        let mut t = NarrationTracker::default();
        for (_, text) in TEXTS.iter() {
            assert_eq!(t.observe(text), Stall::Fine, "false positive on {text:?}");
        }
        // Run through the set a second time with a rewording each pass, still never repeating a
        // fingerprint, well past FREQ_MIN_HISTORY.
        for (_, text) in TEXTS.iter() {
            let reworded = format!("Also: {text} Nothing further to add on that one right now.");
            assert_eq!(
                t.observe(&reworded),
                Stall::Fine,
                "false positive on {reworded:?}"
            );
        }
    }

    #[test]
    fn a_repeated_two_word_acknowledgement_never_trips() {
        let mut t = NarrationTracker::default();
        for _ in 0..2 {
            assert_eq!(t.observe("Got it."), Stall::Fine);
        }
        for (_, text) in TEXTS.iter() {
            assert_eq!(t.observe(text), Stall::Fine, "false positive on {text:?}");
        }
    }

    #[test]
    fn short_or_empty_narration_never_trips() {
        let mut t = NarrationTracker::default();
        for _ in 0..10 {
            assert_eq!(t.observe(""), Stall::Fine);
            assert_eq!(t.observe("Reading."), Stall::Fine);
        }
    }

    #[test]
    fn ordinary_progress_narration_is_not_similar() {
        assert!(!similar(
            &opening_words("Now checking the worker's retry path in worker.rs."),
            &opening_words("Now applying the backoff fix to the worker and re-running the tests."),
        ));
        assert!(similar(&opening_words(LIVE[0]), &opening_words(LIVE[2])));
        assert!(similar(&opening_words(LIVE[3]), &opening_words(LIVE[4])));
    }
}
