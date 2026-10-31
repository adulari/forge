//! Which control tools the user switched off. One small JSON file in the config directory, read
//! by every process that builds a tool list (the daemon, a terminal session, the MCP bridges), so
//! a toggle in any settings page reaches all of them without a restart.

use std::path::{Path, PathBuf};

use forge_core::control;

const FILE: &str = "control-tools.json";

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    disabled: Vec<String>,
}

pub(crate) fn path() -> Option<PathBuf> {
    forge_config::config_dir().map(|d| d.join(FILE))
}

pub(crate) fn disabled() -> Vec<String> {
    path().map(|p| disabled_at(&p)).unwrap_or_default()
}

pub(crate) fn disabled_at(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .map(|s| s.disabled)
        .unwrap_or_default()
}

/// Switch one tool on or off. Unknown names are refused so a typo cannot silently do nothing.
pub(crate) fn set_enabled(name: &str, enabled: bool) -> Result<Vec<String>, String> {
    let path = path().ok_or("no config directory on this machine")?;
    set_enabled_at(&path, name, enabled)
}

pub(crate) fn set_enabled_at(
    path: &Path,
    name: &str,
    enabled: bool,
) -> Result<Vec<String>, String> {
    if !control::is_control_tool(name) {
        return Err(format!("no control tool named {name}"));
    }
    let mut list = disabled_at(path);
    list.retain(|n| n != name);
    if !enabled {
        list.push(name.to_string());
    }
    list.sort();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(&Stored {
        disabled: list.clone(),
    })
    .map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_round_trips_and_rejects_unknown_names() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join(FILE);
        assert!(disabled_at(&file).is_empty());
        let list = set_enabled_at(&file, "merge_worktree", false).unwrap();
        assert_eq!(list, vec!["merge_worktree"]);
        set_enabled_at(&file, "add_schedule", false).unwrap();
        assert_eq!(disabled_at(&file), vec!["add_schedule", "merge_worktree"]);
        set_enabled_at(&file, "merge_worktree", true).unwrap();
        assert_eq!(disabled_at(&file), vec!["add_schedule"]);
        assert!(set_enabled_at(&file, "rm_rf", false).is_err());
    }

    #[test]
    fn a_corrupt_file_means_nothing_is_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        std::fs::write(&file, "not json").unwrap();
        assert!(disabled_at(&file).is_empty());
    }
}
