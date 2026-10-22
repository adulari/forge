//! Keybind defaults that moved, carried forward for configs that persisted the old value.

use crate::{KeyCombo, KeybindsConfig};

/// `compact` used to own Ctrl+L, which every shell (and Claude Code) uses to redraw the screen, so
/// a habitual press spent a model call. Ctrl+L now redraws and compaction moved to Alt+L. A config
/// that still says `compact = Ctrl+L` while `redraw` is also on Ctrl+L (the configurator writes
/// every bind, so the old default may be on disk) would make the two collide; the compact bind
/// moves. A user who rebound `redraw` elsewhere keeps Ctrl+L for compaction.
pub(crate) fn move_compact_off_ctrl_l(keybinds: &mut KeybindsConfig) {
    let ctrl_l = KeyCombo {
        key: "l".to_string(),
        ctrl: true,
        alt: false,
        shift: false,
    };
    if keybinds.binds.get("compact") == Some(&ctrl_l)
        && keybinds.binds.get("redraw") == Some(&ctrl_l)
    {
        keybinds.binds.insert(
            "compact".to_string(),
            KeyCombo {
                ctrl: false,
                alt: true,
                ..ctrl_l
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(ctrl: bool, alt: bool) -> KeyCombo {
        KeyCombo {
            key: "l".into(),
            ctrl,
            alt,
            shift: false,
        }
    }

    #[test]
    fn ctrl_l_redraws_and_compaction_is_alt_l_by_default() {
        let defaults = KeybindsConfig::default();
        assert_eq!(defaults.binds["redraw"], combo(true, false));
        assert_eq!(defaults.binds["compact"], combo(false, true));
    }

    #[test]
    fn a_persisted_ctrl_l_compact_moves_off_the_redraw_key() {
        let mut keybinds = KeybindsConfig::default();
        keybinds.binds.insert("compact".into(), combo(true, false));
        move_compact_off_ctrl_l(&mut keybinds);
        assert_eq!(keybinds.binds["compact"], combo(false, true));
        assert_eq!(keybinds.binds["redraw"], combo(true, false));
    }

    #[test]
    fn a_user_who_moved_redraw_keeps_ctrl_l_for_compaction() {
        let mut keybinds = KeybindsConfig::default();
        keybinds.binds.insert("compact".into(), combo(true, false));
        keybinds.binds.insert(
            "redraw".into(),
            KeyCombo {
                key: "f5".into(),
                ctrl: false,
                alt: false,
                shift: false,
            },
        );
        move_compact_off_ctrl_l(&mut keybinds);
        assert_eq!(keybinds.binds["compact"], combo(true, false));
    }
}
