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
    /// Every `/goal` first drafts a validation contract and asks for approval, as if it were
    /// started with `--contract` (docs/features/goal-contract.md).
    pub contract: bool,
    /// Seconds one contract check may run before the validator kills it and fails the assertion.
    pub contract_check_timeout_secs: u64,
    /// A goal that wedges, or whose session has been compacted this many times, is handed to a
    /// fresh worker session on the same worktree (docs/features/goal-contract.md, "Fresh-worker
    /// relay"). `0` turns the wear trigger off; a wedge still relays.
    pub relay_after_compactions: u32,
    /// Most fresh-worker hand-offs one goal may make before it stops for good. `0` turns the
    /// relay off.
    pub max_relays: u32,
    /// Most milestones a `/goal --mission` may have (2 to 12); the drafter is told this and a
    /// longer draft is cut to it (docs/features/goal-contract.md, "Missions (milestones)").
    pub mission_max_milestones: usize,
}

impl Default for GoalConfig {
    fn default() -> Self {
        Self {
            evaluator: true,
            evaluator_model: None,
            contract: false,
            contract_check_timeout_secs: 60,
            relay_after_compactions: 4,
            max_relays: 3,
            mission_max_milestones: 6,
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

    #[test]
    fn contract_is_off_by_default_and_parses() {
        let parsed: GoalConfig = toml::from_str("").unwrap();
        assert!(!parsed.contract);
        assert_eq!(parsed.contract_check_timeout_secs, 60);
        let parsed: GoalConfig =
            toml::from_str("contract = true\ncontract_check_timeout_secs = 5\n").unwrap();
        assert!(parsed.contract);
        assert_eq!(parsed.contract_check_timeout_secs, 5);
    }

    #[test]
    fn mission_cap_defaults_to_six_and_parses() {
        let parsed: GoalConfig = toml::from_str("").unwrap();
        assert_eq!(parsed.mission_max_milestones, 6);
        let parsed: GoalConfig = toml::from_str("mission_max_milestones = 3\n").unwrap();
        assert_eq!(parsed.mission_max_milestones, 3);
    }

    #[test]
    fn relay_defaults_and_keys_parse() {
        let parsed: GoalConfig = toml::from_str("").unwrap();
        assert_eq!(parsed.relay_after_compactions, 4);
        assert_eq!(parsed.max_relays, 3);
        let parsed: GoalConfig =
            toml::from_str("relay_after_compactions = 2\nmax_relays = 0\n").unwrap();
        assert_eq!(parsed.relay_after_compactions, 2);
        assert_eq!(parsed.max_relays, 0);
    }
}
