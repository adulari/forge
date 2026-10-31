//! The TUI side of `/run`: owns the child processes of the project's `[[actions]]`, keeps the
//! overlay's snapshot current and scans for the ports they open. The overlay itself is pure
//! (`forge_tui::run_menu`); every process and socket goes through here.
//!
//! Each action runs as `$SHELL -lc <command>` in its own process group, so stopping it takes the
//! dev server's children down too. Nothing outlives the TUI: `shutdown` (also run on drop) kills
//! every group.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forge_config::project_actions::{actions_for, ProjectAction};
use forge_tui::run_menu::{ActionRow, ActionView, MenuAction, PortView, RunState, Tone};
use forge_tui::{App, KeyKind};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;

const OUTPUT_CAP: usize = 300;
const SNAPSHOT_LINES: usize = 12;
const PORTS_EVERY: Duration = Duration::from_secs(2);
const KILL_GRACE: Duration = Duration::from_millis(1500);

#[derive(Default)]
struct Shared {
    output: Mutex<VecDeque<String>>,
    /// `Some(code)` once the shell exited; `code` is `None` when a signal ended it.
    exit: Mutex<Option<Option<i32>>>,
}

struct Proc {
    pid: u32,
    started_ms: i64,
    shared: Arc<Shared>,
    stopping: bool,
}

impl Proc {
    fn exited(&self) -> Option<Option<i32>> {
        *self.shared.exit.lock().unwrap()
    }
}

pub(crate) struct RunMenuDriver {
    root: PathBuf,
    defs: Vec<ProjectAction>,
    untrusted: bool,
    procs: HashMap<String, Proc>,
    restart: HashSet<String>,
    loaded: bool,
    ports_tx: mpsc::UnboundedSender<Vec<PortView>>,
    ports_rx: mpsc::UnboundedReceiver<Vec<PortView>>,
    ports_at: Option<Instant>,
    ports_inflight: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn view_of(action: &ProjectAction) -> ActionView {
    ActionView {
        id: action.id.clone(),
        name: action.name.clone(),
        command: action.command.clone(),
        cwd: action.cwd.clone(),
    }
}

/// The action's working directory: inside the root, existing, symlinks resolved.
fn action_cwd(root: &Path, action: &ProjectAction) -> Result<PathBuf, String> {
    let base = root
        .canonicalize()
        .map_err(|e| format!("project directory unavailable: {e}"))?;
    let Some(sub) = action.cwd.as_deref() else {
        return Ok(base);
    };
    let joined = base
        .join(sub)
        .canonicalize()
        .map_err(|_| format!("cwd \"{sub}\" does not exist"))?;
    if joined.starts_with(&base) && joined.is_dir() {
        Ok(joined)
    } else {
        Err(format!("cwd \"{sub}\" is outside the project"))
    }
}

fn declares_actions(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".forge/config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .is_some_and(|table| table.contains_key("actions"))
}

fn shell_args(shell: &str, line: &str) -> Vec<String> {
    if cfg!(windows) {
        let name = Path::new(shell)
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        return match name.as_str() {
            "powershell" | "pwsh" => vec!["-NoLogo".into(), "-Command".into(), line.into()],
            _ => vec!["/C".into(), line.into()],
        };
    }
    vec!["-lc".into(), line.into()]
}

fn signal_group(pid: u32, hard: bool) {
    #[cfg(unix)]
    if pid > 1 {
        let sig = if hard { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: kill(2) with a negative pid signals a process group; no memory is touched.
        // ESRCH when the group is already gone is the expected outcome.
        unsafe { libc::kill(-(pid as i32), sig) };
    }
    #[cfg(not(unix))]
    {
        let _ = hard;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

#[cfg(unix)]
fn group_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    unsafe { libc::kill(-(pid as i32), 0) == 0 }
}

#[cfg(not(unix))]
fn group_alive(_pid: u32) -> bool {
    false
}

fn pump<R: AsyncRead + Unpin + Send + 'static>(reader: R, shared: Arc<Shared>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.replace('\t', "    ");
            let mut out = shared.output.lock().unwrap();
            if out.len() == OUTPUT_CAP {
                out.pop_front();
            }
            out.push_back(line);
        }
    });
}

