//! The TUI side of `/ci`: derives the plan and runs it off the render loop, then hands rows back
//! to the overlay (`forge_tui::ci_menu`). The same engine as `forge ci` (`crate::ci_run`).

use std::path::{Path, PathBuf};

use forge_config::ci_parity::report::load_report;
use forge_tui::ci_menu::{CiAction, CiRow, CiState};
use forge_tui::{App, KeyKind};
use tokio::sync::mpsc;

use crate::ci_run::{make_plan, run_plan, view_for, CiView, RunOptions, ViewState};

type Loaded = Result<(Vec<CiRow>, Option<String>), String>;

pub(crate) struct CiMenuDriver {
    tx: mpsc::UnboundedSender<Loaded>,
    rx: mpsc::UnboundedReceiver<Loaded>,
    /// The repository of the run in flight, whose progress the overlay shows.
    running_in: Option<PathBuf>,
}

fn root() -> PathBuf {
    let dir = std::env::current_dir().unwrap_or_default();
    std::process::Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or(dir, |o| {
            PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())
        })
}

fn tui_rows(view: &CiView) -> Vec<CiRow> {
    view.rows
        .iter()
        .map(|r| CiRow {
            state: match r.state {
                ViewState::Pending => CiState::Pending,
                ViewState::Pass => CiState::Pass,
                ViewState::Cached => CiState::Cached,
                ViewState::Fail => CiState::Fail,
                ViewState::NotRun => CiState::NotRun,
                ViewState::Skip => CiState::Skip,
            },
            name: r.name.clone(),
            detail: r.detail.clone(),
            secs: r.secs,
            command: r.command.clone(),
            env: r.env.clone(),
            tail: r.tail.clone(),
        })
        .collect()
}

fn load(root: &Path, run: bool) -> Loaded {
    let (plan, settings) = make_plan(root);
    let report = if run {
        Some(run_plan(
            root,
            &plan,
            &settings,
            &RunOptions::default(),
            &mut |_| {},
        )?)
    } else {
        load_report(root)
    };
    let view = view_for(&plan, report.as_ref());
    Ok((tui_rows(&view), view.summary))
}

impl CiMenuDriver {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            running_in: None,
        }
    }

    fn spawn(&self, root: PathBuf, run: bool) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || load(&root, run))
                .await
                .unwrap_or_else(|e| Err(format!("{e}")));
            let _ = tx.send(result);
        });
    }

    /// Apply a finished load or run. True when the screen needs a redraw.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        if !app.ci_menu.open {
            while self.rx.try_recv().is_ok() {}
            return false;
        }
        let mut dirty = false;
        if app.ci_menu.take_refresh() {
            self.spawn(root(), false);
            dirty = true;
        }
        while let Ok(result) = self.rx.try_recv() {
            match result {
                Ok((rows, summary)) => {
                    let failed = rows.iter().any(|r| r.state == CiState::Fail);
                    let ran = app.ci_menu.running;
                    app.ci_menu.apply(rows, summary);
                    if ran && failed {
                        app.ci_menu.notify(
                            forge_tui::ci_menu::Tone::Err,
                            "The plan failed: the same step stops a goal or a ship until it passes",
                        );
                    }
                }
                Err(e) => {
                    app.ci_menu.apply(Vec::new(), None);
                    app.ci_menu.notify(forge_tui::ci_menu::Tone::Err, e);
                }
            }
            dirty = true;
        }
        if app.ci_menu.running {
            app.ci_menu.progress = self
                .running_in
                .as_deref()
                .and_then(crate::ci_run::live::for_cwd)
                .and_then(|ci| ci.running)
                .map(|r| format!("{}/{} {}", r.index, r.total, r.name));
        }
        if app.ci_menu.running || app.ci_menu.loading {
            app.ci_menu.tick = app.ci_menu.tick.wrapping_add(1);
            dirty = true;
        }
        dirty
    }

    pub(crate) fn key(&mut self, app: &mut App, key: KeyKind) {
        if app.ci_menu.key(key) == CiAction::Run {
            let root = root();
            self.running_in = Some(root.clone());
            self.spawn(root, true);
        }
    }
}
