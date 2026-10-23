//! Notices shown once per installation, not once per session.
//!
//! A line that is true and useful the first time (the terms a subscription CLI bridge runs under)
//! turns into noise when every new session repeats it. Each notice leaves an empty marker file
//! beside the store, so "already shown" survives restarts and is shared by every project and
//! session of one installation.
//!
//! Where the markers live follows the store: `<FORGE_DB>.notices/` when a scratch or
//! operator-selected store is in use, so an isolated run (tests, benchmarks) never touches the
//! real data directory and sees every notice as new; otherwise `<data dir>/notices/`.

use std::path::{Path, PathBuf};

/// The CLI-bridge terms notice (`forge_provider::CLI_BRIDGE_NOTICE`).
pub const CLI_BRIDGE_TERMS: &str = "cli-bridge-terms";

fn notices_dir() -> PathBuf {
    if let Some(db) = std::env::var_os("FORGE_DB").filter(|v| !v.is_empty()) {
        let mut beside = db;
        beside.push(".notices");
        return PathBuf::from(beside);
    }
    match crate::data_dir() {
        Some(data) => data.join("notices"),
        None => std::env::temp_dir().join("forge-notices"),
    }
}

fn marker_in(dir: &Path, name: &str) -> Option<PathBuf> {
    // A name is a fixed constant of this crate; refuse anything that could leave the directory.
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        .then(|| dir.join(name))
}

fn seen_in(dir: &Path, name: &str) -> bool {
    marker_in(dir, name).is_some_and(|path| path.is_file())
}

fn mark_in(dir: &Path, name: &str) {
    let Some(path) = marker_in(dir, name) else {
        return;
    };
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(path, b"");
}

/// Whether the notice `name` has already been shown on this installation.
pub fn install_notice_seen(name: &str) -> bool {
    seen_in(&notices_dir(), name)
}

/// Record that the notice `name` was shown. Best effort: a read-only data directory only means
/// the notice shows again next session.
pub fn mark_install_notice_seen(name: &str) {
    mark_in(&notices_dir(), name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_name_cannot_escape_the_notices_directory() {
        let dir = Path::new("/nonexistent/notices");
        assert!(marker_in(dir, "../x").is_none());
        assert!(marker_in(dir, "a/b").is_none());
        assert!(marker_in(dir, CLI_BRIDGE_TERMS).is_some());
    }

    #[test]
    fn a_notice_is_new_until_it_is_marked_and_stays_seen() {
        let dir = std::env::temp_dir().join(format!("forge-notices-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!seen_in(&dir, CLI_BRIDGE_TERMS));
        mark_in(&dir, CLI_BRIDGE_TERMS);
        assert!(seen_in(&dir, CLI_BRIDGE_TERMS), "survives a new process");
        assert!(!seen_in(&dir, "another-notice"), "one marker per notice");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
