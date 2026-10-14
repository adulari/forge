//! `[goal]`: the `/goal` progress evaluator. See docs/features/goal-evaluator.md.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoalConfig {
    /// After each `/goal` turn, ask a cheap side-call model whether the goal is met and what is
    /// left, and build the next turn's prompt from that verdict. `false` restores the fixed
    /// continue prompt.
    pub evaluator: bool,
    /// Model for the evaluator. Empty uses the side-call model (the trivial tier, or the
    /// session's pin). A subscription model the session is not itself running on, a CLI bridge,
    /// or a model zero-retention routing forbids is ignored in favour of that default.
    pub evaluator_model: Option<String>,
}

impl Default for GoalConfig {
    fn default() -> Self {
        Self {
            evaluator: true,
            evaluator_model: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_by_default_and_both_keys_parse() {
        let parsed: GoalConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, GoalConfig::default());
        assert!(parsed.evaluator);
        let parsed: GoalConfig =
            toml::from_str("evaluator = false\nevaluator_model = \"groq::llama\"\n").unwrap();
        assert!(!parsed.evaluator);
        assert_eq!(parsed.evaluator_model.as_deref(), Some("groq::llama"));
    }
}
