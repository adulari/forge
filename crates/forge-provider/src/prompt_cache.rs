//! When and how Forge sends OpenAI's optional `prompt_cache_key` request field.

use crate::ProviderError;

/// Whether to attach OpenAI's optional cache-sharding hint to this request.
///
/// NVIDIA NIM enables prefix caching at the deployment/server layer and rejects
/// `prompt_cache_key` as an unknown body field. Sending it there adds a guaranteed failed request
/// before every useful completion (the hosted endpoint may take close to a minute to return that
/// validation response). Other OpenAI-compatible endpoints remain optimistic and retain the
/// narrow validation fallback below, which is required for custom endpoints whose capabilities
/// cannot be known ahead of time.
pub fn should_send_prompt_cache_key(model: &str) -> bool {
    !matches!(model.split_once("::"), Some(("nvidia", _)))
}

/// Whether a key shared by every session with the same fixed prefix is safe and useful here.
///
/// It must be a provider that receives the key at all ([`should_send_prompt_cache_key`]) and
/// treats it purely as a cache-routing hint. Gemini is excluded because Forge derives its explicit
/// `cachedContents` resource from the key, the CLI bridges because they never put it on the wire.
pub fn shares_prompt_cache_key_across_sessions(model: &str) -> bool {
    should_send_prompt_cache_key(model)
        && !crate::is_cli_bridge(model)
        && !matches!(model.split_once("::"), Some(("gemini", _)))
}

/// True only when a provider rejected the optional OpenAI `prompt_cache_key` request field.
///
/// OpenAI-compatible gateways vary: some implement the field, some silently ignore it, and strict
/// ones return a validation error. Callers use this narrow classifier to retry once without the
/// hint, retaining the provider's automatic prefix cache (when it has one) without turning a cache
/// optimization into a failed completion. It deliberately requires the field name plus a clear
/// validation signal so unrelated request failures are never retried under a different payload.
pub(crate) fn prompt_cache_key_unsupported(error: &ProviderError) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("prompt_cache_key")
        && [
            "unsupported",
            "not supported",
            "unknown",
            "unrecognized",
            "not permitted",
            "not allowed",
            "extra inputs",
            "extra_forbidden",
            "invalid parameter",
            "unexpected",
        ]
        .iter()
        .any(|marker| text.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_session_keys_only_go_to_pure_cache_routing_hints() {
        assert!(shares_prompt_cache_key_across_sessions("openai::gpt-5.4"));
        assert!(shares_prompt_cache_key_across_sessions(
            "codex-oauth::gpt-5.6"
        ));
        assert!(shares_prompt_cache_key_across_sessions("openrouter::a/b"));
        assert!(!shares_prompt_cache_key_across_sessions(
            "nvidia::z-ai/glm-5.2"
        ));
        assert!(!shares_prompt_cache_key_across_sessions(
            "gemini::gemini-2.5-flash"
        ));
        assert!(!shares_prompt_cache_key_across_sessions("claude-cli::opus"));
        assert!(!shares_prompt_cache_key_across_sessions(
            "codex-cli::gpt-5.5"
        ));
    }
}
