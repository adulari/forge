//! `[reporting]` — automatic issue reports (see `docs/features/issue-reporting.md`).
//!
//! The mode lives here (not in `forge-reporting`) so config loading, the `/config` editor and the
//! onboarding step can all name it without depending on the reporting crate.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{config_dir, toml_write, ConfigError};

/// How Forge reports its own bugs to the maintainers. On by default; `FORGE_NO_REPORTS=1` always
/// wins over this setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportingMode {
    /// Send a scrubbed report in the background and show a subtle "reported" mark.
    #[default]
    Auto,
    /// Show a card with the exact report and let the user send it, skip it, or always send.
    Ask,
    /// Never collect or send anything.
    Off,
}

impl ReportingMode {
    pub const ALL: [ReportingMode; 3] = [Self::Auto, Self::Ask, Self::Off];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ask => "ask",
            Self::Off => "off",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" | "automatic" | "on" => Some(Self::Auto),
            "ask" => Some(Self::Ask),
            "off" | "never" | "none" => Some(Self::Off),
            _ => None,
        }
    }
}

/// The `[reporting]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReportingConfig {
    #[serde(default)]
    pub mode: ReportingMode,
}

/// Persist `[reporting] mode` in the user config, preserving every other key and comment.
pub fn write_reporting_mode(mode: ReportingMode) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    let mut doc = toml_write::load_doc(&path)?;
    toml_write::set_dotted(
        &mut doc,
        "reporting.mode",
        &toml::Value::String(mode.as_str().to_string()),
    );
    toml_write::save_doc(&path, &doc)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_auto_and_parses_spellings() {
        assert_eq!(ReportingConfig::default().mode, ReportingMode::Auto);
        assert_eq!(ReportingMode::parse("Ask"), Some(ReportingMode::Ask));
        assert_eq!(ReportingMode::parse("never"), Some(ReportingMode::Off));
        assert_eq!(ReportingMode::parse("maybe"), None);
    }

    #[test]
    fn serde_uses_lowercase_names() {
        let json = serde_json::to_string(&ReportingMode::Ask).unwrap();
        assert_eq!(json, "\"ask\"");
        let back: ReportingMode = serde_json::from_str("\"off\"").unwrap();
        assert_eq!(back, ReportingMode::Off);
    }
}
