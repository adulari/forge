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
#[serde(from = "KeyComboRepr")]
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

/// What a bind may be written as in TOML: the `{ key, ctrl, alt, shift }` table, or a string such
/// as `"ctrl+shift+k"`, `"f2"` or `"none"` (the action is unbound).
#[derive(Deserialize)]
#[serde(untagged)]
enum KeyComboRepr {
    Text(String),
    Table {
        key: String,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        shift: bool,
    },
}

impl From<KeyComboRepr> for KeyCombo {
    fn from(repr: KeyComboRepr) -> Self {
        match repr {
            KeyComboRepr::Text(text) => KeyCombo::parse(&text),
            KeyComboRepr::Table {
                key,
                ctrl,
                alt,
                shift,
            } => KeyCombo {
                key,
                ctrl,
                alt,
                shift,
            },
        }
    }
}

/// The key name of a bind that is switched off.
pub const UNBOUND_KEY: &str = "none";

impl KeyCombo {
    /// A combo that matches no key: written `"none"`, it removes a default bind.
    pub fn unbound() -> Self {
        Self::new(UNBOUND_KEY, false, false, false)
    }

    pub fn is_unbound(&self) -> bool {
        self.key.eq_ignore_ascii_case(UNBOUND_KEY)
    }

    /// Parse `"ctrl+alt+k"`, `"shift+tab"`, `"f2"` or `"none"`. Modifier words may come in any
    /// order and case; the last `+`-separated part is the key (so `"ctrl++"` is Ctrl and `+`).
    /// Text that names no key yields a combo with that text as its key, which the validity check
    /// then reports instead of silently dropping the bind.
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        let mut rest = text;
        loop {
            let lower = rest.to_ascii_lowercase();
            let (flag, len) = if lower.starts_with("ctrl+") || lower.starts_with("ctl+") {
                (&mut ctrl, lower.find('+').unwrap_or(0) + 1)
            } else if lower.starts_with("alt+") {
                (&mut alt, 4)
            } else if lower.starts_with("shift+") {
                (&mut shift, 6)
            } else {
                break;
            };
            if rest.len() <= len {
                break;
            }
            *flag = true;
            rest = &rest[len..];
        }
        let key = rest.to_ascii_lowercase();
        let key = if key == "space" { " ".to_string() } else { key };
        Self::new(&key, ctrl, alt, shift)
    }

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
    /// Forge's own defaults (Ctrl+K command center, Ctrl+N new session, Ctrl+Z suspend…).
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindsConfig {
    /// The built-in keymap the binds start from (`"forge"` or `"claude-code"`).
    #[serde(default)]
    pub preset: KeybindPreset,
    #[serde(default)]
    pub binds: BTreeMap<String, KeyCombo>,
    /// The leader key: press it, then a follow-up key (a which-key strip lists them). Off by
    /// default. `leader = true` picks Ctrl+X; a chord such as `"ctrl+space"`-style text or a
    /// `{ key, ctrl, alt, shift }` table picks another.
    #[serde(default, deserialize_with = "deserialize_leader")]
    pub leader: Option<KeyCombo>,
    /// How long a half-typed leader or sequence waits for its next key before it is dropped.
    #[serde(default = "default_leader_timeout_ms")]
    pub leader_timeout_ms: u64,
    /// Key sequences: action name -> steps separated by spaces, e.g. `new_session = "<leader> n"`
    /// or `undo = "ctrl+x u"`. `<leader>` stands for the leader key. The first step must carry
    /// Ctrl or Alt (or be a function key) so ordinary typing is never swallowed. An action set
    /// to `"none"` drops its built-in leader follow-up.
    #[serde(default)]
    pub sequences: BTreeMap<String, String>,
}

fn default_leader_timeout_ms() -> u64 {
    2000
}

/// The leader chord that `leader = true` selects.
pub fn default_leader() -> KeyCombo {
    KeyCombo::new("x", true, false, false)
}

