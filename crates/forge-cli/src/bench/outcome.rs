//! Classifying how one benchmark attempt ended, beyond "patched or not".
//!
//! Provider stop reasons do not reach `ModelResponse`, so a refusal cannot be read off the wire.
//! Two signals are used instead, and both are recorded as such in `outcome_detail`:
//! - a turn that failed with the provider's content-filter error (the Responses API surfaces
//!   `incomplete: content_filter` as an error), and
//! - an empty patch whose final answer contains a refusal phrase (a heuristic).
//!
//! Refusals always score 0 and are never excluded from pass@1, unlike `tools_unavailable`.

use serde::{Deserialize, Serialize};

/// How an attempt ended. `ok` covers every attempt the evaluator should simply score.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    #[default]
    Ok,
    Refused,
    /// Failed a post-hoc integrity check (test tampering, history leak). Scored 0.
    IntegrityFail,
    ToolsUnavailable,
    TimedOut,
}

pub(super) struct AttemptSignals<'a> {
    pub tools_unavailable: bool,
    pub timed_out: bool,
    pub patched: bool,
    pub final_text: Option<&'a str>,
    pub error: Option<&'a str>,
}

/// Lowercase phrases that, in an attempt that changed nothing, mean the model declined the task.
/// Kept to first-person declines so an answer that merely discusses an error is not caught.
const REFUSAL_PHRASES: &[&str] = &[
    "i can't help with",
    "i cannot help with",
    "i can't assist with",
    "i cannot assist with",
    "i'm unable to help",
    "i am unable to help",
    "i'm unable to assist",
    "i am unable to assist",
    "i won't be able to help",
    "i must decline",
    "i can't comply",
    "i cannot comply",
    "against my usage policies",
];

fn refusal_phrase(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase().replace('\u{2019}', "'");
    REFUSAL_PHRASES
        .iter()
        .copied()
        .find(|phrase| lower.contains(phrase))
}

pub(super) fn classify(signals: &AttemptSignals<'_>) -> (AttemptOutcome, Option<String>) {
    if signals.tools_unavailable {
        return (
            AttemptOutcome::ToolsUnavailable,
            Some("mcp-serve failed to start; excluded from pass@1".into()),
        );
    }
    if !signals.patched {
        if signals
            .error
            .is_some_and(|e| e.to_lowercase().contains("content filter"))
        {
            return (
                AttemptOutcome::Refused,
                Some("provider content filter blocked the response".into()),
            );
        }
        if let Some(phrase) = signals.final_text.and_then(refusal_phrase) {
            return (
                AttemptOutcome::Refused,
                Some(format!(
                    "heuristic: empty patch and final answer contains \"{phrase}\""
                )),
            );
        }
    }
    if signals.timed_out {
        return (AttemptOutcome::TimedOut, None);
    }
    (AttemptOutcome::Ok, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals<'a>(patched: bool, final_text: Option<&'a str>) -> AttemptSignals<'a> {
        AttemptSignals {
            tools_unavailable: false,
            timed_out: false,
            patched,
            final_text,
            error: None,
        }
    }

    #[test]
    fn an_empty_patch_with_a_refusal_phrase_is_a_heuristic_refusal() {
        let (outcome, detail) = classify(&signals(
            false,
            Some("Sorry, but I can\u{2019}t help with modifying this repository."),
        ));
        assert_eq!(outcome, AttemptOutcome::Refused);
        assert!(detail.unwrap().starts_with("heuristic:"));
    }

    #[test]
    fn a_refusal_phrase_next_to_a_patch_is_not_a_refusal() {
        let (outcome, _) = classify(&signals(true, Some("I cannot help with the docs part.")));
        assert_eq!(outcome, AttemptOutcome::Ok);
    }

    #[test]
    fn a_plain_miss_stays_ok_and_a_timeout_is_recorded() {
        assert_eq!(
            classify(&signals(false, Some("The fix is in parser.py."))).0,
            AttemptOutcome::Ok
        );
        let timed = AttemptSignals {
            timed_out: true,
            ..signals(true, None)
        };
        assert_eq!(classify(&timed).0, AttemptOutcome::TimedOut);
    }

    #[test]
    fn content_filter_errors_and_toolless_runs_are_classified_first() {
        let filtered = AttemptSignals {
            error: Some(
                "running the agent turn: response blocked by the provider's content filter",
            ),
            ..signals(false, None)
        };
        assert_eq!(classify(&filtered).0, AttemptOutcome::Refused);
        let toolless = AttemptSignals {
            tools_unavailable: true,
            ..signals(false, Some("I cannot help with that"))
        };
        assert_eq!(classify(&toolless).0, AttemptOutcome::ToolsUnavailable);
    }

    #[test]
    fn outcome_serializes_snake_case_and_defaults_to_ok() {
        assert_eq!(
            serde_json::to_string(&AttemptOutcome::ToolsUnavailable).unwrap(),
            "\"tools_unavailable\""
        );
        assert_eq!(AttemptOutcome::default(), AttemptOutcome::Ok);
    }
}
