//! What bare `forge` (no subcommand) prints.
//!
//! Clap's default for a missing subcommand is the full ~40-command help wall on stderr with exit
//! code 2 — the least useful screen for the most likely first keystroke, and it never says the
//! install has no provider yet. Instead: a short first-run panel when nothing is configured, a
//! common-commands summary when something is. `forge --help` / `forge help` keep the full listing.

/// Is there any provider this binary could route a turn to? Reuses doctor's credential-side
/// check (keys, CLI bridges, a local Ollama model) — local evidence only, no network probe.
pub(crate) fn provider_configured() -> bool {
    crate::doctor::provider_credentials_present()
}

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// The bare-`forge` screen. Pure: `configured` picks the panel, `color` picks whether ANSI
/// attributes are emitted (a pipe/CI gets the identical text without them).
pub(crate) fn welcome_text(configured: bool, color: bool) -> String {
    let (bold, dim, reset) = if color {
        (BOLD, DIM, RESET)
    } else {
        ("", "", "")
    };
    let mut out = format!("{bold}⚒ forge{reset} {dim}v{}{reset}\n\n", version());
    if !configured {
        out.push_str("No model provider is configured yet — Forge can't run a turn.\n\n");
        out.push_str(&format!("{bold}Set one up{reset}\n"));
        out.push_str(
            "  forge setup               guided: keys, CLI-bridge subscriptions, local models\n",
        );
        out.push_str(
            "  forge auth <provider>     direct: store one API key (anthropic, openai, groq, …)\n\n",
        );
        out.push_str(&format!("{bold}Then{reset}\n"));
    } else {
        out.push_str(&format!("{bold}Common commands{reset}\n"));
    }
    out.push_str("  forge run \"<prompt>\"      one agent turn, headless\n");
    out.push_str("  forge chat                interactive multi-turn session\n");
    out.push_str("  forge doctor              check config, providers, store\n\n");
    out.push_str(&format!("{dim}run `forge --help` for everything{reset}\n"));
    out
}

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyless_panel_names_setup_and_auth() {
        let text = welcome_text(false, false);
        assert!(text.contains("No model provider is configured yet"));
        assert!(text.contains("forge setup"));
        assert!(text.contains("forge auth <provider>"));
        // The three next commands are on both panels.
        assert!(
            text.contains("forge run")
                && text.contains("forge chat")
                && text.contains("forge doctor")
        );
    }

    #[test]
    fn the_keyed_panel_is_a_command_summary_not_a_setup_prompt() {
        let text = welcome_text(true, false);
        assert!(text.contains("Common commands"));
        assert!(!text.contains("No model provider"));
        assert!(!text.contains("forge setup"));
        assert!(text.contains("run `forge --help` for everything"));
    }

    #[test]
    fn both_panels_stay_short_enough_to_read() {
        for configured in [true, false] {
            assert!(welcome_text(configured, false).lines().count() <= 15);
        }
    }

    #[test]
    fn color_only_adds_attributes_never_changes_the_text() {
        for configured in [true, false] {
            let plain = welcome_text(configured, false);
            let colored = welcome_text(configured, true);
            assert!(!plain.contains('\x1b'));
            assert!(colored.contains(BOLD));
            assert_eq!(
                colored
                    .replace(BOLD, "")
                    .replace(DIM, "")
                    .replace(RESET, ""),
                plain
            );
        }
    }
}
