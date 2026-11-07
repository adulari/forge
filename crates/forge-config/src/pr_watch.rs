//! `[pr_watch]`: settings for the daemon's PR Watch, today the independent review gate that
//! sits in front of every merge (docs/features/pr-review-gate.md).

use serde::{Deserialize, Serialize};

fn default_review() -> String {
    "auto".to_string()
}

fn default_review_severity() -> String {
    "high".to_string()
}

fn default_review_max_findings() -> u32 {
    5
}

fn default_review_on_error() -> String {
    "block".to_string()
}

fn default_review_timeout_secs() -> u64 {
    600
}

fn default_review_max_rounds() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrWatchConfig {
    /// `auto` reviews every PR PR Watch is about to merge; `off` merges as before.
    #[serde(default = "default_review")]
    pub review: String,
    /// The reviewer. Empty resolves like the expert: `[harness.expert]` when set, otherwise the
    /// mesh's own picks. A subscription model needs `review_budget`.
    #[serde(default)]
    pub review_model: String,
    /// A slice of a subscription window (`codex:5%w`) or a dollar cap (`$1`), as for the expert.
    #[serde(default)]
    pub review_budget: String,
    /// Confirmed findings kept per review.
    #[serde(default = "default_review_max_findings")]
    pub review_max_findings: u32,
    /// Lowest severity that blocks the merge: `low` | `medium` | `high` | `critical`.
    #[serde(default = "default_review_severity")]
    pub review_severity: String,
    /// What a reviewer failure does: `block` (do not merge, retry) or `merge`.
    #[serde(default = "default_review_on_error")]
    pub review_on_error: String,
    #[serde(default = "default_review_timeout_secs")]
    pub review_timeout_secs: u64,
    /// Fix rounds sent back to the session before the PR is left blocked for the owner.
    #[serde(default = "default_review_max_rounds")]
    pub review_max_rounds: u32,
}

impl Default for PrWatchConfig {
    fn default() -> Self {
        Self {
            review: default_review(),
            review_model: String::new(),
            review_budget: String::new(),
            review_max_findings: default_review_max_findings(),
            review_severity: default_review_severity(),
            review_on_error: default_review_on_error(),
            review_timeout_secs: default_review_timeout_secs(),
            review_max_rounds: default_review_max_rounds(),
        }
    }
}

impl PrWatchConfig {
    pub fn review_enabled(&self) -> bool {
        !self.review.trim().eq_ignore_ascii_case("off")
    }

    pub fn merge_on_review_error(&self) -> bool {
        self.review_on_error.trim().eq_ignore_ascii_case("merge")
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
    fn review_is_on_by_default_and_blocks_on_error() {
        let c = PrWatchConfig::default();
        assert!(c.review_enabled() && !c.merge_on_review_error());
        assert_eq!(
            (c.review_max_findings, c.review_severity.as_str()),
            (5, "high")
        );
    }

    #[test]
    fn the_table_parses() {
        let cfg = parse(
            "[pr_watch]\nreview = \"off\"\nreview_model = \"bedrock::sonnet\"\nreview_on_error = \"merge\"\nreview_max_findings = 3\n",
        );
        assert!(!cfg.pr_watch.review_enabled());
        assert!(cfg.pr_watch.merge_on_review_error());
        assert_eq!(cfg.pr_watch.review_max_findings, 3);
        assert_eq!(cfg.pr_watch.review_model, "bedrock::sonnet");
    }
}
