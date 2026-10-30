//! The shell side of the TUI's update card: feeding it the background check's events, performing
//! what a key asks for (self-update, a package manager's command, a skip, a service restart) and
//! re-executing Forge once the new build is on disk. The card itself is `forge_tui::update_card`.

use std::sync::mpsc::{Receiver, Sender};

use forge_tui::update_card::{ServiceNote, UpdateAction};
use forge_tui::{App, KeyKind, Tui};

use crate::update_check::UpdateEvent;
use crate::update_state::UpdateState;

/// What the render loop does after a key went to the card.
pub(crate) enum Next {
    Continue,
    /// Leave the loop and re-exec Forge on the new build.
    Restart,
}

/// Fold the background events into the card. Returns `(redraw, installed)`.
pub(crate) fn drain(app: &mut App, rx: &Receiver<UpdateEvent>) -> (bool, bool) {
    let (mut dirty, mut installed) = (false, false);
    while let Ok(event) = rx.try_recv() {
        dirty = true;
        match event {
            UpdateEvent::Offer(offer) => app.update_card.offer(offer),
            UpdateEvent::Progress(text) => app.update_card.progress(&text),
            UpdateEvent::Finished(result) => {
                installed |= result.is_ok();
                app.update_card.finished(result);
            }
            UpdateEvent::Service(note) => app.update_card.set_service(note),
            UpdateEvent::Notice(text) => app.note(&text),
        }
    }
    (dirty, installed)
}

/// Keep the install spinner turning while the session is otherwise idle. Returns whether the
/// frame changed.
pub(crate) fn animate(app: &mut App) -> bool {
    use forge_tui::update_card::CardPhase;
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if !matches!(app.update_card.phase(), CardPhase::Running(_)) || app.busy {
        return false;
    }
    let tick = (START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        / 80) as usize;
    let changed = tick != app.tick;
    app.tick = tick;
    changed
}

/// Whether `key` belongs to the card right now: it is on screen, the composer is empty, and no
/// other modal, permission prompt or question owns the keyboard.
pub(crate) fn key_goes_to_card(app: &App, busy: bool, nothing_pending: bool) -> bool {
    let other_modal = app.effort_slider
        || app.command_center.open
        || app.palette.open
        || app.btw_open()
        || app.usage_overlay.open
        || app.anywhere_menu.open
        || app.status_open
        || app.mesh_overlay.open
        || app.at_picker.open
        || app.picker.open
        || app.voice.is_some();
    nothing_pending && !other_modal && app.update_card.captures_keys(busy, app.input.is_empty())
}

/// Perform what `key` asks of the card.
pub(crate) fn on_key(
    app: &mut App,
    tui: &mut Tui,
    tx: &Sender<UpdateEvent>,
    clipboard: &mut Option<arboard::Clipboard>,
    key: KeyKind,
) -> Next {
    let version = app
        .update_card
        .offer_ref()
        .map(|o| o.version.clone())
        .unwrap_or_default();
    match app.update_card.handle_key(key) {
        UpdateAction::None => {}
        UpdateAction::StartInPlace => start_in_place(tx.clone(), version),
        UpdateAction::RunCommand(command) => {
            let result = run_in_suspended_terminal(tui, &command).map(|()| version);
            let installed = result.is_ok();
            let _ = tx.send(UpdateEvent::Finished(result));
            if installed {
                probe_service(tx.clone());
            }
        }
        UpdateAction::CopyCommand(command) => {
            super::copy::copy_selection(clipboard, &command);
            app.note(&format!("✓ copied `{command}`"));
        }
        UpdateAction::Skip(version) => {
            let mut state = UpdateState::load();
            state.skip(&version);
            state.save();
            app.note(&format!(
                "skipping Forge {version}; the next release will be offered"
            ));
        }
        UpdateAction::Restart => return Next::Restart,
        UpdateAction::RestartService => restart_service(tx.clone()),
    }
    Next::Continue
}

fn start_in_place(tx: Sender<UpdateEvent>, version: String) {
    let _ = tx.send(UpdateEvent::Progress(format!(
        "downloading Forge {version} and replacing this binary…"
    )));
    tokio::task::spawn_blocking(move || match crate::update::apply_in_place(false) {
        Ok(Some(installed)) => {
            let _ = tx.send(UpdateEvent::Finished(Ok(installed)));
            probe_service(tx);
        }
        Ok(None) => {
            let _ = tx.send(UpdateEvent::Finished(Err(
                "this build is already the latest release".to_string(),
            )));
        }
        Err(e) => {
            let _ = tx.send(UpdateEvent::Finished(Err(format!("{e:#}"))));
        }
    });
}

