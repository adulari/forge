//! `[privacy]`: zero-data-retention routing. See docs/features/zero-retention.md.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Route only to providers known not to retain prompts. Off by default. When on, a provider
    /// Forge has no zero-retention evidence for is treated as retaining and never receives a
    /// request (main turns, side calls and subagents alike).
    pub zero_retention: bool,
    /// Per-provider override, keyed by namespace (`openrouter = false`). `false` vouches that the
    /// provider does not retain prompts under your account terms; `true` marks it as retaining
    /// even where Forge's built-in table says otherwise.
    pub retains: BTreeMap<String, bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_and_overrides_parse_from_toml() {
        assert!(!PrivacyConfig::default().zero_retention);
        let parsed: PrivacyConfig =
            toml::from_str("zero_retention = true\n[retains]\nopenrouter = false\nollama = true\n")
                .unwrap();
        assert!(parsed.zero_retention);
        assert_eq!(parsed.retains.get("openrouter"), Some(&false));
        assert_eq!(parsed.retains.get("ollama"), Some(&true));
    }
}
