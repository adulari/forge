//! Session workspace identity.
//!
//! A session owns an immutable canonical root so daemon-hosted sessions never
//! fall back to the process working directory.

use std::path::{Path, PathBuf};

use crate::CoreError;

/// Immutable filesystem identity for one session. A daemon may host sessions from
/// different worktrees concurrently, so this must never be inferred from process cwd.
#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    root: PathBuf,
}

impl WorkspaceContext {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, CoreError> {
        let requested = root.as_ref();
        let root = requested
            .canonicalize()
            .map_err(|error| CoreError::Workspace(error.to_string()))?;
        if !root.is_dir() {
            return Err(CoreError::Workspace(format!(
                "not a directory: {}",
                root.display()
            )));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn display(&self) -> String {
        self.root.display().to_string()
    }
}
