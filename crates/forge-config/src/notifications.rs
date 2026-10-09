//! `[notifications]`: tell the user a long turn finished or that Forge is waiting on them, the
//! way Claude Code's terminal notifications do. Off by default.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// Master switch. Nothing below does anything while this is false.
    pub enabled: bool,
    /// Send a desktop notification (`notify-send` on Linux, `osascript` on macOS).
    pub desktop: bool,
    /// Ring the terminal bell (BEL), which most terminals turn into a taskbar/tab flag.
    pub bell: bool,
    /// A finished turn only notifies when it ran at least this many seconds.
    pub min_turn_secs: u64,
    /// Notify even while the terminal has focus. Off: only when the window is unfocused. Terminals
    /// that never report focus (no focus-events in tmux) need this on to notify at all.
    pub when_focused: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            desktop: true,
            bell: true,
            min_turn_secs: 15,
            when_focused: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default() {
        let cfg: NotificationsConfig = toml::from_str("").unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg, NotificationsConfig::default());
    }

    #[test]
    fn partial_table_keeps_other_defaults() {
        let cfg: NotificationsConfig =
            toml::from_str("enabled = true\nmin_turn_secs = 60\nbell = false").unwrap();
        assert!(cfg.enabled && cfg.desktop && !cfg.bell);
        assert_eq!(cfg.min_turn_secs, 60);
    }
}
