//! `[worktree]`: where Forge puts session worktrees and what it does around creating/removing
//! them. Only the user config can set it: an untrusted project's table is dropped by
//! `project_trust::sanitize_untrusted`, because `setup` and `cleanup` are shell commands.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeRoot {
    /// `<repo>/.forge/worktrees/<name>` (self-ignored, next to the checkout).
    #[default]
    Project,
    /// `<data dir>/worktrees/<repo>-<hash>/<name>`, outside every repository.
    Global,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeConfig {
    /// Where new worktrees are created. Default: `project`.
    pub root: WorktreeRoot,
    /// Shell command run in the new worktree right after it is created (install dependencies,
    /// generate files). A failure is reported but never removes the worktree.
    pub setup: Option<String>,
    /// Shell command run in a worktree just before Forge removes it.
    pub cleanup: Option<String>,
    /// Git-ignored files copied from the main checkout into every new worktree, as glob patterns
    /// relative to the repository root, for example `[".env*", ".vscode"]`. Tracked files are
    /// never copied (the checkout already has them).
    pub copy_ignored: Vec<String>,
}
