//! What bare `forge` (no subcommand) does.
//!
//! On an interactive terminal with a usable provider it starts `forge chat`, the way bare
//! `claude` starts a session. Everywhere else it prints a screen instead of clap's default (the
//! full ~40-command help wall on stderr with exit code 2, which never says the install has no
//! provider yet): a short first-run panel when nothing is configured, a common-commands summary
//! when something is (a pipe, CI, `TERM=dumb`). `forge --help` / `forge help` keep the full listing.

use std::io::IsTerminal;

use crate::Command;

/// Is there any provider this binary could route a turn to? Reuses doctor's credential-side
/// check (keys, CLI bridges, a local Ollama model) — local evidence only, no network probe.
pub(crate) fn provider_configured() -> bool {
    crate::doctor::provider_credentials_present()
}

/// Bare `forge` opens a chat only when someone is at the keyboard (both ends a terminal that can
/// draw the TUI) and a turn could actually run; otherwise the screen explains what to do.
pub(crate) fn bare_starts_chat(terminal: bool, dumb_term: bool, configured: bool) -> bool {
    terminal && !dumb_term && configured
}

/// The command bare `forge` runs, or `None` after printing the welcome screen.
///
/// On a terminal, onboarding opens first when the state file says it has not run or a required
/// step is missing; the answer to "is a provider configured" is read after it, so a person who
/// has just set one up goes straight into a chat.
pub(crate) fn bare_command() -> Option<Command> {
    let stdout_tty = std::io::stdout().is_terminal();
    let terminal = stdout_tty && std::io::stdin().is_terminal();
    let dumb_term = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    if terminal && !dumb_term {
        // The theme normally resolves after the command is chosen; onboarding draws before that.
        if let Ok(config) = forge_config::load() {
            forge_tui::theme::init(&config.tui);
        }
        match crate::onboarding::maybe_open(false, false) {
            // Onboarding just ran and said where things stand; the static panel would repeat it.
            Ok(true) if !provider_configured() => return None,
            Ok(_) => {}
            Err(error) => eprintln!("setup could not open: {error:#}"),
        }
    }
    let configured = provider_configured();
    if bare_starts_chat(terminal, dumb_term, configured) {
        return Some(Command::Chat {
            prompt: Vec::new(),
            prompt_flag: None,
            mock: false,
            mode: None,
            r#continue: false,
            all_dirs: false,
            resume: None,
            plain: false,
            tui: false,
            inline: false,
            fullscreen: false,
            model: None,
        });
    }
    print!("{}", welcome_text(configured, stdout_tty));
    None
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
    out.push_str("  forge chat [\"<prompt>\"]   interactive multi-turn session\n");
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
    fn bare_forge_starts_chat_only_on_a_configured_interactive_terminal() {
        assert!(bare_starts_chat(true, false, true));
        assert!(!bare_starts_chat(false, false, true), "pipe / CI");
        assert!(!bare_starts_chat(true, true, true), "TERM=dumb");
        assert!(
            !bare_starts_chat(true, false, false),
            "no provider: the setup panel"
        );
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
