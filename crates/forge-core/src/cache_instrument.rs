//! Prompt-cache attribution: which code path made a request miss the provider's cache.
//!
//! Each main-loop request is fingerprinted at the wire boundary
//! ([`forge_provider::wire_fingerprint`]) and compared with the previous request to the same model.
//! The comparison plus the usage the provider reports gives every call one label, persisted on its
//! usage row and rolled up by `forge stats cache`:
//!
//! - `append` — the previous request survives whole at the front; the ideal.
//! - `harness_rewrite@<idx>(<kind>)` — Forge rewrote an already-sent message with no deliberate
//!   reason recorded. The regression this exists to catch; two in a row raise a warning.
//! - `floor_advance` / `age_rounds` / `compaction` — a deliberate rewrite (the elision floor moved
//!   past earlier turns / past rounds of the current turn, or a summary or post-compaction clip
//!   replaced history).
//! - `history_rewrite` — the user rewound the session or reloaded its full uncompacted history.
//! - `tools_changed` — the tool list differs from the previous request.
//! - `model_switch` / `cold` — a different model than last time / the first request.
//! - `provider_eviction` — the prefix was intact but most of the input was not served from cache.
//! - `n/a` — a CLI bridge, which reports cumulative usage and cannot be fingerprinted.

use std::collections::HashMap;

use forge_provider::wire_fingerprint::{fingerprint, PrefixReuse, WireFingerprint};
use forge_provider::ToolSpec;
use forge_store::UsagePrefix;
use forge_types::{Message, PresenterEvent, Usage};

use crate::context_floor::FloorMove;
use crate::{CoreError, Session};

/// Below this many input tokens a cache miss on an intact prefix says nothing: providers do not
/// cache short prompts at all (Anthropic's minimum is 1,024 tokens).
const MIN_CACHEABLE_INPUT: u64 = 2_048;

/// Consecutive unexplained rewrites that raise a warning.
const REWRITE_STREAK_WARNING: u32 = 2;

/// A deliberate rewrite of already-sent context, recorded before the request that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewriteReason {
    FloorAdvance,
    AgeRounds,
    Compaction,
    HistoryRewrite,
}

impl RewriteReason {
    fn label(self) -> &'static str {
        match self {
            RewriteReason::FloorAdvance => "floor_advance",
            RewriteReason::AgeRounds => "age_rounds",
            RewriteReason::Compaction => "compaction",
            RewriteReason::HistoryRewrite => "history_rewrite",
        }
    }
}

/// Per-session state: the previous request's fingerprint per model.
#[derive(Debug, Default)]
pub(crate) struct CacheInstrument {
    previous: HashMap<String, WireFingerprint>,
    last_model: Option<String>,
    staged: Option<(String, WireFingerprint)>,
    pending: Option<RewriteReason>,
    rewrite_streak: u32,
}

impl CacheInstrument {
    /// Fingerprint the request about to go to `model`. Called per dispatch attempt, so after a
    /// failover only the request the answering model received is kept.
    pub(crate) fn stage(&mut self, model: &str, sent: &[Message], tools: &[ToolSpec]) {
        self.staged = (!forge_provider::is_cli_bridge(model))
            .then(|| (model.to_string(), fingerprint(sent, tools)));
    }

    pub(crate) fn note_floor_move(&mut self, movement: FloorMove) {
        let reason = match movement {
            FloorMove::Advance => RewriteReason::FloorAdvance,
            FloorMove::AgeRounds => RewriteReason::AgeRounds,
        };
        if !matches!(
            self.pending,
            Some(RewriteReason::Compaction | RewriteReason::HistoryRewrite)
        ) {
            self.pending = Some(reason);
        }
    }

    pub(crate) fn note_compaction(&mut self) {
        if self.pending != Some(RewriteReason::HistoryRewrite) {
            self.pending = Some(RewriteReason::Compaction);
        }
    }

    /// A rewind or a full-history reload replaced the transcript from outside the loop. Without
    /// this the next request read as an unexplained `harness_rewrite`, and two rewinds in a row
    /// raised the regression warning for something the user did on purpose.
    pub(crate) fn note_history_rewrite(&mut self) {
        self.pending = Some(RewriteReason::HistoryRewrite);
    }

