//! What a compaction may put back, budgeted from the trigger that fired it.
//!
//! The ceilings in `compaction_summary`, `compaction_shape` and `file_restore` were sized for a
//! large window. Against a 24k trigger the verbatim block, the kept rounds, a 20k summary and 40k of
//! restored files each exceed it alone, so a fresh compaction landed above the trigger and the next
//! step compacted again (43 summaries in one 38-turn scenario). Every share below is a fraction of
//! the trigger, capped at the old ceiling, so a big window behaves as before and a small one
//! leaves room for work.

use crate::compaction_shape::{compact_target_tokens, COMPACT_KEEP_ROUNDS_TOKENS};
use crate::compaction_summary::{USER_MESSAGES_BUDGET_TOKENS, USER_MESSAGE_OLDER_MAX_CHARS};

/// Output ceiling of one summary call (scratch section plus summary).
pub(crate) const COMPACT_OUTPUT_TOKENS: usize = 20_000;

const SUMMARY_PERCENT: u64 = 18;
const KEEP_ROUNDS_PERCENT: u64 = 14;
const VERBATIM_PERCENT: u64 = 8;

const SUMMARY_FLOOR_TOKENS: usize = 1_000;
const VERBATIM_FLOOR_TOKENS: usize = 400;
/// The scratch section shares the call's output limit with the summary, so the call may write this
/// many times the summary's size before it is cut off.
const OUTPUT_PER_SUMMARY_TOKEN: usize = 3;
/// A summary over this many times its target is cut down (middle dropped, start and end kept).
const SUMMARY_SLACK_PERCENT: usize = 150;

/// A context must grow by this share of the trigger past where a compaction left it before the
/// next auto-compaction may fire.
const REGROWTH_PERCENT: u64 = 25;

fn share(trigger: u64, percent: u64, floor: usize, ceiling: usize) -> usize {
    let tokens = usize::try_from(trigger.saturating_mul(percent) / 100).unwrap_or(usize::MAX);
    tokens.clamp(floor.min(ceiling), ceiling)
}

/// The room each part of a post-compaction context gets, in tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactionBudget {
    /// The number auto-compaction compares against.
    pub trigger: u64,
    /// Where the compaction should leave the context in total.
    pub target: u64,
    /// Size the summary body should stay within.
    pub summary_tokens: usize,
    /// Verbatim user-message section.
    pub verbatim_tokens: usize,
    /// Recent tool rounds kept next to the summary.
    pub keep_rounds_tokens: usize,
    /// `[harness] compaction_summary_cap` held the summary below its normal share.
    pub summary_capped: bool,
}

impl CompactionBudget {
    /// Budget for a compaction against `trigger`, with `pinned_tokens` of standing context that
    /// rides on every request and cannot shrink.
    ///
    /// When the room under the target is tighter than the shares, the parts give way in this order:
    /// restored files (they get whatever is left, see [`Self::restore_room`]), then the verbatim
    /// section, then the kept rounds, then the summary.
    pub(crate) fn for_trigger(trigger: u64, pinned_tokens: usize) -> Self {
        let target = compact_target_tokens(trigger);
        let mut room = usize::try_from(target)
            .unwrap_or(usize::MAX)
            .saturating_sub(pinned_tokens);
        let mut take = |wanted: usize| {
            let granted = wanted.min(room);
            room -= granted;
            granted
        };
        let summary_tokens = take(share(
            trigger,
            SUMMARY_PERCENT,
            SUMMARY_FLOOR_TOKENS,
            COMPACT_OUTPUT_TOKENS,
        ))
        .max(SUMMARY_FLOOR_TOKENS);
        let keep_rounds_tokens = take(share(
            trigger,
            KEEP_ROUNDS_PERCENT,
            0,
            COMPACT_KEEP_ROUNDS_TOKENS,
        ));
        let verbatim_tokens = take(share(
            trigger,
            VERBATIM_PERCENT,
            VERBATIM_FLOOR_TOKENS,
            USER_MESSAGES_BUDGET_TOKENS,
        ))
        .max(VERBATIM_FLOOR_TOKENS);
        Self {
            trigger,
            target,
            summary_tokens,
            verbatim_tokens,
            keep_rounds_tokens,
            summary_capped: false,
        }
    }

