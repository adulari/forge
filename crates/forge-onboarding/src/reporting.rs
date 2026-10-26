//! The error-report choice. Adapter over the `[reporting] mode` config key: the reporting crate
//! is built separately, so this reads and writes the key directly. When that crate lands, swap
//! the two functions for `forge_reporting::{mode, set_mode}`.

use forge_config::ConfigScope;

pub const MODES: [&str; 3] = ["auto", "ask", "off"];

fn configured() -> Option<String> {
    let path = forge_config::scope_path(ConfigScope::User).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let doc: toml::Table = text.parse().ok()?;
    let mode = doc.get("reporting")?.get("mode")?.as_str()?.to_lowercase();
    MODES.contains(&mode.as_str()).then_some(mode)
}

/// The effective mode: what the config says, else `auto`.
pub fn mode() -> String {
    configured().unwrap_or_else(|| "auto".to_string())
}

/// Whether the person has chosen a mode already.
pub fn chosen() -> bool {
    configured().is_some()
}

pub fn set_mode(mode: &str) -> Result<(), String> {
    if !MODES.contains(&mode) {
        return Err(format!("{mode} is not an error report mode."));
    }
    forge_config::set_config_value(ConfigScope::User, "reporting.mode", mode)
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
