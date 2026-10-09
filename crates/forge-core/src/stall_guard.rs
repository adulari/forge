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

//!
//! Narration alone is a poor judge of a loop, in both directions. Observed live (2026-09-08/09,
//! sessions 07ca114e and 432f9364): ~30 steps of `You're right — I looped. ...` slipped past the
//! consecutive check because the fifth word kept changing, and the user had to type "you are
//! getting stuck in a loop" three times before the guard halted it; conversely a model opening
//! seven successful edit/test steps with `Now let me update the ...` is exactly as repetitive in
//! text and exactly as healthy in practice. This tracker therefore only reports which steps
//! REPEAT an earlier opening; whether that repetition matters is decided by
//! [`crate::loop_progress`], which knows whether the step produced anything new.

/// Openings remembered for the comparison. Live, the model alternated between two phrasings
/// ("Reverting the failed live-reuse path …" / "Cutting the failed live-reuse path …"), so
/// comparing with only the previous step saw a change every time.
const WINDOW: usize = 3;

/// Bounded history of recent narration fingerprints used for the frequency check, independent of
/// the consecutive-only `recent` window above. 20 steps comfortably spans the live evidence (the
/// worst phrasing recurred roughly every 5-6 steps over 40) while aging out narration from far
/// earlier in a long, healthy turn.
const FREQ_WINDOW: usize = 20;
/// Below this many stagnant steps the frequency check is disabled: short bursts of
/// mutually-similar text are what the consecutive check already handles.
const FREQ_MIN_HISTORY: usize = 6;
/// A fingerprint (by `similar`) recurring this many times inside the trailing `FREQ_WINDOW`
/// stagnant steps is structural repetition even when no two adjacent steps match.
const FREQ_REPEAT_AT: usize = 4;

#[derive(Debug, Default)]
pub(crate) struct NarrationTracker {
    recent: std::collections::VecDeque<Vec<String>>,
    /// Openings of steps that produced nothing new. Steps that made progress never enter, so a
    /// productive run of `Now let me update …` cannot accumulate into a frequency hit.
    freq_history: std::collections::VecDeque<Vec<String>>,
    /// How many times each EXACT statement has been made on a stagnant step this turn. Verbatim
    /// repetition is unambiguous, so the sampling ladder (see [`Self::max_verbatim`]) is driven by
    /// this rather than by the deliberately fuzzy detectors.
    exact_counts: std::collections::HashMap<String, usize>,
}

impl NarrationTracker {
    /// Feed the text the model emitted alongside this step's tool calls. `novel` is whether the
    /// step produced anything new (see [`crate::loop_progress`]). Returns whether this step is a
    /// stale repeat: it restates an earlier opening AND got nothing new out of its tool calls.
    pub(crate) fn observe(&mut self, text: &str, novel: bool) -> bool {
        let words = opening_words(text);
        if words.is_empty() {
            return false;
        }
        let like_recent = self.recent.iter().any(|prev| similar(prev, &words));
        self.recent.push_back(words.clone());
        if self.recent.len() > WINDOW {
            self.recent.pop_front();
        }
        if novel {
            return false;
        }

        *self.exact_counts.entry(words.join(" ")).or_insert(0) += 1;
        self.freq_history.push_back(words.clone());
        if self.freq_history.len() > FREQ_WINDOW {
            self.freq_history.pop_front();
        }
        let like_often = self.freq_history.len() >= FREQ_MIN_HISTORY
            && self
                .freq_history
                .iter()
                .filter(|prev| similar(prev, &words))
                .count()
                >= FREQ_REPEAT_AT;
        like_recent || like_often
    }

    /// The most times any one verbatim statement has been made on a stagnant step.
    pub(crate) fn max_verbatim(&self) -> usize {
        self.exact_counts.values().copied().max().unwrap_or(0)
    }

