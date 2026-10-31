//! The per-thread "delegation off" switch, readable from another process.
//!
//! `/delegation off` lives in the session, but a claude-cli or codex-cli turn gets its tools from
//! a separate `forge mcp-serve` process that cannot see it. The session leaves a marker file
//! named after its id; the bridge reads it before advertising or running a delegation tool. The
//! file holds the owning process's pid so a marker left by a process that has since died (the
//! switch is not persisted across a restart) is ignored.

use std::path::{Path, PathBuf};

fn flag_dir() -> Option<PathBuf> {
    forge_config::data_dir().map(|d| d.join("delegation-off"))
}

fn safe_id(session_id: &str) -> Option<&str> {
    (!session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(session_id)
}

fn alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        Path::new(&format!("/proc/{pid}")).exists()
    } else {
        true
    }
}

fn set_in(dir: &Path, session_id: &str, off: bool) {
    let Some(id) = safe_id(session_id) else {
        return;
    };
    let path = dir.join(id);
    if off {
        if std::fs::create_dir_all(dir).is_ok() {
            let _ = std::fs::write(&path, std::process::id().to_string());
        }
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

fn is_off_in(dir: &Path, session_id: &str) -> bool {
    let Some(id) = safe_id(session_id) else {
        return false;
    };
    std::fs::read_to_string(dir.join(id))
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .is_some_and(alive)
}

/// Record that delegation is (`off`) or is no longer withheld for `session_id`. Best effort.
pub fn set(session_id: &str, off: bool) {
    if let Some(dir) = flag_dir() {
        set_in(&dir, session_id, off);
    }
}

/// Whether delegation is withheld for `session_id` by a live process.
pub fn is_off(session_id: &str) -> bool {
    flag_dir().is_some_and(|dir| is_off_in(&dir, session_id))
}

/// [`is_off`] for the session named in the bridge environment (`FORGE_CHECKPOINT_SESSION`).
pub fn is_off_from_env() -> bool {
    std::env::var(crate::snapshot::ENV_SESSION).is_ok_and(|id| is_off(&id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_round_trips_and_ignores_dead_owners_and_odd_ids() {
        let dir = std::env::temp_dir().join(format!("forge-deleg-{}", std::process::id()));
        assert!(!is_off_in(&dir, "s1"));
        set_in(&dir, "s1", true);
        assert!(is_off_in(&dir, "s1"));
        assert!(!is_off_in(&dir, "s2"));
        set_in(&dir, "s1", false);
        assert!(!is_off_in(&dir, "s1"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dead"), "4294967295").unwrap();
        assert_eq!(is_off_in(&dir, "dead"), !cfg!(target_os = "linux"));
        set_in(&dir, "../escape", true);
        assert!(!dir.parent().unwrap().join("escape").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
