//! `[keybinds]`: the action → key-combo map and the named presets layered under it.
//!
//! `preset = "claude-code"` moves Forge's own actions off the chords Claude Code users press by
//! habit (Ctrl+K kill-line, Ctrl+N/P history, Ctrl+Z suspend, Ctrl+S stash, Ctrl+L redraw, Ctrl+R
//! history search, Alt+P model, Alt+M mode, Alt+T thinking). A bind the user set explicitly in
//! `[keybinds.binds]` always wins over the preset.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{config_dir, toml_write, ConfigError};

/// A key combination: a key name plus modifier flags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    /// Key name: a single character (e.g. "c", "k", ","), or a named key
    /// ("up", "down", "left", "right", "enter", "esc", "backspace", "delete",
    /// "pageup", "pagedown", "home", "end", "tab", "f1"–"f12"); names are case-insensitive.
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
}

impl KeyCombo {
    fn new(key: &str, ctrl: bool, alt: bool, shift: bool) -> Self {
        Self {
            key: key.to_string(),
            ctrl,
            alt,
            shift,
        }
    }
}

/// Which built-in keymap the action binds start from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeybindPreset {
    /// Forge's own defaults (Ctrl+K command center, Ctrl+N new session, Ctrl+Z undo write…).
    #[default]
    Forge,
    /// Claude Code muscle memory: Forge actions move to free chords so CC's keys keep CC's meaning.
    ClaudeCode,
}

impl KeybindPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forge => "forge",
            Self::ClaudeCode => "claude-code",
        }
    }
}

/// Configurable keybind map: action name → key combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindsConfig {
    /// The built-in keymap the binds start from (`"forge"` or `"claude-code"`).
    #[serde(default)]
    pub preset: KeybindPreset,
    #[serde(default)]
    pub binds: BTreeMap<String, KeyCombo>,
}

impl Default for KeybindsConfig {
    fn default() -> Self {
        let bind = KeyCombo::new;
        let mut binds = BTreeMap::new();
        binds.insert("interrupt".into(), bind("c", true, false, false));
        binds.insert("command_palette".into(), bind("/", false, false, false));
        binds.insert("command_center".into(), bind("k", true, false, false));
        binds.insert("skip_model".into(), bind("k", true, false, true));
        binds.insert("tier_up".into(), bind("up", true, false, false));
        binds.insert("tier_down".into(), bind("down", true, false, false));
        binds.insert("toggle_reasoning".into(), bind("r", true, false, false));
        binds.insert("toggle_tool_card".into(), bind("t", true, false, false));
        binds.insert("toggle_tool_groups".into(), bind("o", false, true, false));
        binds.insert("prev_tool_group".into(), bind("up", false, true, false));
        binds.insert("next_tool_group".into(), bind("down", false, true, false));
        binds.insert("undo".into(), bind("z", true, false, false));
        binds.insert("compact".into(), bind("l", false, true, false));
        binds.insert("redraw".into(), bind("l", true, false, false));
        binds.insert("model_picker".into(), bind("m", false, true, false));
        binds.insert("effort_cycle".into(), bind("e", false, true, false));
        binds.insert("temper_cycle".into(), bind("t", false, true, false));
        binds.insert("keybind_config".into(), bind(",", true, false, false));
        binds.insert("new_session".into(), bind("n", true, false, false));
        binds.insert("copy_last".into(), bind("c", true, false, true));
        binds.insert("scroll_up".into(), bind("pageup", false, false, false));
        binds.insert("scroll_down".into(), bind("pagedown", false, false, false));
        binds.insert("help".into(), bind("f1", false, false, false));
        binds.insert("checkpoint".into(), bind("s", true, false, false));
        binds.insert("reload".into(), bind("r", false, true, false));
        // Ctrl+V: free in the default keymap (paste arrives via bracketed-paste as a distinct
        // terminal event, never as a Ctrl+V keystroke) — see voice.md.
        binds.insert("voice".into(), bind("v", true, false, false));
        binds.insert("kill_line_forward".into(), bind("k", false, true, false));
        Self {
            preset: KeybindPreset::Forge,
            binds,
        }
    }
}