    /// Label the call `model` just answered. Returns the row's attribution and, on the second
    /// unexplained rewrite in a row, a warning to show.
    pub(crate) fn finish(
        &mut self,
        model: &str,
        usage: &Usage,
    ) -> (Option<UsagePrefix>, Option<String>) {
        if forge_provider::is_cli_bridge(model) {
            return (
                Some(UsagePrefix {
                    reused_msgs: None,
                    total_msgs: None,
                    tools_changed: None,
                    label: "n/a".to_string(),
                }),
                None,
            );
        }
        let Some((staged_model, current)) = self.staged.take().filter(|(m, _)| m == model) else {
            return (None, None);
        };
        let reuse = self
            .previous
            .get(&staged_model)
            .map(|previous| current.reuse_against(previous));
        let switched = self.last_model.as_deref().is_some_and(|last| last != model);
        let label = classify(reuse, switched, self.pending.take(), &current, usage);
        let prefix = UsagePrefix {
            reused_msgs: Some(reuse.map_or(0, |r| r.reused) as u32),
            total_msgs: Some(current.len() as u32),
            tools_changed: reuse.map(|r| r.tools_changed),
            label,
        };
        let warning = if prefix.label.starts_with("harness_rewrite") {
            self.rewrite_streak += 1;
            (self.rewrite_streak == REWRITE_STREAK_WARNING).then(|| {
                format!(
                    "prompt cache: the last {REWRITE_STREAK_WARNING} requests rewrote context \
                     already sent to {model} ({}) — each one re-bills the prompt from that point; \
                     see `forge stats cache --session`",
                    prefix.label
                )
            })
        } else {
            self.rewrite_streak = 0;
            None
        };
        self.previous.insert(staged_model.clone(), current);
        self.last_model = Some(staged_model);
        (Some(prefix), warning)
    }
}

/// The label for one request. Pure, so every branch is testable without a session.
fn classify(
    reuse: Option<PrefixReuse>,
    switched: bool,
    pending: Option<RewriteReason>,
    current: &WireFingerprint,
    usage: &Usage,
) -> String {
    if switched {
        return "model_switch".to_string();
    }
    let Some(reuse) = reuse else {
        return "cold".to_string();
    };
    if reuse.reused < reuse.previous_total {
        return match pending {
            Some(reason) => reason.label().to_string(),
            None => format!(
                "harness_rewrite@{}({})",
                reuse.reused,
                current.kind_at(reuse.reused)
            ),
        };
    }
    if reuse.tools_changed {
        return "tools_changed".to_string();
    }
    let evicted = usage.input_tokens >= MIN_CACHEABLE_INPUT
        && usage
            .cached_input_tokens
            .is_some_and(|cached| cached * 2 < usage.input_tokens);
    if evicted {
        "provider_eviction"
    } else {
        "append"
    }
    .to_string()
}

impl Session {
    /// Record a main-loop call's usage with its cache attribution, warning on a rewrite streak.
    pub(crate) fn record_main_usage(
        &mut self,
        message_id: &str,
        usage: &Usage,
        model: &str,
    ) -> Result<(), CoreError> {
        let (prefix, warning) = self.cache.finish(model, usage);
        self.store.record_usage_with_prefix(
            &self.id,
            message_id,
            usage,
            Some(model),
            prefix.as_ref(),
        )?;
        if let Some(warning) = warning {
            self.presenter.emit(PresenterEvent::Warning(warning));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, cached: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: 1,
            cached_input_tokens: Some(cached),
            cost_usd: 0.0,
            ..Default::default()
        }
    }

    fn request(instrument: &mut CacheInstrument, messages: &[Message], cached: u64) -> String {
        instrument.stage("bedrock::kimi", messages, &[]);
        let (prefix, _) = instrument.finish("bedrock::kimi", &usage(10_000, cached));
        prefix.unwrap().label
    }

