//! `[chat]`: what Enter does with a prompt typed while a turn is running.

use serde::{Deserialize, Serialize};

/// How a prompt sent mid-turn reaches the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpMode {
    /// Queue it and also hand it to the running turn at its next step.
    #[default]
    Steer,
    /// Queue it to run after the turn ends; the running turn never sees it.
    Queue,
}

impl FollowUpMode {
    /// The mode the alternate send chord (Cmd/Ctrl+Enter) uses.
    pub fn inverse(self) -> Self {
        match self {
            Self::Steer => Self::Queue,
            Self::Queue => Self::Steer,
        }
    }

    pub fn steers(self) -> bool {
        self == Self::Steer
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatConfig {
    /// `steer` (default) or `queue`: whether a prompt sent while a turn runs is also offered to
    /// that turn at its next step, or only queued behind it.
    pub follow_up: FollowUpMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_parses_and_inverts() {
        assert_eq!(ChatConfig::default().follow_up, FollowUpMode::Steer);
        let queue: ChatConfig = toml::from_str("follow_up = \"queue\"").unwrap();
        assert_eq!(queue.follow_up, FollowUpMode::Queue);
        assert!(!queue.follow_up.steers());
        assert_eq!(queue.follow_up.inverse(), FollowUpMode::Steer);
        assert!(toml::from_str::<ChatConfig>("follow_up = \"later\"").is_err());
    }
}