    /// The budget with the summary held to at most `cap` tokens.
    pub(crate) fn with_summary_cap(mut self, cap: Option<usize>) -> Self {
        if let Some(cap) = cap.filter(|cap| *cap < self.summary_tokens) {
            self.summary_tokens = cap.max(SUMMARY_FLOOR_TOKENS);
            self.summary_capped = true;
        }
        self
    }

    /// Whether the summary is held below its old ceiling, which is when the summarizer must be
    /// told the size.
    pub(crate) fn summary_is_scaled(&self) -> bool {
        self.summary_capped || self.summary_tokens < COMPACT_OUTPUT_TOKENS / 2
    }

    /// `max_output_tokens` of the summary call: the summary's size plus the scratch section it is
    /// written after, never past the old ceiling.
    pub(crate) fn output_tokens(&self) -> usize {
        (self.summary_tokens * OUTPUT_PER_SUMMARY_TOKEN).clamp(4_096, COMPACT_OUTPUT_TOKENS)
    }

    /// Tokens a summary body may reach before it is cut down.
    pub(crate) fn summary_hard_cap(&self) -> usize {
        self.summary_tokens * SUMMARY_SLACK_PERCENT / 100
    }

    /// Longest an older user message stays in the verbatim section.
    pub(crate) fn verbatim_older_chars(&self) -> usize {
        (self.verbatim_tokens / 4).clamp(300, USER_MESSAGE_OLDER_MAX_CHARS)
    }

    /// Tokens restored files may add to a context that is `current` tokens now: what is left under
    /// the target, never more than `ceiling`.
    pub(crate) fn restore_room(&self, current: u64, ceiling: usize) -> usize {
        let left = self.target.saturating_sub(current);
        usize::try_from(left).unwrap_or(usize::MAX).min(ceiling)
    }
}

/// The size the summarizer is asked to stay within, when that is smaller than it would write
/// unprompted.
pub(crate) fn size_directive(summary_tokens: usize) -> String {
    format!(
        "Keep the summary under about {summary_tokens} tokens and the <analysis> section short: \
         this conversation's context window is small. Spend the space on exact names, paths, \
         numbers, decisions, pending work and the current task; leave out narrative and \
         anything the verbatim user messages already carry."
    )
}

/// Where the last compaction left the session, so a context that has not grown since is not
/// summarized again.
#[derive(Debug, Default)]
pub(crate) struct ThrashGuard {
    mark: Option<(String, u64)>,
    noticed: bool,
}

impl ThrashGuard {
    /// Record the context size a compaction of `session` ended at.
    pub(crate) fn note_compaction(&mut self, session: &str, tokens: u64) {
        self.mark = Some((session.to_string(), tokens));
    }

    /// Whether an auto-compaction must wait: the context sits within a quarter of the trigger of
    /// where the last compaction left it, so a summary would fold almost nothing new.
    pub(crate) fn holds(&self, session: &str, pressure: u64, trigger: u64) -> bool {
        match &self.mark {
            Some((id, mark)) if id == session => {
                pressure < mark.saturating_add(trigger.saturating_mul(REGROWTH_PERCENT) / 100)
            }
            _ => false,
        }
    }

    /// True once per session: the first time the notice about an irreducible context is due.
    pub(crate) fn first_notice(&mut self) -> bool {
        !std::mem::replace(&mut self.noticed, true)
    }

