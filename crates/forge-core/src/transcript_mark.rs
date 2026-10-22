//! Positions in the transcript that survive compaction.
//!
//! A turn often remembers "where the transcript ended" before a model loop, then inspects only the
//! messages that loop added. A bare `transcript.len()` cannot be that marker: auto-compaction runs
//! inside the loop and replaces the older part of the transcript with one summary message, so the
//! saved index either points past the end (the `range start index 16 out of range for slice of
//! length 7` panic that killed `forge bench tasks` after a completeness audit) or, worse, at the
//! wrong messages. A [`TranscriptMark`] records an absolute position instead, which compaction
//! keeps valid by moving `Session::transcript_origin`.

use super::*;

/// Where the transcript ended when the mark was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptMark {
    /// `history_epoch` at the mark. Every rewrite bumps it, compaction included.
    epoch: u64,
    /// Compactions at the mark, so the epoch bumps they explain can be told from other rewrites.
    compactions: u64,
    /// Absolute position of the first message added after the mark.
    absolute: usize,
}

/// The index in the current transcript where the messages after `mark` start, or `None` when the
/// transcript was rewritten in a way no position survives (rewind, uncompact, a full reload).
///
/// Compaction folds `transcript[..split]` into one summary at index 0 and moves `origin` forward
/// by `split - 1`, so the kept tail keeps its absolute positions. A mark that fell inside the
/// folded part resolves to index 1: the summary mixes what came before the mark with what came
/// after it, and only the tail is unambiguously "after".
fn since_index(mark: TranscriptMark, epoch: u64, compactions: u64, origin: usize) -> Option<usize> {
    let compacted = compactions.saturating_sub(mark.compactions);
    let rewrites = epoch.saturating_sub(mark.epoch);
    if rewrites > compacted {
        return None;
    }
    let start = mark.absolute.saturating_sub(origin);
    Some(if compacted > 0 { start.max(1) } else { start })
}

impl Session {
    /// Mark the current end of the transcript.
    pub(crate) fn transcript_mark(&self) -> TranscriptMark {
        TranscriptMark {
            epoch: self.history_epoch,
            compactions: self.compactions,
            absolute: self.transcript_origin + self.transcript.len(),
        }
    }

    /// The messages added since `mark` that are still in the transcript. Empty when the
    /// transcript was rewritten past recognition, which callers read as "no evidence".
    pub(crate) fn transcript_since(&self, mark: TranscriptMark) -> &[Message] {
        since_index(
            mark,
            self.history_epoch,
            self.compactions,
            self.transcript_origin,
        )
        .and_then(|start| self.transcript.get(start..))
        .unwrap_or_default()
    }

    /// Record that compaction replaced `transcript[..split]` with one summary message.
    pub(crate) fn note_transcript_folded(&mut self, split: usize) {
        self.transcript_origin += split.saturating_sub(1);
        self.compactions += 1;
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn mark(epoch: u64, compactions: u64, absolute: usize) -> TranscriptMark {
        TranscriptMark {
            epoch,
            compactions,
            absolute,
        }
    }

    #[test]
    fn an_untouched_transcript_resolves_to_the_saved_length() {
        assert_eq!(since_index(mark(3, 0, 16), 3, 0, 0), Some(16));
        assert_eq!(since_index(mark(0, 0, 0), 0, 0, 0), Some(0));
    }

    #[test]
    fn a_mark_in_the_kept_tail_moves_with_it() {
        // 20 messages, mark at 16, compaction folds the first 12 into one summary: origin 11.
        assert_eq!(since_index(mark(0, 0, 16), 1, 1, 11), Some(5));
    }

    #[test]
    fn a_mark_inside_the_folded_part_starts_after_the_summary() {
        assert_eq!(since_index(mark(0, 0, 4), 1, 1, 11), Some(1));
    }

    #[test]
    fn a_rewrite_that_is_not_a_compaction_invalidates_the_mark() {
        assert_eq!(since_index(mark(2, 0, 16), 3, 0, 0), None);
        assert_eq!(since_index(mark(2, 0, 16), 4, 1, 11), None);
    }
}
