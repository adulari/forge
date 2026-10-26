//! Forge onboarding, terminal host. The engine (`forge-onboarding`) decides and acts, the screen
//! (`forge_tui::onboarding`) draws; this module opens it at the right moments:
//!
//! * `forge setup` / `forge init`: always, the full flow ([`setup`]).
//! * bare `forge`, `forge chat`, `forge run`, `forge nl` on a terminal: when the state file says
//!   it has not run (or a required step is missing) ([`maybe_open`]).

mod host;
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
            let outcome = runner::run(*session, false)?;
            print_closing(&outcome, true);
        }
        Start::Quiet | Start::Hint(_) => {}
    }
    Ok(())
}

/// The automatic trigger, for every entry point that is about to start a session. A no-op off a
/// terminal, under `--mock`, with `FORGE_NO_SETUP`, once it ran in this process, and whenever
/// the state file says everything is current.
pub(crate) fn maybe_open(mock: bool) -> Result<()> {
    if mock || AUTO_ATTEMPTED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let host: Arc<dyn Host> = Arc::new(CliHost);
    match start(host, Surface::Tui, false) {
        Start::Run(session) => {
            let outcome = runner::run(*session, true)?;
            print_closing(&outcome, false);
        }
        Start::Hint(text) => eprintln!("{text}"),
        Start::Quiet => {}
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
