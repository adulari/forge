//! The TUI side of `/issues`: asks the daemon for the project's issues when the overlay opens and
//! performs the start it asks for, off the render loop. The overlay itself is pure
//! (`forge_tui::issues_menu`).

use forge_tui::issues_menu::{DrainView, IssueRow, IssuesAction};
use forge_tui::{App, KeyKind};
use tokio::sync::mpsc;

use crate::cli::commands::issues::{current_dir, Daemon, DrainLine, DrainRequest, IssueLine};

/// How often the open overlay asks the daemon how the drain is doing.
const DRAIN_POLL: std::time::Duration = std::time::Duration::from_secs(4);

enum Msg {
    Listed(Result<Vec<IssueRow>, String>),
    Started(Result<String, String>),
    Drained(Result<String, String>),
    Drains(Vec<DrainView>),
}

pub(crate) struct IssuesMenuDriver {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    last_poll: Option<std::time::Instant>,
}

fn drain_view(line: DrainLine) -> DrainView {
    DrainView {
        id: line.id,
        status: line.status,
        line: line.line,
        meter: line.meter,
        paused_reason: line.paused_reason,
        items: line
            .items
            .into_iter()
            .map(|i| (i.number, i.status))
            .collect(),
    }
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

    fn spawn_drains(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let Ok(daemon) = Daemon::resolve(None, None) else {
                return;
            };
            if let Ok(drains) = daemon.drains().await {
                let cwd = current_dir();
                let _ = tx.send(Msg::Drains(
                    drains
                        .into_iter()
                        .filter(|d| d.cwd == cwd || d.cwd.is_empty() || cwd.starts_with(&d.cwd))
                        .map(drain_view)
                        .collect(),
                ));
            }
        });
    }

    fn spawn_drain(
        &self,
        numbers: Vec<u64>,
        worktree: bool,
        mode: &'static str,
        parallel: usize,
        budget: Option<&'static str>,
    ) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let daemon = Daemon::resolve(None, None).map_err(|e| format!("{e:#}"))?;
                let count = numbers.len();
                daemon
                    .drain(
                        &current_dir(),
                        &DrainRequest {
                            numbers,
                            parallel: Some(parallel),
                            budget: budget.map(str::to_string),
                            worktree,
                            mode: mode.to_string(),
                            method: None,
                        },
                    )
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                Ok(format!("Draining {count} issues, {parallel} at a time"))
            }
            .await;
            let _ = tx.send(Msg::Drained(result));
        });
    }

    fn spawn_cancel(&self, id: String) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let daemon = Daemon::resolve(None, None).map_err(|e| format!("{e:#}"))?;
                daemon
                    .cancel_drain(&id)
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                Ok("Drain stopped; running sessions keep going".to_string())
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
            self.last_poll = None;
        }
        if self.last_poll.is_none_or(|at| at.elapsed() >= DRAIN_POLL) {
            self.last_poll = Some(std::time::Instant::now());
            self.spawn_drains();
        }
        let mut dirty = false;
        while let Ok(msg) = self.rx.try_recv() {
            dirty = true;
            match msg {
                Msg::Listed(result) => app.issues_menu.apply_list(result),
                Msg::Started(result) => app.issues_menu.finish_start(result),
                Msg::Drained(result) => {
                    app.issues_menu.finish_drain(result);
                    self.last_poll = None;
                }
                Msg::Drains(drains) => app.issues_menu.apply_drains(drains),
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
        match app.issues_menu.key(key) {
            IssuesAction::Start {
                number,
                worktree,
                watch,
            } => self.spawn_start(number, worktree, watch.wire()),
            IssuesAction::Drain {
                numbers,
                worktree,
                watch,
                parallel,
                budget,
            } => self.spawn_drain(numbers, worktree, watch.wire(), parallel, budget),
            IssuesAction::CancelDrain { id } => self.spawn_cancel(id),
            IssuesAction::None | IssuesAction::Close => {}
        }
    }
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
