//! `[mesh] adaptive_effort`: run the step after a successful read-only tool round at low reasoning
//! effort (docs/features/adaptive-effort.md).
//!
//! The switch is per provider because changing the effort between two requests only saves tokens
//! when it leaves the provider's prompt cache intact. Where it would not, the cheaper step costs a
//! full cache rewrite and the change loses money.

/// Providers where the effort is a request-level field outside the cached prompt, checked against
/// the adapter that builds each request:
///
/// - `openai`: genai's OpenAI adapter sends a top-level `reasoning_effort`; OpenAI caches the
///   prompt (messages, tools, images, schema) by exact prefix.
/// - `codex-oauth`: the Responses body carries `reasoning.effort` beside `input`
///   (codex_oauth.rs), same OpenAI cache. On the WebSocket models a changed effort makes that one
///   request a full resend instead of a `previous_response_id` delta (codex_websocket.rs
///   `request_properties_match`); the replayed encrypted reasoning and `prompt_cache_key` keep it
///   on the cached prefix.
/// - `gemini`: genai puts the level in `generationConfig.thinkingConfig`; Gemini caches
///   `contents`/`systemInstruction`/`tools`, and the explicit cachedContent holds only those.
///
/// Deliberately absent:
/// - `anthropic`, `bedrock`: Anthropic documents that a `thinking` or `effort` change invalidates
///   the messages cache. genai sends `output_config.effort` or `thinking.budget_tokens`; both count.
/// - `openrouter`: forwards `reasoning` to whichever upstream serves the model, Anthropic included.
/// - open-weight hosts (`groq`, `cerebras`, `nvidia`, `together`, `fireworks`, `deepseek`,
///   `ollama`, ...): their chat templates render the reasoning level into the prompt (gpt-oss
///   harmony writes `Reasoning: <level>` into the system message), so a change rewrites the prefix
///   from the first token.
/// - CLI bridges: the effort is a process argument, and the persistent claude bridge would have to
///   restart.
/// - everything else: unverified.
pub const CACHE_SAFE_EFFORT_PROVIDERS: &[&str] = &["openai", "codex-oauth", "gemini"];

pub(crate) fn default_adaptive_effort_providers() -> Vec<String> {
    CACHE_SAFE_EFFORT_PROVIDERS
        .iter()
        .map(|p| p.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn defaults_cover_only_the_cache_safe_providers() {
        let cfg = Config::default();
        assert!(cfg.mesh.adaptive_effort);
        assert_eq!(
            cfg.mesh.adaptive_effort_providers,
            ["openai", "codex-oauth", "gemini"]
        );
        assert!(cfg.mesh.adaptive_effort_for("codex-oauth::gpt-5.6-luna"));
        assert!(!cfg
            .mesh
            .adaptive_effort_for("bedrock::anthropic.claude-opus-5-5"));
        assert!(!cfg.mesh.adaptive_effort_for("groq::openai/gpt-oss-120b"));
    }

    #[test]
    fn the_switch_and_the_list_are_both_honoured() {
        let off: Config = toml::from_str(
            "permission_mode = \"accept-edits\"\n[mesh]\nmodels = {}\nadaptive_effort = false\n",
        )
        .unwrap();
        assert!(!off.mesh.adaptive_effort_for("openai::gpt-5.5"));
        let opted: Config =
            toml::from_str("permission_mode = \"accept-edits\"\n[mesh]\nmodels = {}\nadaptive_effort_providers = [\"xai-oauth\"]\n")
                .unwrap();
        assert!(opted.mesh.adaptive_effort_for("xai-oauth::grok-5"));
        assert!(!opted.mesh.adaptive_effort_for("openai::gpt-5.5"));
        let unset: Config =
            toml::from_str("permission_mode = \"accept-edits\"\n[mesh]\nmodels = {}\n").unwrap();
        assert_eq!(
            unset.mesh.adaptive_effort_providers,
            Config::default().mesh.adaptive_effort_providers
        );
    }
}
