//! Learned output caps for endpoints whose default `max_tokens` is far below the model's limit.

/// The output cap Forge sends once a model has been seen stopping at a small server default.
pub(crate) const RAISED_OUTPUT_TOKENS: u32 = 32_768;

/// A reply that hit the output limit within this many tokens hit a server's DEFAULT cap, not the
/// model's real limit: Refuseless ends every uncapped completion at 1,024 tokens, so with no
/// configured cap each large `write_file`/`append_file` was cut mid-argument and failed. A real
/// native limit (or vLLM's remaining-context default) sits far above this.
const SMALL_DEFAULT_OUTPUT_LIMIT: u64 = 8_192;

/// Models observed stopping at a small server-default output cap while Forge sent none. They get
/// [`RAISED_OUTPUT_TOKENS`] from then on, for the life of the process. Learned only after such a
/// cut, so an endpoint that rejects large `max_tokens` values is never sent one up front.
static RAISED_OUTPUT_CAP: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

pub(crate) fn learned_output_cap(model: &str) -> Option<u32> {
    RAISED_OUTPUT_CAP
        .lock()
        .ok()
        .filter(|set| set.contains(model))
        .map(|_| RAISED_OUTPUT_TOKENS)
}

/// Records a small-default cut and reports whether the cap was raised just now, so the
/// truncation message can tell the model to retry the same call instead of splitting it.
pub(crate) fn learn_small_output_default(
    model: &str,
    sent_cap: Option<u32>,
    output_tokens: u64,
) -> bool {
    if sent_cap.is_some() || output_tokens == 0 || output_tokens > SMALL_DEFAULT_OUTPUT_LIMIT {
        return false;
    }
    let raised = RAISED_OUTPUT_CAP
        .lock()
        .is_ok_and(|mut set| set.insert(model.to_string()));
    if raised {
        tracing::warn!(
            model,
            output_tokens,
            "endpoint stopped at a small default output limit; sending \
             max_tokens={RAISED_OUTPUT_TOKENS} for this model from now on"
        );
    }
    raised
}

/// What a tool call cut off by the output limit tells the model to do next.
pub(crate) fn truncation_reason(cap_raised: bool) -> &'static str {
    if cap_raised {
        "the endpoint's default output limit interrupted this tool call before its JSON arguments were complete; Forge has raised the output limit for this model, so retry the same call unchanged"
    } else {
        "the model's native output limit interrupted this tool call before its JSON arguments were complete; retry with a smaller payload (split a large file into coherent chunks)"
    }
}

/// A tool call's arguments as the loop should see them: a call the output limit cut off becomes
/// an explicit marker (never a salvaged prefix that would run), anything else is repaired.
pub(crate) fn normalize_tool_arguments(
    raw: serde_json::Value,
    output_limit_reached: bool,
    cap_raised: bool,
) -> serde_json::Value {
    if output_limit_reached && raw.is_string() {
        let reason = truncation_reason(cap_raised);
        return serde_json::json!({ crate::TRUNCATED_TOOL_ARGS_KEY: reason });
    }
    crate::repair_malformed_args(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_default_output_cut_raises_the_cap_for_that_model_only() {
        let model = "refuseless-test::small-default";
        assert_eq!(learned_output_cap(model), None);
        assert!(
            !learn_small_output_default(model, Some(1_024), 1_024),
            "a configured cap is the user's choice"
        );
        assert!(
            !learn_small_output_default(model, None, 60_000),
            "a long reply hit a real limit"
        );
        assert!(learn_small_output_default(model, None, 1_024));
        assert!(
            !learn_small_output_default(model, None, 1_024),
            "reported as raised only once"
        );
        assert_eq!(learned_output_cap(model), Some(RAISED_OUTPUT_TOKENS));
        assert_eq!(learned_output_cap("refuseless-test::other"), None);
    }
}
