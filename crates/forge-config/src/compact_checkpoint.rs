//! `[compact]`: the checkpoint compaction at a landed slice (docs/features/context-compaction.md,
//! "Checkpoint at a landed slice").

use serde::{Deserialize, Serialize};

/// Live context size below which a landed slice is not worth folding.
pub const DEFAULT_CHECKPOINT_MIN_TOKENS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactConfig {
    /// Fold a finished slice of work (a commit the agent made, after a passing check) into a
    /// summary the harness writes itself, so the next slice starts lean. Off by default (opt in
    /// with `true`); `false` leaves
    /// compaction to the size trigger alone.
    pub checkpoint: bool,
    /// Live context, in tokens, a landed slice must exceed before it is folded.
    pub checkpoint_min_tokens: u64,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            checkpoint: false,
            checkpoint_min_tokens: DEFAULT_CHECKPOINT_MIN_TOKENS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_and_both_keys_are_settable() {
        let config = CompactConfig::default();
        assert!(!config.checkpoint);
        assert_eq!(config.checkpoint_min_tokens, 60_000);
        let parsed: CompactConfig =
            toml::from_str("checkpoint = false\ncheckpoint_min_tokens = 90000").unwrap();
        assert!(!parsed.checkpoint);
        assert_eq!(parsed.checkpoint_min_tokens, 90_000);
        let partial: CompactConfig = toml::from_str("checkpoint = false").unwrap();
        assert_eq!(partial.checkpoint_min_tokens, 60_000);
    }
}
