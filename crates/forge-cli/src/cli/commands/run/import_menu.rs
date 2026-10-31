//! The TUI side of `/import`: scans in the background when the picker opens and performs the
//! imports it asks for. The picker itself is pure (`forge_tui::import_menu`); every read of the
//! machine and every write to the store happens here, off the render loop.

use anyhow::Result;
use forge_tui::import_menu::{ImportAction, ImportRow};
use forge_tui::{App, KeyKind};
use tokio::sync::mpsc;

use crate::cli::commands::import_sessions::{self, Agent, Selection};

enum Msg {
    Scanned(Result<Vec<ImportRow>, String>),
    Imported(Result<(usize, usize, usize), String>),
}

pub(crate) struct ImportMenuDriver {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
}

fn rows(found: Vec<import_sessions::Candidate>) -> Vec<ImportRow> {
    found
        .into_iter()
        .map(|c| ImportRow {
            agent: c.agent.label().to_string(),
            id: c.id,
            title: c.title,
            cwd: c.cwd,
            cwd_exists: c.cwd_exists,
            modified: c.modified,
            imported: c.imported_session_id.is_some(),
        })
        .collect()
}

/// The folder a conversation with no folder of its own opens in: where this chat was started.
pub(crate) fn fallback_dir() -> String {
    std::env::current_dir()
        .ok()
        .or_else(forge_config::home_dir)
        .map_or_else(|| "/".into(), |d| d.display().to_string())
}

impl ImportMenuDriver {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }

    fn spawn_scan(&self) {
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let found = crate::open_store()
                .map(|store| import_sessions::scan(Some(&store)))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Scanned(found.map(rows)));
        });
    }

    /// Start a scan when the picker just opened and apply finished work. True when the screen
    /// needs a redraw.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        if !app.import_menu.open {
            while self.rx.try_recv().is_ok() {}
            return false;
        }
        if app.import_menu.take_rescan() {
            self.spawn_scan();
        }
        let mut dirty = false;
        while let Ok(msg) = self.rx.try_recv() {
            dirty = true;
            match msg {
                Msg::Scanned(result) => app.import_menu.apply_scan(result),
                Msg::Imported(Ok((imported, skipped, failed))) => {
                    app.import_menu.finish_import(imported, skipped, failed)
                }
                Msg::Imported(Err(error)) => app.import_menu.fail_import(error),
            }
        }
        if app.import_menu.importing
            || matches!(app.import_menu.load, forge_tui::import_menu::Load::Scanning)
        {
            app.import_menu.tick = app.import_menu.tick.wrapping_add(1);
            dirty = true;
        }
        dirty
    }

    pub(crate) fn key(&mut self, app: &mut App, key: KeyKind) {
        if let ImportAction::Import(picks) = app.import_menu.key(key) {
            let picks: Vec<Selection> = picks
                .into_iter()
                .filter_map(|(agent, id)| {
                    let agent = match agent.as_str() {
                        "claude" => Agent::Claude,
                        "codex" => Agent::Codex,
                        _ => return None,
                    };
                    Some(Selection { agent, id })
                })
                .collect();
            let fallback = std::path::PathBuf::from(&app.import_menu.fallback);
            let tx = self.tx.clone();
            tokio::task::spawn_blocking(move || {
                let done = crate::open_store()
                    .map_err(|e| format!("{e:#}"))
                    .map(|store| {
                        import_sessions::import_selected(
                            &store,
                            &picks,
                            Some(&fallback),
                            &import_sessions::default_mode(),
                        )
                    })
                    .map(|results| {
                        let failed = results.iter().filter(|r| r.error.is_some()).count();
                        let skipped = results.iter().filter(|r| r.skipped).count();
                        (results.len() - failed - skipped, skipped, failed)
                    });
                let _ = tx.send(Msg::Imported(done));
            });
        }
    }
}
