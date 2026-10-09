//! Headroom guarantee for auto-compaction.

use super::*;
use crate::compaction_shape::{clip_tool_results_to_budget, compact_target_tokens};

/// Above this share of the trigger after a compaction, headroom is forced by clipping tool output.
const COMPACT_CLIP_ABOVE_PERCENT: u64 = 75;

impl Session {
    /// Guarantee an auto-compaction leaves real headroom. A summary only replaces the older part;
    /// when the verbatim tail (or a failed summarization) leaves the transcript above
    /// [`COMPACT_CLIP_ABOVE_PERCENT`] of the trigger, the next tool result would trip another
    /// compaction at once, so the bulkiest tool results are clipped down to the target instead.
    pub(crate) fn enforce_post_compact_headroom(&mut self, trigger: u64) {
        if self.estimated_transcript_tokens() <= trigger * COMPACT_CLIP_ABOVE_PERCENT / 100 {
            return;
        }
        let target = usize::try_from(compact_target_tokens(trigger)).unwrap_or(usize::MAX);
        let reclaimed = clip_tool_results_to_budget(&mut self.transcript, target);
        if reclaimed > 0 {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "compaction left the context near its ceiling; clipped bulky tool output \
                 (~{reclaimed} tokens) to leave headroom"
            )));
        }
    }
}
