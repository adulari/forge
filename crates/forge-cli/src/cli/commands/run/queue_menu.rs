//! The TUI side of `/queue`: asks the daemon for the merge queue while the overlay is open and
//! performs the hold, release, front or retry it asks for, off the render loop. The overlay
//! itself is pure (`forge_tui::queue_menu`).

use forge_tui::queue_menu::{QueueAction, QueueOp, QueueRow};
use forge_tui::{App, KeyKind};
use tokio::sync::mpsc;

use crate::cli::commands::issues::Daemon;

/// How often the open overlay asks the daemon how the queue is doing.
const POLL: std::time::Duration = std::time::Duration::from_secs(4);

enum Msg {
    Listed(Result<Vec<QueueRow>, String>),
    Acted(Result<(String, Vec<QueueRow>), String>),
}

pub(crate) struct QueueMenuDriver {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    last_poll: Option<std::time::Instant>,
}

impl QueueMenuDriver {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            last_poll: None,
        }
    }

    fn spawn_list(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let daemon = Daemon::resolve(None, None).map_err(|e| format!("{e:#}"))?;
                daemon
                    .merge_queue()
                    .await
                    .map(|answer| crate::cli::commands::prs::queue_rows(&answer))
                    .map_err(|e| format!("{e:#}"))
            }
            .await;
            let _ = tx.send(Msg::Listed(result));
        });
    }

    fn spawn_act(&self, session_id: String, op: QueueOp) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let daemon = Daemon::resolve(None, None).map_err(|e| format!("{e:#}"))?;
                let answer = daemon
                    .queue_action(&serde_json::json!({"session": session_id, "action": op.wire()}))
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                let message = answer["message"].as_str().unwrap_or("done").to_string();
                Ok((message, crate::cli::commands::prs::queue_rows(&answer)))
            }
            .await;
            let _ = tx.send(Msg::Acted(result));
        });
    }

    /// Fetch the list when the overlay just opened (and every few seconds after), apply finished
    /// work. True when the screen needs a redraw.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        if !app.queue_menu.open {
            while self.rx.try_recv().is_ok() {}
            self.last_poll = None;
            return false;
        }
        if app.queue_menu.take_refresh() {
            self.last_poll = None;
        }
        if self.last_poll.is_none_or(|at| at.elapsed() >= POLL) {
            self.last_poll = Some(std::time::Instant::now());
            self.spawn_list();
        }
        let mut dirty = false;
        while let Ok(msg) = self.rx.try_recv() {
            dirty = true;
            match msg {
                Msg::Listed(result) => app.queue_menu.apply_list(result),
                Msg::Acted(result) => app.queue_menu.finish_action(result),
            }
        }
        if app.queue_menu.busy
            || matches!(app.queue_menu.load, forge_tui::queue_menu::Load::Loading)
        {
            app.queue_menu.tick = app.queue_menu.tick.wrapping_add(1);
            dirty = true;
        }
        dirty
    }

    pub(crate) fn key(&mut self, app: &mut App, key: KeyKind) {
        if let QueueAction::Act { session_id, op } = app.queue_menu.key(key) {
            self.spawn_act(session_id, op);
        }
    }
}
