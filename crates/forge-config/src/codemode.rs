//! `[codemode]`: the `run_code` tool on the direct path. A model-written JS program calls Forge's
//! other tools as `await tools.<name>(args)` in one step. See docs/features/codemode.md.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CodemodeConfig {
    /// Advertise `run_code` to direct-path models. `false` is the kill switch: the tool is not
    /// advertised and a call to it is refused as an unknown tool. CLI bridges never see it.
    pub enabled: bool,
    /// Default script budget in seconds, excluding time spent waiting on a permission prompt.
    pub timeout_secs: u64,
    /// Ceiling on a script's own `timeout_secs` argument.
    pub max_timeout_secs: u64,
    /// Inner tool calls one script may make before further calls throw.
    pub max_calls: usize,
    /// Characters of script output (return value, logs, footer) kept before truncation.
    pub result_max_chars: usize,
}

impl Default for CodemodeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_secs: 300,
            max_timeout_secs: 900,
            max_calls: 40,
            result_max_chars: 16_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_table_ships_enabled_with_the_documented_limits() {
        let parsed: CodemodeConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, CodemodeConfig::default());
        assert!(parsed.enabled);
        assert_eq!(parsed.max_calls, 40);
    }

    #[test]
    fn the_kill_switch_parses() {
        let parsed: CodemodeConfig = toml::from_str("enabled = false").unwrap();
        assert!(!parsed.enabled);
        assert_eq!(parsed.timeout_secs, 300);
    }
}
