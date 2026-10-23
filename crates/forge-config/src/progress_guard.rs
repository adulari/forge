//! `[mesh] progress_guard`: what the progress supervisor does when a turn stops making progress
//! (docs/features/progress-supervision.md).

use serde::{Deserialize, Serialize};

/// `enforce` nudges once and then stops a stalled turn. `warn` reports every stop as "would have
/// stopped here" and lets the turn run on, which is how to calibrate the thresholds on real work.
/// `off` keeps only the loop-pattern nudges: nothing ever ends the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressGuardMode {
    #[default]
    Enforce,
    Warn,
    Off,
}

impl ProgressGuardMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ProgressGuardMode::Enforce => "enforce",
            ProgressGuardMode::Warn => "warn",
            ProgressGuardMode::Off => "off",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lowercase_and_defaults_to_enforce() {
        #[derive(Deserialize)]
        struct T {
            #[serde(default)]
            mode: ProgressGuardMode,
        }
        let t: T = toml::from_str("mode = \"warn\"").unwrap();
        assert_eq!(t.mode, ProgressGuardMode::Warn);
        let t: T = toml::from_str("").unwrap();
        assert_eq!(t.mode, ProgressGuardMode::Enforce);
        assert!(toml::from_str::<T>("mode = \"loud\"").is_err());
    }
}