    /// Tokens of regrowth that release the guard.
    pub(crate) fn regrowth(trigger: u64) -> u64 {
        trigger.saturating_mul(REGROWTH_PERCENT) / 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_cap_lowers_a_big_window_and_leaves_a_small_one() {
        let big = CompactionBudget::for_trigger(838_860, 0);
        assert_eq!(big.summary_tokens, COMPACT_OUTPUT_TOKENS);
        assert!(!big.summary_is_scaled() || big.summary_capped);
        let capped = big.with_summary_cap(Some(16_000));
        assert_eq!(capped.summary_tokens, 16_000);
        assert!(
            capped.summary_is_scaled(),
            "the summarizer is told the size"
        );
        assert!(capped.output_tokens() <= COMPACT_OUTPUT_TOKENS);

        let small = CompactionBudget::for_trigger(24_000, 0);
        let kept = small.with_summary_cap(Some(16_000));
        assert_eq!(kept.summary_tokens, small.summary_tokens);
        assert!(!kept.summary_capped);
        assert_eq!(
            big.with_summary_cap(None).summary_tokens,
            COMPACT_OUTPUT_TOKENS
        );
    }

    const PINNED: usize = 1_200;

    fn total(budget: &CompactionBudget) -> usize {
        budget.summary_tokens + budget.verbatim_tokens + budget.keep_rounds_tokens
    }

    #[test]
    fn the_parts_of_a_small_trigger_fit_half_of_it_with_the_pinned_block() {
        for trigger in [24_000u64, 32_000, 64_000] {
            let budget = CompactionBudget::for_trigger(trigger, PINNED);
            assert!(
                (total(&budget) + PINNED) as u64 <= budget.target,
                "{trigger}: {budget:?}"
            );
            assert_eq!(budget.target, trigger / 2);
        }
    }

    #[test]
    fn a_small_trigger_scales_every_ceiling_down() {
        let small = CompactionBudget::for_trigger(24_000, PINNED);
        assert!(small.summary_tokens < 5_000, "{small:?}");
        assert!(small.keep_rounds_tokens < 4_000, "{small:?}");
        assert!(small.verbatim_tokens < 2_500, "{small:?}");
        assert!(small.summary_is_scaled());
        assert!(small.verbatim_older_chars() < USER_MESSAGE_OLDER_MAX_CHARS);
        assert!(small.output_tokens() < COMPACT_OUTPUT_TOKENS);
    }

    #[test]
    fn a_million_token_window_keeps_the_old_ceilings() {
        // The free-model trigger of a 1M window: 80% of it.
        let budget = CompactionBudget::for_trigger(838_860, PINNED);
        assert_eq!(budget.summary_tokens, COMPACT_OUTPUT_TOKENS);
        assert_eq!(budget.keep_rounds_tokens, COMPACT_KEEP_ROUNDS_TOKENS);
        assert_eq!(budget.verbatim_tokens, USER_MESSAGES_BUDGET_TOKENS);
        assert_eq!(budget.verbatim_older_chars(), USER_MESSAGE_OLDER_MAX_CHARS);
        assert_eq!(budget.output_tokens(), COMPACT_OUTPUT_TOKENS);
        assert!(!budget.summary_is_scaled());
    }

    #[test]
    fn restored_files_get_only_what_is_left_under_the_target() {
        let budget = CompactionBudget::for_trigger(24_000, PINNED);
        assert_eq!(budget.restore_room(9_000, 40_000), 3_000);
        assert_eq!(budget.restore_room(12_000, 40_000), 0);
        assert_eq!(budget.restore_room(30_000, 40_000), 0);
        assert_eq!(budget.restore_room(0, 5_000), 5_000);
    }

    #[test]
    fn a_big_pinned_block_squeezes_the_verbatim_section_before_the_kept_rounds() {
        let roomy = CompactionBudget::for_trigger(24_000, PINNED);
        let mid = CompactionBudget::for_trigger(24_000, 4_000);
        assert!(mid.verbatim_tokens < roomy.verbatim_tokens);
        assert_eq!(mid.keep_rounds_tokens, roomy.keep_rounds_tokens);
        assert_eq!(mid.summary_tokens, roomy.summary_tokens);

        let tight = CompactionBudget::for_trigger(24_000, 9_000);
        assert!(tight.keep_rounds_tokens < roomy.keep_rounds_tokens);
        assert!(tight.summary_tokens >= SUMMARY_FLOOR_TOKENS);
        assert!(tight.verbatim_tokens >= VERBATIM_FLOOR_TOKENS);
    }

    #[test]
    fn the_guard_holds_until_the_context_regrows_a_quarter_of_the_trigger() {
        let mut guard = ThrashGuard::default();
        assert!(!guard.holds("s", 30_000, 24_000), "no compaction yet");
        guard.note_compaction("s", 20_000);
        assert!(guard.holds("s", 20_000, 24_000), "straight after");
        assert!(guard.holds("s", 25_999, 24_000));
        assert!(
            !guard.holds("s", 26_000, 24_000),
            "grown by 6k = 25% of 24k"
        );
        assert!(!guard.holds("other", 20_000, 24_000), "another session");
    }

    #[test]
    fn the_irreducible_notice_is_given_once() {
        let mut guard = ThrashGuard::default();
        assert!(guard.first_notice());
        assert!(!guard.first_notice());
    }
}