/// The actions the `claude-code` preset moves, and where to. Every other action keeps its Forge
/// default. The freed chords fall through to the composer's readline keys (Ctrl+K kill to end of
/// line, Ctrl+N history) or are ignored (Ctrl+Z, Ctrl+S, Ctrl+R), never typed as a letter.
fn claude_code_moves() -> [(&'static str, KeyCombo); 7] {
    [
        ("command_center", KeyCombo::new("f2", false, false, false)),
        ("new_session", KeyCombo::new("n", false, true, false)),
        ("undo", KeyCombo::new("z", false, true, false)),
        ("checkpoint", KeyCombo::new("s", false, true, false)),
        ("toggle_reasoning", KeyCombo::new("t", false, true, false)),
        ("model_picker", KeyCombo::new("p", false, true, false)),
        ("temper_cycle", KeyCombo::new("m", false, true, false)),
    ]
}

impl KeybindsConfig {
    /// The built-in binds of `preset`: what `r`/`R` reset to in the configurator.
    pub fn defaults_for(preset: KeybindPreset) -> Self {
        let mut keybinds = Self {
            preset,
            ..Self::default()
        };
        keybinds.apply_preset();
        keybinds
    }

    /// Move every action still on its Forge default to the preset's chord. Config files deep-merge
    /// over the Forge defaults, so a bind equal to the Forge default is one the user never set;
    /// anything else is the user's choice and is left alone.
    pub fn apply_preset(&mut self) {
        if self.preset == KeybindPreset::Forge {
            return;
        }
        let forge = Self::default();
        for (action, combo) in claude_code_moves() {
            if self.binds.get(action) == forge.binds.get(action) {
                self.binds.insert(action.to_string(), combo);
            }
        }
    }
}

/// Resolve the one default-key collision introduced by the command center. Config files deep-merge
/// over defaults, so an older persisted default can retain `skip_model = Ctrl+K` alongside the new
/// command center binding. Move only that legacy default; any other user-selected binding remains
/// untouched and can still be adjusted in the keybind editor.
pub(crate) fn migrate_legacy_keybinds(keybinds: &mut KeybindsConfig) {
    let ctrl_k = KeyCombo::new("k", true, false, false);
    if keybinds.binds.get("command_center") == Some(&ctrl_k)
        && keybinds.binds.get("skip_model") == Some(&ctrl_k)
    {
        keybinds.binds.insert(
            "skip_model".to_string(),
            KeyCombo {
                shift: true,
                ..ctrl_k
            },
        );
    }
    crate::keybind_migration::move_compact_off_ctrl_l(keybinds);
}

/// Normalize a freshly loaded keymap: legacy migration first, then the preset.
pub(crate) fn normalize(keybinds: &mut KeybindsConfig) {
    migrate_legacy_keybinds(keybinds);
    keybinds.apply_preset();
}

fn user_config_doc() -> Result<(PathBuf, toml_edit::DocumentMut), ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    let doc = toml_write::load_doc(&path)?;
    Ok((path, doc))
}

/// Persist a single keybind to `[keybinds.binds]` in the user config TOML, preserving all other keys.
pub fn write_keybind(action: &str, combo: &KeyCombo) -> Result<PathBuf, ConfigError> {
    let (path, mut doc) = user_config_doc()?;
    let binds = toml_write::table_at(&mut doc, &["keybinds", "binds"]);
    let mut combo_t = toml_edit::InlineTable::new();
    combo_t.insert("key", combo.key.as_str().into());
    combo_t.insert("ctrl", combo.ctrl.into());
    combo_t.insert("alt", combo.alt.into());
    combo_t.insert("shift", combo.shift.into());
    toml_write::put(binds, action, toml_edit::Item::Value(combo_t.into()));
    toml_write::save_doc(&path, &doc)?;
    Ok(path)
}

