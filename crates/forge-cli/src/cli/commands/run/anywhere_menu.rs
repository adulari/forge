//! The TUI side of `/anywhere`: polls status while the menu is open and performs what
//! `forge_tui::anywhere_menu::MenuAction` asks for. The menu itself is pure; every read of the
//! machine or the service goes through `crate::anywhere::tui_menu`.

use std::time::{Duration, Instant};

use anyhow::Result;
use forge_tui::anywhere_menu::{AnywhereSnapshot, Devices, MenuAction, Tone};
use forge_tui::{App, KeyKind, Tui};
use tokio::sync::mpsc;

use crate::anywhere::tui_menu;

const WEB_URL: &str = "https://app.forge.adulari.dev/anywhere";
const STATUS_EVERY: Duration = Duration::from_secs(3);
const DEVICES_EVERY: Duration = Duration::from_secs(30);

enum Msg {
    Snapshot(Result<AnywhereSnapshot, String>),
    Devices(Devices),
    Toggled {
        label: String,
        enabled: bool,
        result: Result<(), String>,
    },
    Synced(Result<(), String>),
}

pub(crate) struct AnywhereMenuDriver {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    mock: bool,
    status_at: Option<Instant>,
    devices_at: Option<Instant>,
    status_inflight: bool,
    devices_inflight: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl AnywhereMenuDriver {
    pub(crate) fn new(mock: bool) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            mock,
            status_at: None,
            devices_at: None,
            status_inflight: false,
            devices_inflight: false,
        }
    }

    fn spawn_status(&mut self) {
        if self.status_inflight {
            return;
        }
        self.status_inflight = true;
        self.status_at = Some(Instant::now());
        let (tx, mock) = (self.tx.clone(), self.mock);
        tokio::spawn(async move {
            let _ = tx.send(Msg::Snapshot(tui_menu::load_snapshot(mock).await));
        });
    }

    fn spawn_devices(&mut self) {
        if self.devices_inflight {
            return;
        }
        self.devices_inflight = true;
        self.devices_at = Some(Instant::now());
        let (tx, mock) = (self.tx.clone(), self.mock);
        tokio::spawn(async move {
            let _ = tx.send(Msg::Devices(tui_menu::load_devices(mock).await));
        });
    }

    /// Results that land while the menu is closed are discarded, but the fetches they answer are
    /// over: clear their inflight flags so the next open can start new ones.
    fn drop_stale_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Snapshot(_) => self.status_inflight = false,
                Msg::Devices(_) => self.devices_inflight = false,
                _ => {}
            }
        }
    }

    /// Drive refreshes and apply results. True when the screen needs a redraw.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        if !app.anywhere_menu.open {
            self.status_at = None;
            self.devices_at = None;
            self.drop_stale_messages();
            return false;
        }
        let mut dirty = false;
        if app.anywhere_menu.take_refresh() {
            self.spawn_status();
            self.spawn_devices();
        }
        if self.status_at.is_none_or(|at| at.elapsed() >= STATUS_EVERY) {
            self.spawn_status();
        }
        if self
            .devices_at
            .is_none_or(|at| at.elapsed() >= DEVICES_EVERY)
        {
            self.spawn_devices();
        }
        while let Ok(msg) = self.rx.try_recv() {
            dirty = true;
            match msg {
                Msg::Snapshot(result) => {
                    self.status_inflight = false;
                    app.anywhere_menu.apply_snapshot(result);
                }
                Msg::Devices(devices) => {
                    self.devices_inflight = false;
                    app.anywhere_menu.apply_devices(devices);
                }
                Msg::Toggled {
                    label,
                    enabled,
                    result,
                } => {
                    match result {
                        Ok(()) if enabled => app
                            .anywhere_menu
                            .notify(Tone::Ok, format!("{label} sync is on for this device.")),
                        Ok(()) => app.anywhere_menu.notify(
                            Tone::Ok,
                            format!("{label} sync is off. Data already on this machine stays."),
                        ),
                        Err(error) => app.anywhere_menu.notify(Tone::Err, error),
                    }
                    self.status_at = None;
                }
                Msg::Synced(result) => {
                    app.anywhere_menu.syncing = false;
                    match result {
                        Ok(()) => app.anywhere_menu.notify(Tone::Ok, "Sync cycle requested."),
                        Err(error) => app.anywhere_menu.notify(Tone::Warn, error),
                    }
                    self.status_at = None;
                }
            }
        }
        app.anywhere_menu.advance(now_ms()) || dirty
    }

    pub(crate) fn key(&mut self, app: &mut App, tui: &mut Tui, key: KeyKind) -> Result<()> {
        match app.anywhere_menu.key(key) {
            MenuAction::None | MenuAction::Close => {}
            MenuAction::Refresh => {}
            MenuAction::Toggle { category, enabled } => {
                let label = app
                    .anywhere_menu
                    .data
                    .as_ref()
                    .and_then(|d| d.categories.iter().find(|c| c.id == category))
                    .map_or_else(|| category.clone(), |c| c.label.clone());
                let (tx, mock) = (self.tx.clone(), self.mock);
                tokio::spawn(async move {
                    let result = tui_menu::set_category(mock, &category, enabled).await;
                    let _ = tx.send(Msg::Toggled {
                        label,
                        enabled,
                        result,
                    });
                });
            }
            MenuAction::SyncNow => {
                let (tx, mock) = (self.tx.clone(), self.mock);
                tokio::spawn(async move {
                    let _ = tx.send(Msg::Synced(tui_menu::sync_now(mock).await));
                });
            }
            MenuAction::SignInProvider { label, command } => {
                self.run_forge(app, tui, &label, &command);
            }
            MenuAction::SetupAnywhere => {
                self.run_forge(app, tui, "Forge Anywhere", "forge anywhere setup");
            }
            MenuAction::OpenWeb => match crate::cli::commands::mcp::open_browser(WEB_URL) {
                Ok(()) => app
                    .anywhere_menu
                    .notify(Tone::Ok, "Opening the Recovery Center in your browser."),
                Err(error) => app.anywhere_menu.notify(
                    Tone::Warn,
                    format!("Could not open a browser: {error}. Visit {WEB_URL}"),
                ),
            },
        }
        Ok(())
    }

    /// Run `forge ...` with the terminal handed over, then come back and re-read status. The
    /// existing OAuth and setup flows print and prompt, so they get the whole terminal.
    fn run_forge(&mut self, app: &mut App, tui: &mut Tui, label: &str, command: &str) {
        let args: Vec<String> = command
            .split_whitespace()
            .skip(1)
            .map(str::to_owned)
            .collect();
        if self.mock {
            app.anywhere_menu.notify(
                Tone::Ok,
                format!("--mock: would run `{command}` for {label}."),
            );
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            app.anywhere_menu
                .notify(Tone::Err, "Could not find the forge binary.");
            return;
        };
        let ran = tui.run_fullscreen(|| {
            let _ = crossterm::terminal::disable_raw_mode();
            println!("\n{label}: running `{command}`\n");
            let status = std::process::Command::new(exe).args(&args).status();
            println!("\nPress Enter to return to Forge.");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            status
        });
        match ran {
            Ok(status) if status.success() => {
                app.anywhere_menu
                    .notify(Tone::Ok, format!("{label}: done."));
            }
            Ok(status) => app.anywhere_menu.notify(
                Tone::Warn,
                format!("{label}: `{command}` exited with {status}."),
            ),
            Err(error) => app.anywhere_menu.notify(
                Tone::Err,
                format!("{label}: could not run `{command}`: {error}"),
            ),
        }
        self.status_at = None;
        self.devices_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fetch_that_finishes_while_closed_does_not_wedge_the_next_open() {
        let mut driver = AnywhereMenuDriver::new(true);
        driver.status_inflight = true;
        driver.devices_inflight = true;
        driver
            .tx
            .send(Msg::Snapshot(Err("gone".to_string())))
            .unwrap();
        driver
            .tx
            .send(Msg::Devices(Devices::Unavailable("gone".to_string())))
            .unwrap();
        let mut app = App::default();
        assert!(!app.anywhere_menu.open);
        driver.tick(&mut app);
        assert!(!driver.status_inflight);
        assert!(!driver.devices_inflight);
    }
}
