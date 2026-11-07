//! Per-model and per-endpoint policy: embedding failure finality, Qwen thinking budgets and
//! OpenCode Go wrong-wire-format detection.

use super::*;

/// Whether a failed embedding call fails the same way on every retry in this process: a rejection
/// of the request itself (a retired model, a missing credential, a malformed call), not a rate
/// limit or an outage.
pub(crate) fn embed_failure_is_final(err: &genai::Error) -> bool {
    let classified = classify_genai_error(err);
    classified.is_permanent() || matches!(classified, ProviderError::Request(_))
}

/// Qwen 3.8 Max Preview is thinking-only on the Token Plan endpoint (the API rejects
/// `enable_thinking=false`). Bound its private reasoning phase independently from the answer/tool
/// payload so a difficult coding turn cannot spend many minutes re-planning before its first tool
/// call. This is deliberately *not* `max_tokens`: the final response remains at the model's native
/// output limit.
pub(super) fn qwencloud_thinking_budget(effort: Option<EffortLevel>) -> u32 {
    match effort.unwrap_or(EffortLevel::Medium) {
        EffortLevel::Low => 1_024,
        EffortLevel::Medium => 4_096,
        EffortLevel::High => 8_192,
        EffortLevel::XHigh | EffortLevel::WhiteHot => 16_384,
    }
}

/// The failure shape OpenCode Go returns when a model is sent to the wrong wire format: an
/// immediate 5xx (`Unavailable`) or 401 (`Auth`) with no useful body. Rate limits, capability
/// and access failures are real answers about the model and must not trigger endpoint learning.
pub(super) fn opencode_go_wrong_endpoint(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Unavailable(_) | ProviderError::Auth(_)
    )
}
