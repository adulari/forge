//! Copy for the key-based model providers: names, which have a free tier, and where to create a
//! key. The model step leads with the featured ones; everything else Forge knows stays reachable.

/// A provider the model step leads with.
#[derive(Debug, Clone, Copy)]
pub struct Featured {
    pub id: &'static str,
    pub name: &'static str,
    pub free_tier: bool,
    pub key_url: &'static str,
}

/// Free tiers first: they are what a person with no subscription should see first.
pub const FEATURED: &[Featured] = &[
    Featured {
        id: "groq",
        name: "Groq",
        free_tier: true,
        key_url: "https://console.groq.com/keys",
    },
    Featured {
        id: "cerebras",
        name: "Cerebras",
        free_tier: true,
        key_url: "https://cloud.cerebras.ai",
    },
    Featured {
        id: "gemini",
        name: "Google Gemini",
        free_tier: true,
        key_url: "https://aistudio.google.com/apikey",
    },
    Featured {
        id: "openrouter",
        name: "OpenRouter",
        free_tier: true,
        key_url: "https://openrouter.ai/keys",
    },
    Featured {
        id: "opencode",
        name: "OpenCode Zen",
        free_tier: true,
        key_url: "https://opencode.ai/zen",
    },
    Featured {
        id: "anthropic",
        name: "Anthropic",
        free_tier: false,
        key_url: "https://console.anthropic.com/settings/keys",
    },
    Featured {
        id: "openai",
        name: "OpenAI",
        free_tier: false,
        key_url: "https://platform.openai.com/api-keys",
    },
    Featured {
        id: "xai",
        name: "xAI",
        free_tier: false,
        key_url: "https://console.x.ai",
    },
    Featured {
        id: "deepseek",
        name: "DeepSeek",
        free_tier: false,
        key_url: "https://platform.deepseek.com/api_keys",
    },
];

pub fn featured(id: &str) -> Option<&'static Featured> {
    FEATURED.iter().find(|f| f.id == id)
}

/// A human label plus a free/paid hint for a key-based provider (`forge auth`, `/config`, the
/// provider list).
pub fn provider_label(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "Anthropic (Claude API) — paid",
        "openai" => "OpenAI (GPT API) — paid",
        "gemini" => "Google Gemini — free tier + paid",
        "xai" => "xAI (Grok) — paid",
        "xai-oauth" => "xAI (Grok) — SuperGrok/X Premium subscription (OAuth, no API key)",
        "codex-oauth" => "OpenAI ChatGPT — Plus/Pro subscription (OAuth, no API key)",
        "deepseek" => "DeepSeek — paid",
        "openrouter" => "OpenRouter (gateway, many models) — paid + some :free",
        "groq" => "Groq — free tier (fast)",
        "opencode_go" => "OpenCode Go — subscription (curated coding models)",
        "kimi" => "Kimi Code — subscription (K3, Kimi for Coding)",
        "opencode" => "OpenCode Zen — API credits + free tier",
        "github_copilot" => "GitHub Models — free inference",
        "mimo" => "Xiaomi MiMo — free",
        "minimax" => "MiniMax — free tier",
        "cohere" => "Cohere — Command A (218B), free trial tier",
        "bedrock" => "AWS Bedrock — enterprise (Bedrock API key; pin model ids)",
        "vertex" => "Google Vertex AI — enterprise (needs VERTEX_PROJECT_ID; pin model ids)",
        "together" => "Together AI — gateway (OSS frontier, metered)",
        "fireworks" => "Fireworks AI — gateway (fast OSS, metered)",
        "perplexity" => "Perplexity — Sonar (online + reasoning, metered)",
        other => forge_config::custom_provider(other)
            .map(|p| p.label)
            .unwrap_or("provider"),
    }
}

/// The short display name of a provider: the featured name, else the id.
pub fn display_name(provider: &str) -> String {
    featured(provider)
        .map(|f| f.name.to_string())
        .unwrap_or_else(|| provider.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_tiers_come_before_paid_providers() {
        let first_paid = FEATURED.iter().position(|f| !f.free_tier).unwrap();
        assert!(FEATURED[..first_paid].iter().all(|f| f.free_tier));
        assert!(FEATURED[first_paid..].iter().all(|f| !f.free_tier));
    }

    #[test]
    fn every_featured_provider_is_one_forge_can_authenticate() {
        let known: Vec<&str> = forge_config::known_key_providers().collect();
        for f in FEATURED {
            assert!(
                known.contains(&f.id),
                "{} is not a known key provider",
                f.id
            );
            assert!(f.key_url.starts_with("https://"));
        }
    }

    #[test]
    fn labels_cover_the_subscription_and_gateway_providers() {
        assert!(provider_label("groq").contains("free tier"));
        assert!(provider_label("codex-oauth").contains("OAuth"));
        assert_eq!(provider_label("not-a-provider"), "provider");
        assert_eq!(display_name("groq"), "Groq");
        assert_eq!(display_name("cohere"), "cohere");
    }
}
