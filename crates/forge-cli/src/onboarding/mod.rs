//! Forge onboarding, terminal host. The engine (`forge-onboarding`) decides and acts, the screen
//! (`forge_tui::onboarding`) draws; this module opens it at the right moments:
//!
//! * `forge setup` / `forge init`: always, the full flow ([`setup`]).
//! * bare `forge`, `forge chat`, `forge run`, `forge nl` on a terminal: when the state file says
//!   it has not run (or a required step is missing) ([`maybe_open`]).

pub(crate) mod host;
mod runner;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use forge_onboarding::{start, Host, Start, Surface};
use forge_tui::onboarding::Exit;

use host::CliHost;

/// Onboarding opens by itself at most once per process: bare `forge` opens it, then starts a
/// chat, and the chat's own check must not open it a second time.
static AUTO_ATTEMPTED: AtomicBool = AtomicBool::new(false);

/// `forge setup`: the full flow, whatever the state file says.
pub(crate) fn setup(invoked_as: &str) -> Result<()> {
    if !stdio_is_terminal() {
        anyhow::bail!(crate::cli::commands::local::non_interactive_message(
            invoked_as
        ));
    }
    let host: Arc<dyn Host> = Arc::new(CliHost);
    match start(host, Surface::Tui, true) {
        Start::Run(session) => {
            let outcome = runner::run(*session, false, false)?;
            print_closing(&outcome, true);
        }
        Start::Quiet | Start::Hint(_) => {}
    }
    Ok(())
}

/// The automatic trigger, for every entry point that is about to start a session. A no-op off a
/// terminal, under `--mock`, with `FORGE_NO_SETUP`, once it ran in this process, and whenever
/// the state file says everything is current.
pub(crate) fn maybe_open(mock: bool) -> Result<bool> {
    if mock || AUTO_ATTEMPTED.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }
    let host: Arc<dyn Host> = Arc::new(CliHost);
    match start(host, Surface::Tui, false) {
        Start::Run(session) => {
            let outcome = runner::run(*session, true, false)?;
            print_closing(&outcome, false);
            return Ok(true);
        }
        Start::Hint(text) => {
            eprintln!("{text}");
            reconnect_card()?;
        }
        Start::Quiet => reconnect_card()?,
    }
    Ok(false)
}

/// The notice marker that records "don't ask again" for the session-start card.
const CARD_DISMISSED: &str = "claude-reconnect-card-dismissed";

/// Whether the session-start "Connect Claude Code" card is due. Pure so the rule is testable:
/// a person is at the terminal, nothing opted out, they did not dismiss it, and Claude Code is
/// installed with its own login but no token for Forge.
pub(crate) fn card_due(interactive: bool, opted_out: bool, dismissed: bool, needed: bool) -> bool {
    interactive && !opted_out && !dismissed && needed
}

/// Existing users whose Claude bridge now needs a token get an actionable card the next time a
/// session starts, outside the once-per-release onboarding run: Enter runs the whole connection.
fn reconnect_card() -> Result<()> {
    let opted_out = std::env::var_os(forge_onboarding::OPT_OUT_ENV).is_some_and(|v| !v.is_empty());
    if !card_due(
        forge_onboarding::interactive_terminal(),
        opted_out,
        forge_config::install_notice_seen(CARD_DISMISSED),
        forge_onboarding::claude_reconnect_needed(),
    ) {
        return Ok(());
    }
    let host: Arc<dyn Host> = Arc::new(CliHost);
    let machine = forge_onboarding::detect(host);
    let state = forge_onboarding::load().unwrap_or_default();
    let def = forge_onboarding::StepId::Models.def();
    let session = forge_onboarding::Session {
        surface: Surface::Tui,
        mode: forge_onboarding::PlanMode::Resume,
        plan: vec![forge_onboarding::PlanStep {
            id: def.id,
            title: def.title.to_string(),
            summary: def.summary.to_string(),
            required: false,
            status: forge_onboarding::StepStatus::Pending,
            new: false,
            satisfied: false,
        }],
        state,
        machine,
    };
    let outcome = runner::run(session, true, true)?;
    if outcome.card_dismissed {
        forge_config::mark_install_notice_seen(CARD_DISMISSED);
    }
    Ok(())
}

fn stdio_is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// One line after the screen closes. A run that goes on to start a session says nothing when it
/// finished: the session itself is the answer.
fn print_closing(outcome: &runner::Outcome, announce_finish: bool) {
    match outcome.exit {
        Exit::Finished if !announce_finish => {}
        Exit::Finished => {
            let gate = outcome.session.gate();
            if gate.complete {
                println!("Forge is set up. Run `forge doctor` any time to check it.");
            } else {
                println!("Setup saved, but Forge cannot run a turn yet. Run `forge setup` to connect a model.");
            }
        }
        Exit::Left => {
            println!("Setup paused. Run `forge setup` any time to pick it up again.");
        }
    }
}

#[cfg(test)]
mod tests;
