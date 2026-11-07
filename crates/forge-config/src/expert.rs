//! `[harness.expert]`: a stronger model consulted at the moments that decide a long run.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_max_consults() -> u32 {
    2
}

fn default_expert_timeout_secs() -> u64 {
    120
}

/// Which moments may consult the expert. All on once a model is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertTriggers {
    /// Before the progress supervisor nudges or stops a turn that is going in circles.
    #[serde(default = "default_true")]
    pub stall: bool,
    /// At DONE: the advisor review, sent to the expert model.
    #[serde(default = "default_true")]
    pub done: bool,
    /// The model may call `consult_expert` itself.
    #[serde(default = "default_true")]
    pub tool: bool,
}

impl Default for ExpertTriggers {
    fn default() -> Self {
        Self {
            stall: true,
            done: true,
            tool: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertConfig {
    /// The expert. Empty means no expert (the DONE review keeps using `[harness.advisor]`).
    /// A `codex-oauth::*` subscription model is allowed here and only here, and only with a
    /// `budget`. A CLI bridge (`claude-cli::*`, `codex-cli::*`) is refused: it runs as an agent.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub triggers: ExpertTriggers,
    /// Consults one turn may spend across all triggers.
    #[serde(default = "default_max_consults")]
    pub max_consults_per_turn: u32,
    /// A slice of a subscription window, spelled like a goal envelope: `codex:5%w`,
    /// `claude:10%5h`, or `$2` for a metered API model. Required for a subscription expert.
    #[serde(default)]
    pub budget: String,
    #[serde(default = "default_expert_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for ExpertConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            triggers: ExpertTriggers::default(),
            max_consults_per_turn: default_max_consults(),
            budget: String::new(),
            timeout_secs: default_expert_timeout_secs(),
        }
    }
}

impl ExpertConfig {
    pub fn is_set(&self) -> bool {
        !self.model.trim().is_empty() && self.max_consults_per_turn > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> crate::Config {
        use figment::providers::{Format, Serialized, Toml};
        figment::Figment::from(Serialized::defaults(crate::Config::default()))
            .merge(Toml::string(toml))
            .extract()
            .unwrap()
    }

    #[test]
    fn off_by_default_with_every_trigger_ready() {
        let e = ExpertConfig::default();
        assert!(!e.is_set());
        assert!(e.triggers.stall && e.triggers.done && e.triggers.tool);
        assert_eq!(e.max_consults_per_turn, 2);
    }

    #[test]
    fn the_table_parses_with_a_nested_triggers_table() {
        let cfg = parse(
            "[harness.expert]\nmodel = \"codex-oauth::gpt-5.6-sol\"\nmax_consults_per_turn = 3\nbudget = \"codex:5%w\"\n\n[harness.expert.triggers]\ntool = false\n",
        );
        let e = &cfg.harness.expert;
        assert!(e.is_set());
        assert_eq!(e.model, "codex-oauth::gpt-5.6-sol");
        assert_eq!(e.budget, "codex:5%w");
        assert_eq!(e.max_consults_per_turn, 3);
        assert!(e.triggers.stall && e.triggers.done && !e.triggers.tool);
    }

    #[test]
    fn the_advisor_table_still_parses_beside_it() {
        let cfg = parse("[harness.advisor]\nenabled = true\nmodel = \"bedrock::sonnet\"\n");
        assert!(cfg.harness.advisor.enabled);
        assert!(!cfg.harness.expert.is_set());
    }
}
