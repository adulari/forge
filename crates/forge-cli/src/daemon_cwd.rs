//! A working directory safe to hand a process that outlives the one spawning it.
//!
//! A detached daemon inherits the spawner's cwd unless told otherwise. Forge frequently runs from a
//! directory that is later removed — a git worktree, a scratch dir, a test temp dir — and the
//! daemon outlives it, holding a deleted cwd for the rest of its life.
//!
//! That failure is quiet and confusing. Observed with `ollama serve`: its cwd was
//! `/tmp/.tmpK3dDVO (deleted)`, and because ollama resolves a path on every model load, EVERY
//! local model call then died with
//!
//! ```text
//! llama-server process has terminated: error: cannot get current path: No such file or directory
//! ```
//!
//! while `forge doctor` still reported ollama healthy, the port still listened, and `/api/tags`
//! still answered. Only inference was dead, so the mesh saw the model as merely "unavailable" and
//! silently failed over to a worse one. `git` behaves the same way — it calls `getcwd()` at startup
//! and fails with "fatal: Unable to read current working directory" even when given `-C`.
//!
//! Diagnosis recipe: `readlink /proc/<pid>/cwd`; a `(deleted)` suffix is the tell.

use std::path::PathBuf;

/// A directory that will still exist an hour from now.
///
/// The home directory is the stable choice. The temp ROOT is an acceptable fallback because it
/// persists — unlike directories created *inside* it, which are exactly the disposable parents that
/// cause this bug.
pub(crate) fn stable_daemon_cwd() -> PathBuf {
    forge_config::home_dir()
        .filter(|home| home.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_cwd_is_a_directory_that_outlives_this_process() {
        let cwd = stable_daemon_cwd();
        assert!(
            cwd.is_dir(),
            "daemon cwd must exist at spawn time: {}",
            cwd.display()
        );

        // It must not sit INSIDE the temp root — the disposable-parent case that produced the bug.
        // The temp root itself is an acceptable last-resort fallback.
        let temp_root = std::env::temp_dir();
        assert!(
            cwd == temp_root || !cwd.starts_with(&temp_root),
            "daemon cwd must not be a directory created inside the temp root: {}",
            cwd.display()
        );
    }
}
