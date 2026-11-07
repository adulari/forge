//! `[harness] scouts`: throwaway read-only orientation runs (docs/features/scouts.md).

use serde::{Deserialize, Serialize};

/// Whether, and how loudly, the model is offered the `scout` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScoutsMode {
    /// Advertise `scout` and tell the model to call it first, before reading code itself, when a
    /// task needs orientation in code it has not seen.
    Auto,
    /// Advertise `scout`; the model decides when it helps.
    #[default]
    Tool,
    /// No `scout` tool.
    Off,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_spellings_and_defaults_to_tool() {
        for (text, want) in [
            ("auto", ScoutsMode::Auto),
            ("tool", ScoutsMode::Tool),
            ("off", ScoutsMode::Off),
        ] {
            let cfg: crate::HarnessConfig =
                toml::from_str(&format!("scouts = \"{text}\"")).unwrap();
            assert_eq!(cfg.scouts, want);
        }
        let cfg: crate::HarnessConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.scouts, ScoutsMode::Tool);
    }
}
