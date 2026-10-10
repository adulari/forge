//! Headroom guarantee for auto-compaction.

use super::*;
use crate::compaction_shape::{clip_tool_results_to_budget, compact_target_tokens};

/// The summarizer candidates the user's configuration still allows. The main loop refuses a model
/// that is in `mesh.disabled` or has no key before dispatching it; the compaction chain skipped
/// that check, so a model the user had disabled still received the whole transcript to summarize
/// (measured: 138k tokens of a session's code sent to a disabled provider). The session's own
/// model is always kept — it is the one the user chose and the chain's guaranteed last resort.
pub(crate) fn callable_summarizers(
    models: Vec<String>,
    guaranteed: &str,
    disabled: &[String],
    has_key: impl Fn(&str) -> bool,
) -> Vec<String> {
    models
        .into_iter()
        .filter(|model| {
            model == guaranteed
                || (!forge_config::is_model_disabled(model, disabled)
                    && has_key(forge_config::provider_of(model)))
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::callable_summarizers;

    fn list(models: &[&str]) -> Vec<String> {
        models.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn a_disabled_provider_or_model_never_receives_the_transcript() {
        let out = callable_summarizers(
            list(&[
                "gemini::gemini-2.5-flash",
                "groq::llama-3.1-8b",
                "groq::big",
            ]),
            "claude-cli::haiku",
            &list(&["gemini", "groq::big"]),
            |_| true,
        );
        assert_eq!(out, list(&["groq::llama-3.1-8b"]));
    }

    #[test]
    fn a_provider_without_a_key_is_skipped() {
        let out = callable_summarizers(
            list(&["groq::fast", "ollama::llama3.2"]),
            "claude-cli::haiku",
            &[],
            |provider| provider == "ollama",
        );
        assert_eq!(out, list(&["ollama::llama3.2"]));
    }

    #[test]
    fn the_sessions_own_model_is_kept_even_when_it_is_listed_as_disabled() {
        let out = callable_summarizers(
            list(&["claude-cli::haiku"]),
            "claude-cli::haiku",
            &list(&["claude-cli"]),
            |_| false,
        );
        assert_eq!(out, list(&["claude-cli::haiku"]));
    }
}
