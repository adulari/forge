//! The error-report choice, as the onboarding step sees it: a thin string-typed adapter over
//! `forge_reporting::{mode, set_mode}`.

use forge_config::ConfigScope;
use forge_reporting::ReportingMode;

pub const MODES: [&str; 3] = ["auto", "ask", "off"];

/// The effective mode (`FORGE_NO_REPORTS` wins, else the config, else `auto`).
pub fn mode() -> String {
    forge_reporting::mode().as_str().to_string()
}

/// Whether the person has written a mode to their config already.
pub fn chosen() -> bool {
    let Ok(path) = forge_config::scope_path(ConfigScope::User) else {
        return false;
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|doc| {
            doc.get("reporting")?
                .get("mode")?
                .as_str()
                .map(ReportingMode::parse)
        })
        .is_some_and(|mode| mode.is_some())
}

pub fn set_mode(mode: &str) -> Result<(), String> {
    let Some(parsed) = ReportingMode::parse(mode).filter(|_| MODES.contains(&mode)) else {
        return Err(format!("{mode} is not an error report mode."));
    };
    forge_reporting::set_mode(parsed)
        .map_err(|e| format!("Could not save the error report setting: {e}"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, ScopedEnv};

    #[test]
    fn the_mode_defaults_to_auto_and_round_trips_through_the_config() {
        let _g = env_lock();
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join("config");
        let _env = ScopedEnv::set(&[
            ("HOME", Some(home.path().as_os_str())),
            ("XDG_CONFIG_HOME", Some(config.as_os_str())),
        ]);
        assert_eq!(mode(), "auto");
        assert!(!chosen());
        set_mode("ask").unwrap();
        assert_eq!(mode(), "ask");
        assert!(chosen());
        set_mode("off").unwrap();
        assert_eq!(mode(), "off");
        assert!(set_mode("sometimes").is_err());
    }
}
