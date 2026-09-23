//! Explicit prompt-cache breakpoints, and the providers that must not receive them.

use genai::chat::{CacheControl, ChatMessage, ChatRole};

/// Whether a request to `model` may carry explicit cache breakpoints. Everywhere else an
/// unsupported hint is ignored, but Bedrock Converse rejects the whole request ("This model doesn't
/// support the cachePoint field") for any model outside its prompt-caching list — Kimi K3 among
/// them. There the hint is sent only to the families Bedrock caches: Claude and Nova.
fn accepts_cache_breakpoints(model: &str) -> bool {
    match model.split_once("::") {
        Some(("bedrock", id)) => {
            let id = id.to_ascii_lowercase();
            id.contains("anthropic.") || id.contains("amazon.nova")
        }
        _ => true,
    }
}

pub(crate) fn mark_cache_breakpoints(model: &str, msgs: &mut [ChatMessage]) {
    if msgs.is_empty() || !accepts_cache_breakpoints(model) {
        return;
    }
    // Anchor on the END of the leading system run, not just the first system message: Forge emits
    // several stacked system messages (base prompt + env + AGENTS.md + skill guidance), and a
    // breakpoint caches everything UP TO it — so marking the last leading system message caches the
    // whole standing prefix instead of re-billing all but the first every turn. Plus a breakpoint on
    // the final message so the rest of the conversation prefix is cached for the next turn's reuse.
    let last_leading_system = msgs
        .iter()
        .take_while(|m| m.role == ChatRole::System)
        .count()
        .checked_sub(1);
    let last = msgs.len() - 1;
    for idx in [last_leading_system, Some(last)].into_iter().flatten() {
        msgs[idx].options = Some(CacheControl::Ephemeral.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_sends_cache_breakpoints_only_to_the_families_it_caches() {
        // Kimi K3 on Bedrock failed every request: Converse rejects a cachePoint it does not
        // support instead of ignoring it.
        assert!(!accepts_cache_breakpoints(
            "bedrock::global.moonshotai.kimi-k3"
        ));
        assert!(accepts_cache_breakpoints(
            "bedrock::us.anthropic.claude-sonnet-4-5-20250929-v1:0"
        ));
        assert!(accepts_cache_breakpoints("bedrock::amazon.nova-pro-v1:0"));
        assert!(accepts_cache_breakpoints("kimi::k3"));
        assert!(accepts_cache_breakpoints("anthropic::claude-opus-5"));
    }
}
