//! `[harness.advisor]`: a second-model review of the turn's work before it finishes.

use serde::{Deserialize, Serialize};

fn default_advisor_timeout_secs() -> u64 {
    120
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorConfig {
    /// Master switch. Off by default.
    #[serde(default)]
    pub enabled: bool,
    /// Model that reviews. Empty means the model driving the turn. A Claude Sonnet-class model
    /// is the recommended setting: a small same-model reviewer truncated on large transcripts.
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_advisor_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for AdvisorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            timeout_secs: default_advisor_timeout_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_and_inherit_the_turn_model() {
        let a = AdvisorConfig::default();
        assert!(!a.enabled);
        assert_eq!(a.model, "");
        assert_eq!(a.timeout_secs, 120);
    }

    #[test]
    fn the_table_parses_after_providers_and_a_harness_table() {
        let toml = "[harness]\nrepeat_interrupt = true\n\n[[providers.custom]]\nnamespace = \"x\"\nlabel = \"X\"\nbase_url = \"http://localhost\"\nmodels = [\"m\"]\n\n[harness.advisor]\nenabled = true\nmodel = \"bedrock::sonnet\"\ntimeout_secs = 30\n";
        use figment::providers::{Format, Serialized, Toml};
        let cfg: crate::Config =
            figment::Figment::from(Serialized::defaults(crate::Config::default()))
                .merge(Toml::string(toml))
                .extract()
                .unwrap();
        assert!(cfg.harness.advisor.enabled);
        assert_eq!(cfg.harness.advisor.model, "bedrock::sonnet");
        assert_eq!(cfg.harness.advisor.timeout_secs, 30);
        assert!(cfg.harness.repeat_interrupt);
    }
}
