//! The keybinding step of `forge setup`: offer the `claude-code` preset to people who come from
//! Claude Code, so CC's chords keep CC's meaning (docs/features/keybinds.md).

use super::prompt_line;

/// What the preset changes, in one line each, for the setup prompt.
pub(crate) const CLAUDE_CODE_PRESET_SUMMARY: &str = "  Claude Code keys keep their meaning: Ctrl+K cuts to the end of the line, Ctrl+N/Ctrl+P walk\n  history, and Ctrl+Z, Ctrl+S, Ctrl+L, Ctrl+R never undo a write, checkpoint, compact or hide\n  reasoning. Forge's actions move to F2 (actions), Alt+N (new session), Alt+Z (undo write),\n  Alt+S (checkpoint), Alt+L (compact), Alt+P (model), Alt+M (mode) and Alt+T (reasoning).";

/// Whether an answer to a `[Y/n]` / `[y/N]` prompt means yes.
pub(crate) fn answer_is_yes(answer: &str, default_yes: bool) -> bool {
    match answer.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    }
}

/// Ask once; skipped when the preset is already on. Defaults to yes when `~/.claude` exists.
pub(crate) fn offer_claude_code_keybinds() {
    let current = super::super::load_config()
        .map(|c| c.keybinds.preset)
        .unwrap_or_default();
    if current == forge_config::KeybindPreset::ClaudeCode {
        return;
    }
    let from_cc = forge_config::claude_dir().is_some_and(|d| d.is_dir());
    println!("\n⌨ Keybindings");
    println!("{CLAUDE_CODE_PRESET_SUMMARY}");
    let question = if from_cc {
        "  Claude Code is installed here. Use its keybindings? [Y/n]: "
    } else {
        "  Use the Claude Code keybindings? [y/N]: "
    };
    let Ok(answer) = prompt_line(question) else {
        return;
    };
    if !answer_is_yes(&answer, from_cc) {
        println!(
            "  Kept Forge's keys. Switch any time with `[keybinds] preset = \"claude-code\"`."
        );
        return;
    }
    match forge_config::write_keybind_preset(forge_config::KeybindPreset::ClaudeCode) {
        Ok(path) => println!("  ✓ Claude Code keybindings on ({}).", path.display()),
        Err(e) => println!("  ⚠ could not save the preset: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_answer_follows_the_detected_install() {
        assert!(answer_is_yes("", true));
        assert!(!answer_is_yes("", false));
        assert!(answer_is_yes(" Yes ", false));
        assert!(!answer_is_yes("n", true));
        assert!(!answer_is_yes("maybe", true));
    }
}
