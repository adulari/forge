//! Usage accounting for a finished stream: genai usage normalisation and phantom-truncation
//! detection.

use super::*;

/// Normalize genai's provider-independent usage into Forge accounting. OpenAI-compatible APIs
/// report prompt-cache hits under `prompt_tokens_details.cached_tokens`; preserving that subset
/// lets every API-key provider receive the correct discounted cache-read pricing when known.
///
/// Providers that omit `prompt_tokens_details` entirely (groq among them) yield `None` rather than
/// 0: they never told us how much was cached, which is not a claim that nothing was.
pub(super) fn normalize_usage(usage: &genai::chat::Usage) -> Usage {
    let count = |n: Option<i32>| n.map(|n| n.max(0) as u64);
    let prompt = usage.prompt_tokens_details.as_ref();
    let completion = usage.completion_tokens_details.as_ref();
    Usage {
        input_tokens: count(usage.prompt_tokens).unwrap_or(0),
        output_tokens: count(usage.completion_tokens).unwrap_or(0),
        cached_input_tokens: count(prompt.and_then(|d| d.cached_tokens)),
        cost_usd: 0.0, // priced by the mesh from token counts (FR-5)
        // Anthropic/Bedrock cache creation and reasoning counts; genai turns a zero into None.
        cache_write_tokens: count(prompt.and_then(|d| d.cache_creation_tokens)),
        reasoning_tokens: count(completion.and_then(|d| d.reasoning_tokens)),
        // The provider's own charge (`usage.cost`), when it sends one; the mesh prefers it to its
        // rate-table estimate for metered models.
        reported_cost_usd: usage.cost,
    }
}

/// Decide whether a finished stream is a silently-truncated phantom rather than a real completion.
/// True (→ reject as retryable) only when NONE of the completion signals are present: no finish/
/// `End` event, no tool calls, and zero usage. Any one of those means the provider actually
/// finished (even an empty answer), so the response is kept.
pub(super) fn is_phantom_truncation(saw_end: bool, has_tool_calls: bool, usage: &Usage) -> bool {
    !saw_end && !has_tool_calls && usage.input_tokens == 0 && usage.output_tokens == 0
}