    /// A new user turn: what was said before is no longer evidence of anything stagnant, but the
    /// last few openings stay so a model that opens the next turn with the same sentence is still
    /// compared against it.
    pub(crate) fn begin_turn(&mut self) {
        self.freq_history.clear();
        self.exact_counts.clear();
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
/// preamble). Otherwise: the same first four words, or a word-set overlap of at least 60% — the
/// live loop rephrased its second half every step ("finishing the revert I left half-done",
/// "fixing the broken revert", "pinning the 429 source") around a fixed opening, and its
/// fifth word changed every time ("You're right — I looped. Answering / Recovering / I'll …"), so
/// a five-word prefix let thirty near-identical apologies through.
fn similar(a: &[String], b: &[String]) -> bool {
    const MIN_WORDS: usize = 5;
    const PREFIX_WORDS: usize = 4;
    if a.len() < MIN_WORDS || b.len() < MIN_WORDS {
        return false;
    }
    if a[..PREFIX_WORDS] == b[..PREFIX_WORDS] {
        return true;
    }
    let sa: std::collections::HashSet<&String> = a.iter().collect();
    let sb: std::collections::HashSet<&String> = b.iter().collect();
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    union > 0 && inter * 5 >= union * 3
}

pub(crate) const STALL_NUDGE: &str = "You have opened your last several replies with the same \
sentence while your tool calls kept returning nothing you had not already seen, and you have not \
answered. Stop investigating. Reply NOW, in text, with the answer you can give from what you \
have already read (say what is uncertain), then carry on with the remaining work with a concrete \
next edit or command. Do not repeat that opening sentence again, and do not apologise for \
looping — act.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed every text as a stagnant (nothing-new) step and return the stale-repeat flags.
    fn stale_flags(texts: &[&str]) -> Vec<bool> {
        let mut t = NarrationTracker::default();
        texts.iter().map(|s| t.observe(s, false)).collect()
    }

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
    fn the_live_loop_restates_from_the_second_statement_on() {
        assert_eq!(
            stale_flags(&LIVE),
            [false, true, true, true, true, true, true]
        );
    }

    #[test]
    fn two_alternating_phrasings_still_count_as_the_same_stall() {
        let a = "Reverting the failed live-reuse path — keeping the proven wins, restoring fresh-connection + finish PUT.";
        let b = "Cutting the failed live-reuse path and keeping the proven perf wins.";
        let flags = stale_flags(&[a, b, a, b, a, b]);
        assert!(flags[2..].iter().all(|f| *f), "{flags:?}");
    }

    #[test]
    fn a_step_that_found_something_new_is_never_a_stale_repeat() {
        let mut t = NarrationTracker::default();
        for _ in 0..8 {
            assert!(
                !t.observe(LIVE[0], true),
                "progress makes repetition harmless"
            );
        }
        assert_eq!(t.max_verbatim(), 0, "productive steps don't build pressure");
        assert!(
            t.observe(LIVE[0], false),
            "the first stagnant restatement counts"
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
    fn the_interleaved_live_pattern_is_flagged_repeatedly() {
        // The live distinct-text pattern (2026-09-17): 'A' recurs 7 times and 'C' 5 times with only
        // 5 of 39 adjacent pairs identical. The consecutive window misses most of it; the
        // frequency window must flag a steady stream.
        const PATTERN: &str = "ABACADEFFEDGCHAIACCBADIIGJDCAKBGBBLMMNEO";
        let texts: Vec<&str> = PATTERN.chars().map(text_for).collect();
        let flags = stale_flags(&texts);
        let hits = flags.iter().filter(|f| **f).count();
        assert!(hits >= 12, "only {hits} stale repeats flagged: {flags:?}");
    }

    #[test]
    fn verbatim_counts_ignore_productive_steps() {
        let mut t = NarrationTracker::default();
        for _ in 0..5 {
            t.observe(
                "Now let me update the parser to handle the new header",
                true,
            );
        }
        assert_eq!(t.max_verbatim(), 0);
        t.observe(
            "Inspecting the diff to identify the missing fix, then applying it",
            false,
        );
        t.observe(
            "Inspecting the diff to identify the missing fix, then applying it",
            false,
        );
        assert_eq!(t.max_verbatim(), 2);
    }

    #[test]
    fn a_genuine_multi_step_turn_with_different_narration_each_step_never_trips() {
        let mut t = NarrationTracker::default();
        for (_, text) in TEXTS.iter() {
            assert!(!t.observe(text, false), "false positive on {text:?}");
        }
        // Run through the set a second time with a rewording each pass, still never repeating a
        // fingerprint, well past FREQ_MIN_HISTORY.
        for (_, text) in TEXTS.iter() {
            let reworded = format!("Also: {text} Nothing further to add on that one right now.");
            assert!(
                !t.observe(&reworded, false),
                "false positive on {reworded:?}"
            );
        }
    }

    #[test]
    fn a_repeated_two_word_acknowledgement_never_trips() {
        let mut t = NarrationTracker::default();
        for _ in 0..2 {
            assert!(!t.observe("Got it.", false));
        }
        for (_, text) in TEXTS.iter() {
            assert!(!t.observe(text, false), "false positive on {text:?}");
        }
    }

    #[test]
    fn short_or_empty_narration_never_trips() {
        let mut t = NarrationTracker::default();
        for _ in 0..10 {
            assert!(!t.observe("", false));
            assert!(!t.observe("Reading.", false));
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