impl RunMenuDriver {
    pub(crate) fn new() -> Self {
        Self::in_dir(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    fn in_dir(root: PathBuf) -> Self {
        let (ports_tx, ports_rx) = mpsc::unbounded_channel();
        Self {
            root,
            defs: Vec::new(),
            untrusted: false,
            procs: HashMap::new(),
            restart: HashSet::new(),
            loaded: false,
            ports_tx,
            ports_rx,
            ports_at: None,
            ports_inflight: false,
        }
    }

    fn reload(&mut self) {
        let mut defs = actions_for(&self.root);
        self.untrusted = defs.is_empty()
            && declares_actions(&self.root)
            && !forge_config::project_trust::is_trusted(&self.root);
        // An action removed from the config while it runs stays visible so it can be stopped.
        for (id, proc) in &self.procs {
            if proc.exited().is_none() && !defs.iter().any(|d| &d.id == id) {
                if let Some(old) = self.defs.iter().find(|d| &d.id == id) {
                    defs.push(old.clone());
                }
            }
        }
        self.defs = defs;
        self.loaded = true;
    }

    fn start(&mut self, app: &mut App, id: &str) {
        if self.procs.get(id).is_some_and(|p| p.exited().is_none()) {
            return;
        }
        let Some(action) = self.defs.iter().find(|d| d.id == id).cloned() else {
            return;
        };
        match self.spawn(&action) {
            Ok(proc) => {
                self.procs.insert(id.to_string(), proc);
            }
            Err(error) => app
                .run_menu
                .notify(Tone::Warn, format!("{}: {error}", action.name)),
        }
    }

    fn spawn(&self, action: &ProjectAction) -> Result<Proc, String> {
        let cwd = action_cwd(&self.root, action)?;
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "cmd".into()
                } else {
                    "sh".into()
                }
            });
        let mut command = tokio::process::Command::new(&shell);
        command
            .args(shell_args(&shell, &action.command))
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start `{shell}`: {e}"))?;
        let pid = child.id().ok_or("the process exited immediately")?;
        let shared = Arc::new(Shared::default());
        if let Some(out) = child.stdout.take() {
            pump(out, shared.clone());
        }
        if let Some(err) = child.stderr.take() {
            pump(err, shared.clone());
        }
        let waiter = shared.clone();
        tokio::spawn(async move {
            let code = match child.wait().await {
                Ok(status) => status.code(),
                Err(_) => None,
            };
            *waiter.exit.lock().unwrap() = Some(code);
        });
        Ok(Proc {
            pid,
            started_ms: now_ms(),
            shared,
            stopping: false,
        })
    }

    fn stop(&mut self, id: &str) {
        let Some(proc) = self.procs.get_mut(id) else {
            return;
        };
        if proc.exited().is_some() && !group_alive(proc.pid) {
            return;
        }
        proc.stopping = true;
        let pid = proc.pid;
        signal_group(pid, false);
        tokio::spawn(async move {
            tokio::time::sleep(KILL_GRACE).await;
            signal_group(pid, true);
        });
    }

    /// Kill every action's process group before the TUI goes away. Blocks for at most the grace
    /// period, and only when something ignored SIGTERM.
    pub(crate) fn shutdown(&mut self) {
        let pids: Vec<u32> = self.procs.values().map(|p| p.pid).collect();
        for &pid in &pids {
            signal_group(pid, false);
        }
        let deadline = Instant::now() + KILL_GRACE;
        while pids.iter().any(|&p| group_alive(p)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        for &pid in &pids {
            signal_group(pid, true);
        }
        self.procs.clear();
    }

    fn state_of(&self, id: &str) -> (RunState, Vec<String>) {
        let Some(proc) = self.procs.get(id) else {
            return (RunState::Idle, Vec::new());
        };
        let output = {
            let out = proc.shared.output.lock().unwrap();
            out.iter()
                .skip(out.len().saturating_sub(SNAPSHOT_LINES))
                .cloned()
                .collect()
        };
        let state = match proc.exited() {
            None => RunState::Running {
                since_ms: proc.started_ms,
            },
            Some(code) => RunState::Exited {
                code,
                stopped: proc.stopping,
            },
        };
        (state, output)
    }

    fn rows(&self) -> Vec<ActionRow> {
        self.defs
            .iter()
            .map(|def| {
                let (state, output) = self.state_of(&def.id);
                ActionRow {
                    action: view_of(def),
                    state,
                    output,
                }
            })
            .collect()
    }

    fn scan_ports(&mut self) {
        if self.ports_inflight {
            return;
        }
        self.ports_inflight = true;
        self.ports_at = Some(Instant::now());
        let mut roots: Vec<(u32, Option<String>)> = self
            .procs
            .iter()
            .filter(|(_, p)| p.exited().is_none())
            .map(|(id, p)| (p.pid, Some(id.clone())))
            .collect();
        roots.push((std::process::id(), None));
        let root = self.root.clone();
        let tx = self.ports_tx.clone();
        tokio::task::spawn_blocking(move || {
            let ports = crate::ports::scan(&roots, Some(&root))
                .into_iter()
                .map(|p| PortView {
                    port: p.port,
                    host: p.host.to_string(),
                    pid: p.pid,
                    process: p.process,
                    command: p.command,
                    source: p.source.to_string(),
                    action: p.action,
                })
                .collect();
            let _ = tx.send(ports);
        });
    }

    /// Reload, reap, refresh the overlay's snapshot and ports. True when a redraw is needed.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        let mut dirty = false;
        if !self.loaded || app.run_menu.take_refresh() {
            self.reload();
            app.run_menu.project = self
                .root
                .file_name()
                .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned());
            self.ports_at = None;
            dirty = true;
        }
        let finished: Vec<String> = self
            .restart
            .iter()
            .filter(|id| {
                self.procs
                    .get(*id)
                    .is_none_or(|p| p.exited().is_some() && !group_alive(p.pid))
            })
            .cloned()
            .collect();
        for id in finished {
            self.restart.remove(&id);
            self.start(app, &id);
        }
        if app.run_menu.open || !self.procs.is_empty() || !app.run_menu.actions.is_empty() {
            let rows = self.rows();
            if rows != app.run_menu.actions || self.untrusted != app.run_menu.untrusted {
                app.run_menu.apply_actions(rows, self.untrusted);
                dirty = true;
            }
        }
        if app.run_menu.open {
            if self.ports_at.is_none_or(|at| at.elapsed() >= PORTS_EVERY) {
                self.scan_ports();
            }
            while let Ok(ports) = self.ports_rx.try_recv() {
                self.ports_inflight = false;
                if ports != app.run_menu.ports || !app.run_menu.ports_scanned {
                    app.run_menu.apply_ports(ports);
                    dirty = true;
                }
            }
        } else {
            self.ports_at = None;
            while self.ports_rx.try_recv().is_ok() {
                self.ports_inflight = false;
            }
        }
        app.run_menu.advance(now_ms()) || dirty
    }

    pub(crate) fn key(
        &mut self,
        app: &mut App,
        clipboard: &mut Option<arboard::Clipboard>,
        key: KeyKind,
    ) {
        match app.run_menu.key(key) {
            MenuAction::None | MenuAction::Close => {}
            MenuAction::Start(id) => self.start(app, &id),
            MenuAction::Stop(id) => self.stop(&id),
            MenuAction::Restart(id) => {
                if self.procs.get(&id).is_some_and(|p| p.exited().is_none()) {
                    self.restart.insert(id.clone());
                    self.stop(&id);
                } else {
                    self.start(app, &id);
                }
            }
            MenuAction::CopyUrl(port) => {
                let url = format!("http://localhost:{port}");
                super::copy::copy_selection(clipboard, &url);
                app.run_menu.notify(Tone::Ok, format!("Copied {url}"));
            }
            MenuAction::OpenUrl(port) => {
                let url = format!("http://localhost:{port}");
                match crate::cli::commands::mcp::open_browser(&url) {
                    Ok(()) => app.run_menu.notify(Tone::Ok, format!("Opening {url}")),
                    Err(error) => app
                        .run_menu
                        .notify(Tone::Warn, format!("Could not open a browser: {error}")),
                }
            }
        }
        self.tick(app);
    }
}

impl Drop for RunMenuDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
