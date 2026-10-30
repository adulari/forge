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
use forge_tui::onboarding::{self, Effect, Exit, Screen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::host::CliHost;

pub(crate) struct Outcome {
    pub(crate) exit: Exit,
    pub(crate) session: Session,
    /// The session-start card was answered with "don't ask again".
    pub(crate) card_dismissed: bool,
}

type Term = Terminal<CrosstermBackend<io::Stdout>>;

pub(crate) fn run(session: Session, launches_session: bool, card: bool) -> Result<Outcome> {
    onboarding::terminal::install_panic_restore();
    onboarding::terminal::enter().context("opening the setup screen")?;
    let result = event_loop(session, launches_session, card);
    onboarding::terminal::leave();
    result
}

fn new_terminal() -> Result<Term> {
    Terminal::new(CrosstermBackend::new(io::stdout())).context("drawing the setup screen")
}

fn event_loop(session: Session, launches_session: bool, card: bool) -> Result<Outcome> {
    let host: Arc<dyn Host> = Arc::new(CliHost);
    let jobs = Jobs::new();
    let mut terminal = new_terminal()?;
    let mut screen = Screen::new(session);
    screen.launches_session = launches_session;
    screen.reconnect_card = card;
    screen.can_open_browser = browser_opens_here();
    screen.rich_glyphs = draws_sextants();
    let mut detecting: Option<Receiver<Machine>> = None;
    let mut effects = screen.opening_effects();
    let mut links: Vec<onboarding::LinkRow> = Vec::new();

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
        let frame = terminal.draw(|f| onboarding::render(f, &screen))?;
        // ratatui only redraws changed cells, so a hyperlink written once stays until its rows move.
        match screen.sign_in.as_ref().and_then(|s| s.url.as_deref()) {
            Some(url) => {
                let rows = onboarding::link_rows(frame.buffer, url);
                if rows != links {
                    write_links(&rows, url)?;
                    links = rows;
                }
            }
            None => links.clear(),
        }

        if event::poll(Duration::from_millis(60))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    effects.extend(screen.key(key));
                }
                Event::Paste(text) => effects.extend(screen.paste(&text)),
                Event::Resize(..) => {
                    terminal.clear()?;
                    links.clear();
                }
                _ => {}
            }
        } else {
            screen.tick();
        }
    }

    // Settings saved in the background (the last page's choices) must land before the process
    // can end; a sign-in or install is waited for only briefly.
    let settle = std::time::Instant::now() + Duration::from_secs(5);
    while screen
        .jobs
        .iter()
        .any(|j| jobs.state(j.id) == Some(forge_onboarding::JobState::Running))
        && std::time::Instant::now() < settle
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    if !card {
        screen.session.save();
    }
    if screen.exit == Some(Exit::Finished) && screen.ui.trust_folder {
        if let Ok(dir) = std::env::current_dir() {
            let _ = forge_config::project_trust::trust(&dir);
        }
    }
    let exit = screen.exit.unwrap_or(Exit::Left);
    Ok(Outcome {
        exit,
        card_dismissed: screen.card_dismissed,
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
            let ok = hand_over_terminal(&action, host.as_ref())?;
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
        Effect::Input { job, text } => {
            jobs.send_input(job, text);
            Vec::new()
        }
        Effect::Cancel(job) => {
            jobs.cancel(job);
            Vec::new()
        }
        Effect::OpenUrl(url) => {
            let _ = crate::cli::commands::mcp::open_browser(&url);
            Vec::new()
        }
        Effect::Copy(text) => {
            crate::cli::commands::run::copy::osc52_copy(&text);
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
fn hand_over_terminal(action: &Action, host: &dyn Host) -> Result<bool> {
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
    let hint = if ok {
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

/// Rewrite the sign-in link's rows with an OSC 8 hyperlink so a click opens it, wrapped or not.
/// The text and colours are the ones just drawn, so nothing visibly changes.
fn write_links(rows: &[onboarding::LinkRow], url: &str) -> io::Result<()> {
    use crossterm::cursor::MoveTo;
    use crossterm::style::{Print, ResetColor, SetAttribute, SetStyle};
    use ratatui::prelude::IntoCrossterm;
    if rows.is_empty() {
        return Ok(());
    }
    let mut out = io::stdout();
    for row in rows {
        crossterm::queue!(
            out,
            MoveTo(row.x, row.y),
            SetStyle(row.style.into_crossterm()),
            Print(onboarding::osc8(url, &row.text)),
            SetAttribute(crossterm::style::Attribute::Reset),
            ResetColor,
        )?;
    }
    out.flush()
}

/// `o` would open a browser on the remote machine over SSH, and there is none without a display.
fn browser_opens_here() -> bool {
    if ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
        .iter()
        .any(|v| std::env::var_os(v).is_some())
    {
        return false;
    }
    cfg!(any(target_os = "macos", windows))
        || ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

/// Terminals known to draw Unicode block sextants themselves (gapless, no font needed). SSH
/// carries `TERM` across, so this also holds on a remote host.
fn draws_sextants() -> bool {
    let term = std::env::var("TERM").unwrap_or_default();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    ["kitty", "ghostty", "foot", "wezterm"]
        .iter()
        .any(|t| term.contains(t))
        || matches!(program.as_str(), "WezTerm" | "ghostty")
}
