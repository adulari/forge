//! `keybindings.json` in the config directory: one shortcut file the desktop/web app, the daemon
//! and the TUI all read. `app` holds the app's action → chord overrides (kept verbatim, including
//! actions this build does not know, so an older client never erases a newer one's bindings);
//! `tui` holds terminal-UI chords that layer over `[keybinds.binds]` in `config.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{config_dir, ConfigError, KeyCombo, KeybindsConfig};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct KeybindingsFile {
    #[serde(default)]
    pub app: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub tui: BTreeMap<String, KeyCombo>,
}

pub fn keybindings_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("keybindings.json"))
}

/// A missing or unreadable file is an empty one: shortcuts must never block startup.
pub fn read_keybindings_file(path: &Path) -> KeybindingsFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn write_keybindings_file(path: &Path, file: &KeybindingsFile) -> Result<(), ConfigError> {
    let write_err = |e: std::io::Error| ConfigError::Write(e.to_string());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(write_err)?;
    }
    let body = serde_json::to_string_pretty(file).map_err(|e| ConfigError::Write(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(write_err)?;
    std::fs::rename(&tmp, path).map_err(write_err)
}

/// Replace only the `app` section, leaving the user's `tui` chords as they are.
pub fn write_app_overrides(
    path: &Path,
    app: BTreeMap<String, serde_json::Value>,
) -> Result<KeybindingsFile, ConfigError> {
    let mut file = read_keybindings_file(path);
    file.app = app;
    write_keybindings_file(path, &file)?;
    Ok(file)
}

/// Layer the file's `tui` chords over known actions; an action the TUI does not define is ignored.
pub fn overlay_tui(keybinds: &mut KeybindsConfig, file: &KeybindingsFile) {
    for (action, combo) in &file.tui {
        if keybinds.binds.contains_key(action) {
            keybinds.binds.insert(action.clone(), combo.clone());
        }
    }
}

pub(crate) fn overlay_user_file(keybinds: &mut KeybindsConfig) {
    if let Some(path) = keybindings_path().filter(|p| p.is_file()) {
        overlay_tui(keybinds, &read_keybindings_file(&path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_write_keeps_tui_chords_and_unknown_app_actions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keybindings.json");
        std::fs::write(
            &path,
            r#"{"app":{"future.action":{"key":"x"}},"tui":{"undo":{"key":"u","ctrl":true}}}"#,
        )
        .unwrap();
        let mut app = read_keybindings_file(&path).app;
        app.insert(
            "nav.fleet".into(),
            serde_json::json!({"key":"9","alt":true}),
        );
        let written = write_app_overrides(&path, app).unwrap();
        let again = read_keybindings_file(&path);
        assert_eq!(written, again);
        assert!(again.app.contains_key("future.action"));
        assert!(again.app.contains_key("nav.fleet"));
        assert_eq!(again.tui["undo"].key, "u");
    }

    #[test]
    fn tui_overlay_rebinds_known_actions_only() {
        let mut keybinds = KeybindsConfig::default();
        let mut file = KeybindingsFile::default();
        file.tui.insert(
            "undo".into(),
            KeyCombo {
                key: "u".into(),
                ctrl: true,
                alt: false,
                shift: false,
            },
        );
        file.tui.insert(
            "no_such_action".into(),
            KeyCombo {
                key: "q".into(),
                ctrl: false,
                alt: false,
                shift: false,
            },
        );
        overlay_tui(&mut keybinds, &file);
        assert_eq!(keybinds.binds["undo"].key, "u");
        assert!(!keybinds.binds.contains_key("no_such_action"));
    }

    #[test]
    fn a_missing_or_corrupt_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keybindings.json");
        assert_eq!(read_keybindings_file(&path), KeybindingsFile::default());
        std::fs::write(&path, "{nope").unwrap();
        assert_eq!(read_keybindings_file(&path), KeybindingsFile::default());
    }
}