/// After an install, say whether a background Forge service is still running the old build.
fn probe_service(tx: Sender<UpdateEvent>) {
    tokio::task::spawn_blocking(move || {
        let running = crate::cli::commands::service::service_is_running();
        if running {
            let _ = tx.send(UpdateEvent::Service(ServiceNote::Running));
        }
    });
}

/// Restart the Forge service, but only when no session it hosts is mid-turn: a restart drains
/// and re-adopts sessions, and cutting a running turn is not something an update should do.
fn restart_service(tx: Sender<UpdateEvent>) {
    tokio::spawn(async move {
        let note = match crate::serve_update::restart_service_when_idle().await {
            Ok(()) => ServiceNote::Restarted,
            Err(why) => ServiceNote::Held(why),
        };
        let _ = tx.send(UpdateEvent::Service(note));
    });
}

/// Run a package manager's upgrade with the terminal handed back to it, then wait for Enter so
/// its output can be read before the TUI redraws.
fn run_in_suspended_terminal(tui: &mut Tui, command: &str) -> Result<(), String> {
    tui.run_fullscreen(|| {
        use std::io::Write;
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
        println!("\n$ {command}\n");
        let status = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", command])
                .status()
        } else {
            std::process::Command::new("sh")
                .args(["-c", command])
                .status()
        };
        let outcome = match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("`{command}` exited with {s}")),
            Err(e) => Err(format!("could not run `{command}`: {e}")),
        };
        print!("\n[forge] press Enter to return to Forge ");
        let _ = std::io::stdout().flush();
        let _ = std::io::stdin().read_line(&mut String::new());
        Ok(outcome)
    })
    .map_err(|e| e.to_string())?
}

/// Replace this process with the new build, resuming `session` when it has history. Only returns
/// on failure: the caller then just exits and the person starts Forge by hand.
pub(crate) fn exec_new_build(resume: Option<&str>) -> std::io::Result<()> {
    crate::update_auto::exec_new_build(&crate::update_auto::swap_target(), resume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_tui::update_card::{UpdateOffer, UpdatePlan};

    fn app_with_offer() -> App {
        let mut app = App::default();
        app.update_card.offer(UpdateOffer {
            version: "9.9.9".into(),
            current: "1.0.0".into(),
            notes: vec![],
            url: String::new(),
            plan: UpdatePlan::InPlace,
        });
        app
    }

    #[test]
    fn letters_go_to_the_card_only_when_nothing_else_owns_the_keyboard() {
        let mut app = app_with_offer();
        assert!(key_goes_to_card(&app, false, true));
        assert!(!key_goes_to_card(&app, true, true), "a turn is running");
        assert!(!key_goes_to_card(&app, false, false), "a prompt is pending");
        app.input = "typing".into();
        assert!(
            !key_goes_to_card(&app, false, true),
            "a draft keeps its letters"
        );
        app.input.clear();
        app.picker.open = true;
        assert!(!key_goes_to_card(&app, false, true), "a modal is open");
    }

    #[test]
    fn background_events_drive_the_card_and_flag_a_finished_install() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App::default();
        let offer = app_with_offer().update_card.offer_ref().cloned().unwrap();
        tx.send(UpdateEvent::Offer(offer)).unwrap();
        assert_eq!(drain(&mut app, &rx), (true, false));
        assert!(app.update_card.visible(false));
        assert_eq!(
            app.update_card.handle_key(KeyKind::Char('u')),
            UpdateAction::StartInPlace
        );
        tx.send(UpdateEvent::Progress("halfway".into())).unwrap();
        tx.send(UpdateEvent::Finished(Ok("9.9.9".into()))).unwrap();
        tx.send(UpdateEvent::Service(ServiceNote::Running)).unwrap();
        assert_eq!(drain(&mut app, &rx), (true, true));
        assert_eq!(app.update_card.service(), &ServiceNote::Running);
        assert_eq!(
            drain(&mut app, &rx),
            (false, false),
            "nothing queued, nothing to redraw"
        );
    }
}