fn deserialize_leader<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<KeyCombo>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Leader {
        Flag(bool),
        Combo(KeyCombo),
    }
    Ok(match Option::<Leader>::deserialize(deserializer)? {
        None | Some(Leader::Flag(false)) => None,
        Some(Leader::Flag(true)) => Some(default_leader()),
        Some(Leader::Combo(combo)) if combo.is_unbound() => None,
        Some(Leader::Combo(combo)) => Some(combo),
    })
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
        // Ctrl+Z is the terminal's suspend chord; undoing a write is an Alt chord so a reflexive
        // Ctrl+Z can never revert a file.
        binds.insert("undo".into(), bind("z", false, true, false));
        binds.insert("suspend".into(), bind("z", true, false, false));
        binds.insert("verbosity_cycle".into(), bind("v", false, true, false));
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
        binds.insert("stash".into(), bind("x", true, false, false));
        Self {
            preset: KeybindPreset::Forge,
            binds,
            leader: None,
            leader_timeout_ms: default_leader_timeout_ms(),
            sequences: BTreeMap::new(),
        }
    }
}

/// The actions the `claude-code` preset moves, and where to. Every other action keeps its Forge
/// default. The freed chords fall through to the composer's readline keys (Ctrl+K kill to end of
/// line, Ctrl+N history) or are ignored (Ctrl+Z, Ctrl+S, Ctrl+R), never typed as a letter.
fn claude_code_moves() -> [(&'static str, KeyCombo); 6] {
    [
        ("command_center", KeyCombo::new("f2", false, false, false)),
        ("new_session", KeyCombo::new("n", false, true, false)),
        ("checkpoint", KeyCombo::new("s", false, true, false)),
        ("toggle_reasoning", KeyCombo::new("t", false, true, false)),
        ("model_picker", KeyCombo::new("p", false, true, false)),
        ("temper_cycle", KeyCombo::new("m", false, true, false)),
    ]
}

/// What each key does after the leader, unless `[keybinds.sequences]` rebinds the action.
const LEADER_FOLLOW_UPS: [(&str, &str); 16] = [
    ("n", "new_session"),
    ("m", "model_picker"),
    ("e", "effort_cycle"),
    ("t", "temper_cycle"),
    ("c", "compact"),
    ("k", "command_center"),
    ("h", "help"),
    ("s", "checkpoint"),
    ("u", "undo"),
    ("y", "copy_last"),
    ("r", "reload"),
    ("x", "stash"),
    ("z", "suspend"),
    ("v", "verbosity_cycle"),
    (",", "keybind_config"),
    ("l", "redraw"),
];

/// An action reached by pressing `steps` one after another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySequence {
    pub action: String,
    pub steps: Vec<KeyCombo>,
}

impl KeybindsConfig {
    /// Parse one sequence string (`"<leader> n"`, `"ctrl+x ctrl+k"`). `None` when it has fewer
    /// than two steps, uses `<leader>` with no leader set, or starts with a plain key that
    /// ordinary typing would trigger.
    fn parse_sequence(&self, text: &str) -> Option<Vec<KeyCombo>> {
        let steps: Vec<KeyCombo> = text
            .split_whitespace()
            .map(|token| {
                if token.eq_ignore_ascii_case("<leader>") {
                    self.leader.clone()
                } else {
                    Some(KeyCombo::parse(token))
                }
            })
            .collect::<Option<_>>()?;
        let first = steps.first()?;
        (steps.len() >= 2 && Self::safe_first_step(first)).then_some(steps)
    }

    /// Whether `combo` can start a sequence without stealing ordinary typing or editing: a
    /// function key, or a Ctrl/Alt chord that the composer does not already use. Esc, Enter,
    /// Space, arrows and bare letters are never allowed.
    pub fn safe_first_step(combo: &KeyCombo) -> bool {
        const CTRL_EDITING: &str = "acdejuwy";
        const ALT_EDITING: &str = "bfy";
        if combo.is_unbound() {
            return false;
        }
        let is_function = combo.key.len() > 1
            && combo.key.starts_with('f')
            && combo.key[1..].chars().all(|c| c.is_ascii_digit());
        if is_function {
            return true;
        }
        let single = combo.key.chars().count() == 1;
        let letter = combo.key.chars().next().unwrap_or(' ');
        (combo.ctrl || combo.alt)
            && !(single && combo.ctrl && !combo.alt && CTRL_EDITING.contains(letter))
            && !(single && combo.alt && !combo.ctrl && ALT_EDITING.contains(letter))
            && combo.key != "esc"
            && combo.key != "enter"
            && combo.key != "backspace"
    }

