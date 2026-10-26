//! Owns the terminal for an onboarding run: draws the screen, polls jobs, carries out the
//! screen's effects, and hands the real terminal to interactive children (a browser sign-in,
//! `claude setup-token`) before taking the screen back.

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind};
use forge_onboarding::{Action, Exec, Host, Jobs, Machine, Session};
use forge_tui::onboarding::{self, Effect, Exit, Purpose, Screen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::host::CliHost;

pub(crate) struct Outcome {
    pub(crate) exit: Exit,
    pub(crate) session: Session,
}

type Term = Terminal<CrosstermBackend<io::Stdout>>;

pub(crate) fn run(session: Session, launches_session: bool) -> Result<Outcome> {
    onboarding::terminal::install_panic_restore();
    onboarding::terminal::enter().context("opening the setup screen")?;
    let result = event_loop(session, launches_session);
    onboarding::terminal::leave();
    result
}

fn new_terminal() -> Result<Term> {
    Terminal::new(CrosstermBackend::new(io::stdout())).context("drawing the setup screen")
}

fn event_loop(session: Session, launches_session: bool) -> Result<Outcome> {
    let host: Arc<dyn Host> = Arc::new(CliHost);
    let jobs = Jobs::new();
    let mut terminal = new_terminal()?;
    let mut screen = Screen::new(session);
    screen.launches_session = launches_session;
    let mut detecting: Option<Receiver<Machine>> = None;
    let mut effects = screen.opening_effects();

    loop {
        while !effects.is_empty() {
            let batch = std::mem::take(&mut effects);
            for effect in batch {
                effects.extend(carry_out(
                    effect,
                    &mut screen,
                    &jobs,
                    &host,
                    &mut detecting,
                    &mut terminal,
                )?);
            }
        }
        if screen.exit.is_some() {
            break;
        }

        for (id, cursor) in screen.pollable() {
            if let Some(poll) = jobs.poll(id, cursor) {
                effects.extend(screen.apply_poll(&poll));
            }
        }
        if let Some(rx) = &detecting {
            if let Ok(machine) = rx.try_recv() {
                detecting = None;
                effects.extend(screen.machine_updated(machine));
            }
        }
        terminal.draw(|f| onboarding::render(f, &screen))?;

        if event::poll(Duration::from_millis(60))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    effects.extend(screen.key(key));
                }
                Event::Resize(..) => terminal.clear()?,
                _ => {}
            }
        } else {
            screen.tick();
        }
    }

    screen.session.save();
    if screen.exit == Some(Exit::Finished) && screen.ui.trust_folder {
        if let Ok(dir) = std::env::current_dir() {
            let _ = forge_config::project_trust::trust(&dir);
        }
    }
    let exit = screen.exit.unwrap_or(Exit::Left);
    Ok(Outcome {
        exit,
        session: screen.session,
    })
}

fn carry_out(
    effect: Effect,
    screen: &mut Screen,
    jobs: &Jobs,
    host: &Arc<dyn Host>,
    detecting: &mut Option<Receiver<Machine>>,
    terminal: &mut Term,
) -> Result<Vec<Effect>> {
    Ok(match effect {
        Effect::Run { action, purpose } => {
            let label = action.label();
            let id = jobs.run(action, host.clone());
            screen.job_started(id, purpose, label);
            Vec::new()
        }
        Effect::Terminal { action, purpose } => {
            let ok = hand_over_terminal(&action, &purpose, host.as_ref())?;
            *terminal = new_terminal()?;
            terminal.clear()?;
            screen.terminal_finished(&purpose, ok)
        }
        Effect::Redetect => {
            if detecting.is_none() {
                let (tx, rx) = mpsc::channel();
                let host = host.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(forge_onboarding::detect(host));
                });
                *detecting = Some(rx);
            }
            Vec::new()
        }
        Effect::OpenUrl(url) => {
            let _ = crate::cli::commands::mcp::open_browser(&url);
            Vec::new()
        }
        Effect::Persist => {
            screen.session.save();
            Vec::new()
        }
    })
}

/// Suspend the screen, run `action` with the terminal inherited, wait for Enter so its output
/// can be read (and a printed token copied), then take the screen back.
fn hand_over_terminal(action: &Action, purpose: &Purpose, host: &dyn Host) -> Result<bool> {
    let Exec::Terminal(cmd) = action.exec(host) else {
        return Ok(false);
    };
    onboarding::terminal::leave();
    let mut out = io::stdout();
    let _ = writeln!(out, "\n{}\n", action.label());
    let program = cmd
        .program
        .clone()
        .or_else(|| host.forge_exe().map(|p| p.display().to_string()))
        .unwrap_or_else(|| "forge".to_string());
    let status = std::process::Command::new(&program)
        .args(&cmd.args)
        .status();
    let ok = match status {
        Ok(status) => status.success(),
        Err(error) => {
            let _ = writeln!(out, "Could not run `{}`: {error}", cmd.display());
            false
        }
    };
    let hint = if matches!(purpose, Purpose::ClaudeTokenFlow) {
        "Copy the token it printed, then press Enter to return to setup and paste it."
    } else if ok {
        "Press Enter to return to setup."
    } else {
        "That did not finish. Press Enter to return to setup."
    };
    let _ = write!(out, "\n{hint} ");
    let _ = out.flush();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    onboarding::terminal::enter().context("reopening the setup screen")?;
    Ok(ok)
}
