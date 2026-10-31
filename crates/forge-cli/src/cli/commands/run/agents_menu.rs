//! The TUI side of `/clis`: reads the agent CLIs' install state and, once the person has
//! confirmed the exact command in the menu, runs the vendor installers one at a time, streaming
//! their output back. `--mock` never starts a process.

use forge_provider::agent_install::{self, AgentStatus};
use forge_provider::CliKind;
use forge_tui::agents_menu::{AgentRow, Job, MenuAction};
use forge_tui::{App, KeyKind};
use tokio::sync::mpsc;

enum Msg {
    Rows(Result<Vec<AgentRow>, String>),
    Line(String),
    Finished {
        id: String,
        result: Result<String, String>,
    },
    AllDone,
}

/// `/clis`: the overlay in the full-screen terminal, a pointer to `forge doctor --fix` elsewhere.
pub(crate) fn open_or_explain(app: &mut App, full_screen: bool) {
    if full_screen {
        app.agents_menu.open();
    } else {
        app.note(
            "/clis needs the full-screen terminal; run `forge doctor --fix` to update agent CLIs.",
        );
    }
}

pub(crate) struct AgentsMenuDriver {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    mock: bool,
    running: bool,
    loading: bool,
}

fn row_from(status: AgentStatus) -> AgentRow {
    AgentRow {
        id: status.id.to_string(),
        name: status.name.to_string(),
        installed: status.installed,
        version: status.version,
        latest: status.latest,
        outdated: status.outdated,
        runtime: match status.runtime {
            "wsl" => "WSL",
            "windows_interop" => "Windows via WSL",
            _ => "",
        }
        .to_string(),
        command: status.command,
        hint: status.manual_hint,
        job: Job::Idle,
    }
}

fn mock_rows() -> Vec<AgentRow> {
    let row = |id: &str, name: &str, version: &str, outdated: bool, command: &str| AgentRow {
        id: id.into(),
        name: name.into(),
        installed: true,
        version: Some(version.into()),
        latest: Some("2.1.400".into()),
        outdated,
        command: Some(command.into()),
        ..Default::default()
    };
    vec![
        row(
            "claude-cli",
            "Claude Code",
            "2.1.314",
            true,
            "claude update",
        ),
        row(
            "codex-cli",
            "OpenAI Codex",
            "2.1.400",
            false,
            "codex update",
        ),
        AgentRow {
            id: "agy-cli".into(),
            name: "Google Antigravity".into(),
            hint: Some("install Antigravity from its download page, then run `agy` once".into()),
            ..Default::default()
        },
    ]
}

impl AgentsMenuDriver {
    pub(crate) fn new(mock: bool) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            mock,
            running: false,
            loading: false,
        }
    }

    fn load(&mut self) {
        if self.loading {
            return;
        }
        self.loading = true;
        let (tx, mock) = (self.tx.clone(), self.mock);
        tokio::spawn(async move {
            let rows = if mock {
                Ok(mock_rows())
            } else {
                Ok(agent_install::statuses(true)
                    .await
                    .into_iter()
                    .map(row_from)
                    .collect())
            };
            let _ = tx.send(Msg::Rows(rows));
        });
    }

    /// Apply finished background work. True when the screen needs a redraw.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        let mut dirty = false;
        if app.agents_menu.open && app.agents_menu.take_refresh() {
            self.load();
        }
        while let Ok(msg) = self.rx.try_recv() {
            dirty = true;
            let menu = &mut app.agents_menu;
            match msg {
                Msg::Rows(rows) => {
                    self.loading = false;
                    menu.apply_rows(rows);
                }
                Msg::Line(line) => menu.push_log(line),
                Msg::Finished { id, result } => match result {
                    Ok(_) => menu.set_job(&id, Job::Done),
                    Err(error) => {
                        menu.set_job(&id, Job::Failed);
                        menu.push_log(error);
                    }
                },
                Msg::AllDone => {
                    self.running = false;
                    self.load();
                }
            }
        }
        dirty
    }

    pub(crate) fn key(&mut self, app: &mut App, key: KeyKind) {
        let MenuAction::Run(ids) = app.agents_menu.key(key) else {
            return;
        };
        if self.running {
            for id in &ids {
                app.agents_menu.set_job(id, Job::Idle);
            }
            app.agents_menu.notice = Some("Another job is still running.".to_string());
            return;
        }
        self.running = true;
        let (tx, mock) = (self.tx.clone(), self.mock);
        tokio::spawn(async move {
            for id in ids {
                let Some(kind) = CliKind::all().into_iter().find(|k| k.prefix() == id) else {
                    continue;
                };
                let lines = tx.clone();
                let result = if mock {
                    let _ = lines.send(Msg::Line(format!(
                        "$ {} (mock: nothing is run)",
                        kind.binary_name()
                    )));
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                    Ok("mock".to_string())
                } else {
                    agent_install::apply(kind, move |line| {
                        let _ = lines.send(Msg::Line(line));
                    })
                    .await
                    .map(|done| done.after.unwrap_or_default())
                };
                let _ = tx.send(Msg::Finished { id, result });
            }
            let _ = tx.send(Msg::AllDone);
        });
    }
}
