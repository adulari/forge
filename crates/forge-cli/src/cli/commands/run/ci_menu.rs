//! The TUI side of `/ci`: derives the plan and runs it off the render loop, then hands rows back
//! to the overlay (`forge_tui::ci_menu`). The same engine as `forge ci` (`crate::ci_run`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    /// Stops the run in flight; set when the overlay closes and when the driver is dropped (the
    /// chat exits), so a quit never waits on the rest of the plan.
    cancel: Arc<AtomicBool>,
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

fn load(root: &Path, run: Option<Arc<AtomicBool>>) -> Loaded {
    let (plan, settings) = make_plan(root);
    let report = if let Some(cancel) = run {
        let opts = RunOptions {
            cancel: Some(cancel),
            ..RunOptions::default()
        };
        Some(run_plan(root, &plan, &settings, &opts, &mut |_| {})?)
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
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn spawn(&mut self, root: PathBuf, run: bool) {
        let tx = self.tx.clone();
        let run = run.then(|| {
            self.cancel = Arc::new(AtomicBool::new(false));
            Arc::clone(&self.cancel)
        });
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
        match app.ci_menu.key(key) {
            CiAction::Run => {
                let root = root();
                self.running_in = Some(root.clone());
                self.spawn(root, true);
            }
            CiAction::Close => self.cancel.store(true, Ordering::Relaxed),
            CiAction::None => {}
        }
    }
}

impl Drop for CiMenuDriver {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn quitting_mid_run_stops_the_plan_instead_of_waiting_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join(".github/workflows");
        std::fs::create_dir_all(&wf).unwrap();
        let started = dir.path().join("started");
        std::fs::write(
            wf.join("ci.yml"),
            format!(
                "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: long\n        run: touch {} && sleep 60\n",
                started.display()
            ),
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let begun = Instant::now();
        rt.block_on(async {
            let mut driver = CiMenuDriver::new();
            driver.spawn(dir.path().to_path_buf(), true);
            while !started.exists() {
                assert!(
                    begun.elapsed() < Duration::from_secs(20),
                    "the step never started"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            drop(driver);
        });
        drop(rt);
        assert!(
            begun.elapsed() < Duration::from_secs(20),
            "exit waited {:?} for the CI step",
            begun.elapsed()
        );
    }
}
