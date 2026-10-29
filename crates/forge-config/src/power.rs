//! `[power]`: whether the host keeps itself awake. A laptop that idles into sleep takes a running
//! `forge serve` with it, so a turn dies mid-flight and the phone driving it loses the session.

use serde::{Deserialize, Serialize};

/// When `forge serve` holds an idle/sleep inhibitor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAwake {
    /// Never hold one.
    Off,
    /// Hold one while a session has a turn running.
    #[default]
    WhileWorking,
    /// Hold one while a turn is running or any app is connected to the daemon.
    WhileRemote,
    /// Hold one for as long as the daemon runs.
    Always,
}

impl KeepAwake {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::WhileWorking => "while_working",
            Self::WhileRemote => "while_remote",
            Self::Always => "always",
        }
    }

    /// Whether the inhibitor should be held right now.
    pub fn wants_inhibitor(self, working: bool, clients_attached: bool) -> bool {
        match self {
            Self::Off => false,
            Self::WhileWorking => working,
            Self::WhileRemote => working || clients_attached,
            Self::Always => true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerConfig {
    /// `off`, `while_working` (default), `while_remote` or `always`. Stops the computer sleeping;
    /// the screen can still lock.
    pub keep_awake: KeepAwake,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_awake_while_working_and_parses_every_mode() {
        assert_eq!(PowerConfig::default().keep_awake, KeepAwake::WhileWorking);
        for (text, mode) in [
            ("off", KeepAwake::Off),
            ("while_working", KeepAwake::WhileWorking),
            ("while_remote", KeepAwake::WhileRemote),
            ("always", KeepAwake::Always),
        ] {
            let parsed: PowerConfig = toml::from_str(&format!("keep_awake = \"{text}\"")).unwrap();
            assert_eq!(parsed.keep_awake, mode);
            assert_eq!(mode.wire_name(), text);
        }
        assert!(toml::from_str::<PowerConfig>("keep_awake = \"sometimes\"").is_err());
    }

    #[test]
    fn each_mode_holds_the_inhibitor_for_its_own_reasons() {
        let cases = [
            (KeepAwake::Off, [false, false, false, false]),
            (KeepAwake::WhileWorking, [false, false, true, true]),
            (KeepAwake::WhileRemote, [false, true, true, true]),
            (KeepAwake::Always, [true, true, true, true]),
        ];
        for (mode, expected) in cases {
            let got = [
                mode.wants_inhibitor(false, false),
                mode.wants_inhibitor(false, true),
                mode.wants_inhibitor(true, false),
                mode.wants_inhibitor(true, true),
            ];
            assert_eq!(got, expected, "{mode:?}");
        }
    }
}
