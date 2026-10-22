//! `!command` from the composer (Claude Code's shell mode): run it through the session's shell
//! tool as a turn-shaped background task, so the spinner, Esc-to-stop and the done signal behave
//! exactly as for a prompt. See `forge_core::Session::run_user_shell`.

use super::*;

/// What the composer should do with a submitted line that may be a `!command`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BangRoute<'a> {
    /// Not a shell line: the usual prompt / command handling applies.
    NotShell,
    /// A bare `!` — nothing to run.
    Empty,
    /// Run this command now.
    Run(&'a str),
    /// A turn is running: shell commands wait until it ends (the line goes back in the composer).
    Busy,
}

pub(crate) fn route_bang(line: &str, busy: bool) -> BangRoute<'_> {
    match forge_tui::bang_command(line) {
        None => BangRoute::NotShell,
        Some("") => BangRoute::Empty,
        Some(_) if busy => BangRoute::Busy,
        Some(cmd) => BangRoute::Run(cmd),
    }
}

pub(crate) const BANG_EMPTY_NOTE: &str =
    "type a command after ! to run it in the shell, e.g. !git status";
pub(crate) const BANG_BUSY_NOTE: &str =
    "⏳ ! commands run between turns — press Enter again once this turn finishes (Esc stops it)";

/// Echo `! command`, run it, then either let the model respond to the output (`respond`) or just
/// add the run to the context. A floor refusal never reaches the model.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_bang(
    command: &str,
    respond: bool,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.on_turn_start();
    app.submit_user(&format!("! {command}"));
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    let command = command.to_string();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        let run = match sess.run_user_shell(&command).await {
            Ok(run) => run,
            Err(e) => {
                sess.notify_error(&format!("! {command} failed: {e}"));
                return;
            }
        };
        if run.refused {
            return;
        }
        let outcome = if respond {
            sess.run_turn(&run.context_message()).await.map(|_| ())
        } else {
            sess.record_user_shell(&run)
        };
        if let Err(e) = outcome {
            sess.notify_error(&format!("! {command}: {e}"));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_lines_route_by_state() {
        assert_eq!(route_bang("!ls", false), BangRoute::Run("ls"));
        assert_eq!(route_bang("!ls", true), BangRoute::Busy);
        assert_eq!(route_bang("!", false), BangRoute::Empty);
        assert_eq!(route_bang("!  ", true), BangRoute::Empty);
        assert_eq!(route_bang("ls", false), BangRoute::NotShell);
        assert_eq!(route_bang(" !ls", false), BangRoute::NotShell);
    }
}
