//! `[mesh.budget] hard_stop`: whether a spend cap ends a turn or only warns.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `true` refuses and stops everywhere (the behaviour before `auto` existed), `false` only ever
/// warns on the USD caps, and `"auto"` (the default) stops an unattended turn and only warns an
/// attended one, whose person can see the spend and press escape. The session, daily, weekly and
/// monthly caps act between turns and are never relaxed by `auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HardStop {
    Always,
    Never,
    #[default]
    Auto,
}

impl HardStop {
    /// Whether the per-turn USD cap ends the turn.
    pub fn stops_turn_usd(self, unattended: bool) -> bool {
        match self {
            HardStop::Always => true,
            HardStop::Never => false,
            HardStop::Auto => unattended,
        }
    }

    /// Whether the per-turn token ceilings (`max_turn_input_tokens`, `bridge_turn_token_cap`) end
    /// the turn. They never relaxed with `false` (a token ceiling is not a USD cap), only with
    /// `auto` on an attended turn.
    pub fn stops_turn_tokens(self, unattended: bool) -> bool {
        match self {
            HardStop::Always | HardStop::Never => true,
            HardStop::Auto => unattended,
        }
    }

    /// Whether a cap that acts between turns (session, daily, weekly, monthly) is enforced.
    pub fn enforces_between_turns(self) -> bool {
        self != HardStop::Never
    }
}

impl From<bool> for HardStop {
    fn from(value: bool) -> Self {
        if value {
            HardStop::Always
        } else {
            HardStop::Never
        }
    }
}

impl Serialize for HardStop {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            HardStop::Always => serializer.serialize_bool(true),
            HardStop::Never => serializer.serialize_bool(false),
            HardStop::Auto => serializer.serialize_str("auto"),
        }
    }
}

impl<'de> Deserialize<'de> for HardStop {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Accept;
        impl Visitor<'_> for Accept {
            type Value = HardStop;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("true, false or \"auto\"")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<HardStop, E> {
                Ok(value.into())
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<HardStop, E> {
                match value.trim().to_ascii_lowercase().as_str() {
                    "auto" => Ok(HardStop::Auto),
                    "true" => Ok(HardStop::Always),
                    "false" => Ok(HardStop::Never),
                    other => Err(E::custom(format!(
                        "hard_stop must be true, false or \"auto\", not {other:?}"
                    ))),
                }
            }
        }
        deserializer.deserialize_any(Accept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize, Serialize)]
    struct T {
        hard_stop: HardStop,
    }

    #[test]
    fn accepts_a_bool_or_auto_and_round_trips() {
        for (text, want) in [
            ("hard_stop = true", HardStop::Always),
            ("hard_stop = false", HardStop::Never),
            ("hard_stop = \"auto\"", HardStop::Auto),
        ] {
            let parsed: T = toml::from_str(text).unwrap();
            assert_eq!(parsed.hard_stop, want, "{text}");
            let again: T = toml::from_str(&toml::to_string(&parsed).unwrap()).unwrap();
            assert_eq!(again.hard_stop, want);
        }
        assert!(toml::from_str::<T>("hard_stop = \"sometimes\"").is_err());
        assert_eq!(HardStop::default(), HardStop::Auto);
    }

    #[test]
    fn auto_stops_unattended_turns_and_warns_attended_ones() {
        assert!(HardStop::Auto.stops_turn_usd(true));
        assert!(!HardStop::Auto.stops_turn_usd(false));
        assert!(HardStop::Auto.stops_turn_tokens(true));
        assert!(!HardStop::Auto.stops_turn_tokens(false));
        assert!(HardStop::Always.stops_turn_usd(false));
        assert!(!HardStop::Never.stops_turn_usd(true));
        assert!(
            HardStop::Never.stops_turn_tokens(false),
            "false relaxes the USD caps, not the token ceilings"
        );
        assert!(HardStop::Auto.enforces_between_turns());
        assert!(!HardStop::Never.enforces_between_turns());
    }
}
