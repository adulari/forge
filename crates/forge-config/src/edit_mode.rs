//! `[harness] edit_mode`: how the model edits existing files.

use serde::{Deserialize, Serialize};

/// `replace` is `edit_file`/`multi_edit` with exact `old` -> `new` text. `hashline` tags every
/// line `read_file` returns with its number under a per-file tag, and `edit_file` takes line
/// anchors plus that tag instead (docs/features/hashline-edits.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EditMode {
    #[default]
    Replace,
    Hashline,
}

impl EditMode {
    pub fn is_hashline(self) -> bool {
        self == Self::Hashline
    }
}
