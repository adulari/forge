//! `[host]`: ceilings on what one `forge serve` daemon will run at once, so a runaway client or an
//! eager agent cannot exhaust a shared machine. Every limit is off (`0`) by default.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostConfig {
    /// Sessions the daemon keeps live at once. A new session past this is refused. `0` = no limit.
    pub max_active_sessions: u32,
    /// Terminals open across every session. A new terminal past this is refused. `0` = no limit.
    pub max_terminals: u32,
    /// Subagent helpers running at once across every session. Extra helpers wait for a free slot
    /// rather than failing the turn that spawned them. `0` = no limit.
    pub max_helpers: u32,
}

impl HostConfig {
    /// `Some(limit)` when a ceiling is set, `None` when the value means "no limit".
    pub fn limit(value: u32) -> Option<usize> {
        (value > 0).then_some(value as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_limit_is_off_until_set() {
        let parsed: HostConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, HostConfig::default());
        assert_eq!(HostConfig::limit(parsed.max_active_sessions), None);
        let set: HostConfig = toml::from_str("max_terminals = 4\nmax_helpers = 2").unwrap();
        assert_eq!(HostConfig::limit(set.max_terminals), Some(4));
        assert_eq!(HostConfig::limit(set.max_helpers), Some(2));
        assert_eq!(HostConfig::limit(set.max_active_sessions), None);
    }
}
