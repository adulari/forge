//! Narration-stall guard: a model that opens step after step with the same sentence while its
//! tool calls keep changing is stuck, even though no single call repeats.
//!
//! Observed live (2026-09-09, a 27k-message session on muse-spark after a compaction): 23
//! consecutive steps of `You're right — I looped. Answering the 429 question from evidence, then
//! finishing the revert I left half-done.` followed by a *different* read of the same file each
//! time. The identical-call doom-loop guard never fired because the arguments differed; the
//! failure-loop guard never fired because every read succeeded; the step cap was the only thing
//! that would have ended it. The repeated sentence is the structural signal this guard uses.

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

#[derive(Debug, Default)]
pub(crate) struct NarrationTracker {
    recent: std::collections::VecDeque<Vec<String>>,
    repeats: usize,
    nudged: bool,
    repeats_since_nudge: usize,
}

impl NarrationTracker {
    /// Feed the text the model emitted alongside this step's tool calls.
    pub(crate) fn observe(&mut self, text: &str) -> Stall {
        let words = opening_words(text);
        if words.is_empty() {
            return Stall::Fine;
        }
        let same = self.recent.iter().any(|prev| similar(prev, &words));
        self.recent.push_back(words);
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
