//! `[preview]`: public preview URLs for a dev server the agent started (`shell_job` action
//! `expose`). See docs/features/shell-tool.md.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PreviewConfig {
    /// Kill switch. On by default because every expose already needs an explicit approval and is
    /// denied in read-only/plan mode; `false` refuses the action outright.
    pub enabled: bool,
    /// Reuse `[remote] tunnel_name` / `tunnel_hostname` for previews. Off by default so app
    /// traffic never lands on the Forge Anywhere / `/remote` hostname by accident.
    pub use_fixed_hostname: bool,
    /// Minutes a preview tunnel may live before the reaper kills it, even if its job is still up.
    pub ttl_minutes: u64,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            use_fixed_hostname: false,
            ttl_minutes: 120,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_on_quick_tunnel_and_a_two_hour_ttl() {
        let parsed: PreviewConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, PreviewConfig::default());
        assert!(parsed.enabled);
        assert!(!parsed.use_fixed_hostname);
        assert_eq!(parsed.ttl_minutes, 120);
        let off: PreviewConfig = toml::from_str("enabled = false").unwrap();
        assert!(!off.enabled);
        assert_eq!(off.ttl_minutes, 120);
    }
}
