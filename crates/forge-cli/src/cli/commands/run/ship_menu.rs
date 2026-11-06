//! The TUI side of `/ship`: performs the ship request off the render loop and hands the outcome
//! back to the overlay (`forge_tui::ship_menu`).

use forge_tui::ship_menu::{ShipAction, ShipOutcome};
use forge_tui::{App, KeyKind};
use tokio::sync::mpsc;

use crate::cli::commands::issues::Daemon;

pub(crate) struct ShipMenuDriver {
    tx: mpsc::UnboundedSender<Result<ShipOutcome, String>>,
    rx: mpsc::UnboundedReceiver<Result<ShipOutcome, String>>,
}

impl ShipMenuDriver {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }

    fn spawn_ship(&self, session: String) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let daemon = Daemon::resolve(None, None).map_err(|e| format!("{e:#}"))?;
                let answer = daemon
                    .ship(&session, &serde_json::json!({}))
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                Ok(ShipOutcome::from_json(&answer))
            }
            .await;
            let _ = tx.send(result);
        });
    }

    /// Apply a finished run. True when the screen needs a redraw.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        if !app.ship_menu.open {
            while self.rx.try_recv().is_ok() {}
            return false;
        }
        let mut dirty = false;
        while let Ok(result) = self.rx.try_recv() {
            app.ship_menu.finish(result);
            dirty = true;
        }
        if app.ship_menu.running {
            app.ship_menu.tick = app.ship_menu.tick.wrapping_add(1);
            dirty = true;
        }
        dirty
    }

    pub(crate) fn key(&mut self, app: &mut App, key: KeyKind) {
        if app.ship_menu.key(key) == ShipAction::Start {
            self.spawn_ship(app.session_id.clone());
        }
    }
}
