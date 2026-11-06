//! The TUI side of `/issues`: asks the daemon for the project's issues when the overlay opens and
//! performs the start it asks for, off the render loop. The overlay itself is pure
//! (`forge_tui::issues_menu`).

use forge_tui::issues_menu::{IssueRow, IssuesAction};
use forge_tui::{App, KeyKind};
use tokio::sync::mpsc;

use crate::cli::commands::issues::{current_dir, Daemon, IssueLine};

enum Msg {
    Listed(Result<Vec<IssueRow>, String>),
    Started(Result<String, String>),
}

pub(crate) struct IssuesMenuDriver {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
}

fn rows(lines: Vec<IssueLine>) -> Vec<IssueRow> {
    lines
        .into_iter()
        .map(|l| IssueRow {
            number: l.number,
            title: l.title,
            labels: l.labels,
            author: l.author,
            comments: l.comments,
        })
        .collect()
}

impl IssuesMenuDriver {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }

    fn spawn_list(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let daemon = Daemon::resolve(None, None).map_err(|e| format!("{e:#}"))?;
                daemon
                    .list(&current_dir(), "open")
                    .await
                    .map(rows)
                    .map_err(|e| format!("{e:#}"))
            }
            .await;
            let _ = tx.send(Msg::Listed(result));
        });
    }

    fn spawn_start(&self, number: u64, worktree: bool, mode: &'static str) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let daemon = Daemon::resolve(None, None).map_err(|e| format!("{e:#}"))?;
                let started = daemon
                    .start(&current_dir(), number, worktree, mode, None)
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                Ok(format!(
                    "Started session {} on #{number}; follow it with: forge attach {}",
                    short(&started.session_id),
                    started.session_id
                ))
            }
            .await;
            let _ = tx.send(Msg::Started(result));
        });
    }

    /// Fetch the list when the overlay just opened and apply finished work. True when the screen
    /// needs a redraw.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        if !app.issues_menu.open {
            while self.rx.try_recv().is_ok() {}
            return false;
        }
        if app.issues_menu.take_refresh() {
            self.spawn_list();
        }
        let mut dirty = false;
        while let Ok(msg) = self.rx.try_recv() {
            dirty = true;
            match msg {
                Msg::Listed(result) => app.issues_menu.apply_list(result),
                Msg::Started(result) => app.issues_menu.finish_start(result),
            }
        }
        if app.issues_menu.starting
            || matches!(app.issues_menu.load, forge_tui::issues_menu::Load::Loading)
        {
            app.issues_menu.tick = app.issues_menu.tick.wrapping_add(1);
            dirty = true;
        }
        dirty
    }

    pub(crate) fn key(&mut self, app: &mut App, key: KeyKind) {
        if let IssuesAction::Start {
            number,
            worktree,
            watch,
        } = app.issues_menu.key(key)
        {
            self.spawn_start(number, worktree, watch.wire());
        }
    }
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
