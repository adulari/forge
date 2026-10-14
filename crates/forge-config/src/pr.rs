//! `[pr]`: how the `open_pr` tool publishes a pull request. See docs/features/open-pr.md.

use std::path::Path;

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

/// `[pr] publish_evidence_images` as `dir/.forge/config.toml` sets it, if it does. Read per call by
/// `open_pr`, since a daemon's startup config came from no particular repository.
pub fn repo_publish_evidence_images(dir: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(dir.join(".forge").join("config.toml")).ok()?;
    let value: toml::Table = toml::from_str(&text).ok()?;
    value.get("pr")?.get("publish_evidence_images")?.as_bool()
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

    #[test]
    fn a_repo_config_can_turn_publishing_off() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(repo_publish_evidence_images(dir.path()), None);
        std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
        std::fs::write(dir.path().join(".forge/config.toml"), "[mesh]\n").unwrap();
        assert_eq!(repo_publish_evidence_images(dir.path()), None);
        std::fs::write(
            dir.path().join(".forge/config.toml"),
            "[pr]\npublish_evidence_images = false\n",
        )
        .unwrap();
        assert_eq!(repo_publish_evidence_images(dir.path()), Some(false));
    }
}