    #[test]
    fn an_append_only_transcript_is_labelled_append() {
        let mut instrument = CacheInstrument::default();
        let mut messages = vec![Message::system("sys"), Message::user("task")];
        assert_eq!(request(&mut instrument, &messages, 0), "cold");
        for step in 0..5 {
            messages.push(Message::assistant(format!("step {step}")));
            messages.push(Message::user(format!("more {step}")));
            assert_eq!(request(&mut instrument, &messages, 9_000), "append");
        }
    }

    #[test]
    fn mutating_an_earlier_message_is_a_harness_rewrite_and_two_in_a_row_warn() {
        let mut instrument = CacheInstrument::default();
        let mut messages = vec![
            Message::system("sys"),
            Message::user("task"),
            Message::tool_result("c1", "x".repeat(5_000)),
            Message::user("next"),
        ];
        request(&mut instrument, &messages, 0);
        messages[2].content = "pruned".to_string();
        instrument.stage("bedrock::kimi", &messages, &[]);
        let (prefix, warning) = instrument.finish("bedrock::kimi", &usage(10_000, 1_000));
        assert_eq!(prefix.unwrap().label, "harness_rewrite@2(tool)");
        assert!(warning.is_none(), "one rewrite is not yet a streak");

        messages[3].content = "edited".to_string();
        instrument.stage("bedrock::kimi", &messages, &[]);
        let (_, warning) = instrument.finish("bedrock::kimi", &usage(10_000, 1_000));
        assert!(warning.unwrap().contains("harness_rewrite@3"));
    }

    #[test]
    fn a_recorded_floor_move_explains_the_rewrite_it_causes() {
        let mut instrument = CacheInstrument::default();
        let mut messages = vec![Message::user("task"), Message::tool_result("c1", "long")];
        request(&mut instrument, &messages, 0);
        messages[1].content = "short".to_string();
        instrument.note_floor_move(FloorMove::AgeRounds);
        assert_eq!(request(&mut instrument, &messages, 0), "age_rounds");
        messages.push(Message::user("again"));
        assert_eq!(request(&mut instrument, &messages, 9_000), "append");
    }

    #[test]
    fn a_rewind_is_labelled_and_never_counts_toward_the_rewrite_warning() {
        let mut instrument = CacheInstrument::default();
        let mut messages = vec![
            Message::user("one"),
            Message::assistant("a"),
            Message::user("two"),
            Message::assistant("b"),
        ];
        request(&mut instrument, &messages, 0);
        for _ in 0..2 {
            messages.truncate(messages.len() - 2);
            messages.push(Message::user("again"));
            instrument.note_history_rewrite();
            instrument.note_floor_move(FloorMove::Advance);
            instrument.stage("bedrock::kimi", &messages, &[]);
            let (prefix, warning) = instrument.finish("bedrock::kimi", &usage(10_000, 0));
            assert_eq!(prefix.unwrap().label, "history_rewrite");
            assert!(warning.is_none());
            messages.push(Message::assistant("c"));
            assert_eq!(request(&mut instrument, &messages, 9_000), "append");
        }
    }

    #[test]
    fn an_intact_prefix_that_missed_the_cache_is_the_providers_doing() {
        let mut instrument = CacheInstrument::default();
        let mut messages = vec![Message::user("task")];
        request(&mut instrument, &messages, 0);
        messages.push(Message::assistant("answer"));
        assert_eq!(
            request(&mut instrument, &messages, 100),
            "provider_eviction"
        );
    }

    #[test]
    fn switching_models_and_bridges_get_their_own_labels() {
        let mut instrument = CacheInstrument::default();
        let messages = vec![Message::user("task")];
        request(&mut instrument, &messages, 0);
        instrument.stage("deepseek::chat", &messages, &[]);
        let (prefix, _) = instrument.finish("deepseek::chat", &usage(10_000, 0));
        assert_eq!(prefix.unwrap().label, "model_switch");

        instrument.stage("claude-cli::sonnet", &messages, &[]);
        let (prefix, _) = instrument.finish("claude-cli::sonnet", &usage(10_000, 0));
        let prefix = prefix.unwrap();
        assert_eq!(prefix.label, "n/a");
        assert_eq!(prefix.reused_msgs, None);
    }
}