    /// Every working key sequence: the leader's built-in follow-ups (while a leader is set) plus
    /// the user's `[keybinds.sequences]`. A user sequence for an action replaces that action's
    /// built-in follow-up; `"none"` removes it.
    pub fn effective_sequences(&self) -> Vec<KeySequence> {
        let mut out = Vec::new();
        if let Some(leader) = self.leader.as_ref().filter(|l| Self::safe_first_step(l)) {
            for (key, action) in LEADER_FOLLOW_UPS {
                if !self.sequences.contains_key(action) {
                    out.push(KeySequence {
                        action: action.to_string(),
                        steps: vec![leader.clone(), KeyCombo::parse(key)],
                    });
                }
            }
        }
        for (action, text) in &self.sequences {
            if let Some(steps) = self.parse_sequence(text) {
                out.push(KeySequence {
                    action: action.clone(),
                    steps,
                });
            }
        }
        out
    }

    /// `(action, text)` of sequences that can never fire, for a startup note.
    pub fn invalid_sequences(&self) -> Vec<(String, String)> {
        let bad_leader = self
            .leader
            .as_ref()
            .filter(|l| !Self::safe_first_step(l))
            .map(|l| ("leader".to_string(), format!("{l:?}")));
        bad_leader
            .into_iter()
            .chain(
                self.sequences
                    .iter()
                    .filter(|(_, text)| {
                        !text.trim().eq_ignore_ascii_case(UNBOUND_KEY)
                            && self.parse_sequence(text).is_none()
                    })
                    .map(|(a, t)| (a.clone(), t.clone())),
            )
            .collect()
    }

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
    crate::keybind_migration::move_undo_off_ctrl_z(keybinds);
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

/// Drop a saved bind so the action falls back to its default (or preset) chord again. A no-op
/// when nothing is saved for it.
pub fn remove_keybind(action: &str) -> Result<(), ConfigError> {
    let (path, mut doc) = user_config_doc()?;
    if remove_bind_from(&mut doc, action) {
        toml_write::save_doc(&path, &doc)?;
    }
    Ok(())
}

fn remove_bind_from(doc: &mut toml_edit::DocumentMut, action: &str) -> bool {
    let saved = doc
        .get("keybinds")
        .and_then(|k| k.get("binds"))
        .is_some_and(|b| b.get(action).is_some());
    if saved {
        toml_write::remove_dotted(doc, &format!("keybinds.binds.{action}"));
    }
    saved
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
        assert_eq!(kb.binds.len(), 30);
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
        assert_eq!(kb.binds.len(), 30);
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
                ("z", false) => Some("suspend"),
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
            30,
            "the preset moves actions, it never drops one"
        );
        let mut unique = taken.clone();
        unique.sort_by_key(|c| (c.key.clone(), c.ctrl, c.alt, c.shift));
        unique.dedup();
        assert_eq!(unique.len(), 30, "no two actions share a chord: {kb:?}");
    }

    #[test]
    fn no_two_default_actions_share_a_chord() {
        let kb = KeybindsConfig::default();
        let mut chords: Vec<_> = kb.binds.values().cloned().collect();
        chords.sort_by_key(|c| (c.key.clone(), c.ctrl, c.alt, c.shift));
        let total = chords.len();
        chords.dedup();
        assert_eq!(chords.len(), total);
    }

    #[test]
    fn a_bind_can_be_text_and_none_unbinds_it() {
        let kb =
            load("[keybinds.binds]\nundo = \"none\"\nhelp = \"ctrl+Shift+h\"\nredraw = \"f5\"\n");
        assert!(kb.binds["undo"].is_unbound());
        assert_eq!(kb.binds["help"], combo("h", true, false, true));
        assert_eq!(kb.binds["redraw"], combo("f5", false, false, false));
        assert_eq!(kb.binds["interrupt"].key, "c");
    }

    #[test]
    fn the_leader_is_off_until_set_and_true_means_ctrl_x() {
        let off = load("");
        assert!(off.leader.is_none());
        assert!(off.effective_sequences().is_empty());
        assert_eq!(off.leader_timeout_ms, 2000);
        let on = load("[keybinds]\nleader = true\n");
        assert_eq!(on.leader, Some(combo("x", true, false, false)));
        let custom = load("[keybinds]\nleader = \"alt+space\"\nleader_timeout_ms = 500\n");
        assert_eq!(custom.leader, Some(combo(" ", false, true, false)));
        assert_eq!(custom.leader_timeout_ms, 500);
        assert!(load("[keybinds]\nleader = false\n").leader.is_none());
    }

    #[test]
    fn leader_follow_ups_and_user_sequences_combine() {
        let kb = load(
            "[keybinds]\nleader = true\n[keybinds.sequences]\nnew_session = \"<leader> o\"\nundo = \"none\"\nhelp = \"ctrl+g h\"\nstash = \"q q\"\nreload = \"<leader>\"\n",
        );
        let seqs = kb.effective_sequences();
        let find = |action: &str| {
            seqs.iter()
                .find(|s| s.action == action)
                .map(|s| s.steps.clone())
        };
        let leader = combo("x", true, false, false);
        assert_eq!(
            find("new_session"),
            Some(vec![leader.clone(), combo("o", false, false, false)])
        );
        assert_eq!(
            find("model_picker"),
            Some(vec![leader.clone(), combo("m", false, false, false)])
        );
        assert_eq!(find("undo"), None, "none drops the built-in follow-up");
        assert_eq!(
            find("help"),
            Some(vec![
                combo("g", true, false, false),
                combo("h", false, false, false)
            ])
        );
        assert_eq!(
            find("stash"),
            None,
            "a plain first key would swallow typing"
        );
        assert_eq!(find("reload"), None, "one step is not a sequence");
        let bad: Vec<_> = kb.invalid_sequences().into_iter().map(|(a, _)| a).collect();
        assert_eq!(bad, vec!["reload".to_string(), "stash".to_string()]);
    }

    #[test]
    fn a_leader_or_first_step_that_would_eat_typing_is_refused() {
        for leader in ["space", "esc", "enter", "a", "ctrl+a", "ctrl+d", "alt+b"] {
            let kb = load(&format!("[keybinds]\nleader = \"{leader}\"\n"));
            assert!(kb.effective_sequences().is_empty(), "{leader}");
            assert_eq!(kb.invalid_sequences().len(), 1, "{leader}");
        }
        let kb =
            load("[keybinds.sequences]\nhelp = \"esc h\"\nundo = \"enter x\"\nreload = \"f5 r\"\n");
        let ok: Vec<_> = kb
            .effective_sequences()
            .into_iter()
            .map(|s| s.action)
            .collect();
        assert_eq!(ok, vec!["reload".to_string()]);
        assert_eq!(kb.invalid_sequences().len(), 2);
    }

    #[test]
    fn remove_keybind_deletes_only_the_saved_entry() {
        let mut doc: toml_edit::DocumentMut =
            "[keybinds.binds]\nsuspend = { key = \"none\" }\nhelp = { key = \"h\", ctrl = true }\n"
                .parse()
                .unwrap();
        assert!(remove_bind_from(&mut doc, "suspend"));
        assert!(!remove_bind_from(&mut doc, "never_saved"));
        let text = doc.to_string();
        assert!(!text.contains("suspend"), "{text}");
        assert!(text.contains("help"), "{text}");
    }

    #[test]
    fn leader_sequences_need_a_leader() {
        let kb = load("[keybinds.sequences]\nnew_session = \"<leader> o\"\n");
        assert!(kb.effective_sequences().is_empty());
        assert_eq!(kb.invalid_sequences().len(), 1);
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
