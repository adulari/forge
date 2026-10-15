//! `[mesh] compact_model`: which model writes the summary when the context is compacted
//! (docs/features/context-compaction.md).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompactModel {
    /// The model the session is running on (the `/model` pin, else the model the last turn was
    /// routed to), then its fallbacks, then the free shortlist. The default: a summary replaces
    /// everything it folds, so it is written by the model that knows the task.
    #[default]
    Session,
    /// The best free capable model first and the session's own model last, the order Forge used
    /// before `compact_model` existed. Spends nothing on the session's model while a free one works.
    Auto,
}

impl CompactModel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Auto => "auto",
        }
    }
}