/// Persist `[keybinds] preset` in the user config TOML, preserving all other keys.
pub fn write_keybind_preset(preset: KeybindPreset) -> Result<PathBuf, ConfigError> {
    let (path, mut doc) = user_config_doc()?;
    let table = toml_write::table_at(&mut doc, &["keybinds"]);
    toml_write::put(table, "preset", toml_edit::value(preset.as_str()));
    toml_write::save_doc(&path, &doc)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use figment::providers::{Format, Serialized, Toml};
    use figment::Figment;

    fn combo(key: &str, ctrl: bool, alt: bool, shift: bool) -> KeyCombo {
        KeyCombo::new(key, ctrl, alt, shift)
    }

    fn load(toml: &str) -> KeybindsConfig {
        let mut cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml))
            .extract()
            .unwrap();
        normalize(&mut cfg.keybinds);
        cfg.keybinds
    }

    #[test]
    fn keybinds_default_has_all_actions() {
        let kb = KeybindsConfig::default();
        assert_eq!(kb.binds.len(), 27);
        assert_eq!(kb.preset, KeybindPreset::Forge);
        assert_eq!(kb.binds["interrupt"], combo("c", true, false, false));
        assert_eq!(kb.binds["copy_last"], combo("c", true, false, true));
        assert_eq!(kb.binds["tier_up"], combo("up", true, false, false));
        assert_eq!(kb.binds["help"], combo("f1", false, false, false));
        assert_eq!(kb.binds["command_center"], combo("k", true, false, false));
    }

    /// A partial `[keybinds.binds]` override in the user TOML must DEEP-MERGE over the serialized
    /// defaults (keeping the other 22 binds), not replace the whole map. `write_keybind` writes a
    /// single bind, so a replace would unbind everything else — this guards that regression.
    #[test]
    fn partial_keybind_override_deep_merges_over_defaults() {
        let kb = load("[keybinds.binds.toggle_reasoning]\nkey = \"x\"\nctrl = true\n");
        assert_eq!(kb.binds["toggle_reasoning"].key, "x");
        assert_eq!(kb.binds.len(), 27);
        assert_eq!(kb.binds["interrupt"].key, "c");
        assert_eq!(kb.binds["help"].key, "f1");
    }

    #[test]
    fn legacy_ctrl_k_skip_binding_moves_to_shifted_chord() {
        let mut keybinds = KeybindsConfig::default();
        keybinds
            .binds
            .insert("skip_model".into(), combo("k", true, false, false));
        migrate_legacy_keybinds(&mut keybinds);
        assert_eq!(keybinds.binds["skip_model"], combo("k", true, false, true));
    }

    #[test]
    fn the_claude_code_preset_frees_every_cc_chord() {
        let kb = load("[keybinds]\npreset = \"claude-code\"\n");
        assert_eq!(kb.preset, KeybindPreset::ClaudeCode);
        let taken: Vec<&KeyCombo> = kb.binds.values().collect();
        for (key, ctrl, alt) in [
            ("k", true, false),
            ("n", true, false),
            ("z", true, false),
            ("s", true, false),
            ("l", true, false),
            ("r", true, false),
            ("p", false, true),
            ("t", false, true),
        ] {
            let cc = combo(key, ctrl, alt, false);
            let owner = kb.binds.iter().find(|(_, c)| **c == cc).map(|(a, _)| a);
            let expected_owner = match (key, alt) {
                ("l", false) => Some("redraw"),
                ("p", true) => Some("model_picker"),
                ("t", true) => Some("toggle_reasoning"),
                _ => None,
            };
            assert_eq!(owner.map(String::as_str), expected_owner, "{cc:?}");
        }
        assert_eq!(kb.binds["command_center"], combo("f2", false, false, false));
        assert_eq!(kb.binds["new_session"], combo("n", false, true, false));
        assert_eq!(kb.binds["temper_cycle"], combo("m", false, true, false));
        assert_eq!(
            taken.len(),
            27,
            "the preset moves actions, it never drops one"
        );
        let mut unique = taken.clone();
        unique.sort_by_key(|c| (c.key.clone(), c.ctrl, c.alt, c.shift));
        unique.dedup();
        assert_eq!(unique.len(), 27, "no two actions share a chord: {kb:?}");
    }

    #[test]
    fn an_explicit_bind_wins_over_the_preset() {
        let kb = load(
            "[keybinds]\npreset = \"claude-code\"\n[keybinds.binds.undo]\nkey = \"u\"\nctrl = false\nalt = true\nshift = false\n",
        );
        assert_eq!(kb.binds["undo"], combo("u", false, true, false));
        assert_eq!(kb.binds["checkpoint"], combo("s", false, true, false));
    }

    #[test]
    fn the_forge_preset_changes_nothing_and_defaults_for_matches_load() {
        assert_eq!(load("").binds, KeybindsConfig::default().binds);
        assert_eq!(
            KeybindsConfig::defaults_for(KeybindPreset::ClaudeCode).binds,
            load("[keybinds]\npreset = \"claude-code\"\n").binds
        );
    }

    #[test]
    fn an_unknown_preset_is_a_config_error() {
        let res: Result<Config, _> = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("[keybinds]\npreset = \"emacs\"\n"))
            .extract();
        assert!(res.is_err());
    }
}
