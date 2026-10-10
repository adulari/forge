//! Headroom guarantee for auto-compaction.

use super::*;
use crate::compaction_shape::{clip_tool_results_to_budget, compact_target_tokens};

/// Above this share of the trigger after a compaction, headroom is forced by clipping tool output.
const COMPACT_CLIP_ABOVE_PERCENT: u64 = 75;

impl Session {
    /// Refresh what the CLI bridge says its context holds after a model call, and return the live
    /// context fill for the gauge.
    pub(crate) fn note_context_fill(&mut self, model: &str, reported_input: u64) -> u64 {
        if !forge_provider::is_cli_bridge(model) {
            self.bridge_context_tokens = 0;
        } else if let Some(fill) = self.provider.context_fill(model, &self.id) {
            // A missing reading (the slot was busy, the call ran one-shot) says nothing about the
            // window shrinking — only a compaction or a respawn does, and those reset it.
            self.bridge_context_tokens = fill;
        }
        context_fill_tokens(model, self.context_pressure_tokens(), reported_input)
    }

    /// The context size compaction and the gauge should act on: the transcript Forge manages, or
    /// the CLI bridge's own context when that is larger. A bridged claude runs every tool inside
    /// its own process, so file reads and build logs pile up there while the transcript Forge
    /// estimates stays a fraction of the size — a measured session sat at ~12k estimated tokens
    /// with 400k in claude's window, and never compacted.
    pub(crate) fn context_pressure_tokens(&self) -> u64 {
        self.estimated_transcript_tokens()
            .max(self.bridge_context_tokens)
    }

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
