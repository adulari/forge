//! `[pr]`: how the `open_pr` tool publishes a pull request. See docs/features/open-pr.md.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrConfig {
    /// Push the session's screenshots to the repository's orphan `forge-evidence` branch so they
    /// render inline in the PR. Set `false` (per repo in `.forge/config.toml`) to keep evidence
    /// images local; the PR then names them by path.
    pub publish_evidence_images: bool,
}

impl Default for PrConfig {
    fn default() -> Self {
        Self {
            publish_evidence_images: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_publish_by_default_and_can_be_turned_off() {
        let parsed: PrConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, PrConfig::default());
        assert!(parsed.publish_evidence_images);
        let off: PrConfig = toml::from_str("publish_evidence_images = false").unwrap();
        assert!(!off.publish_evidence_images);
    }
}
