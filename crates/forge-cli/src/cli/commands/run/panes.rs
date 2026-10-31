//! The TUI host for the session strip, the right sidebar, the `/diff` overlay and `/jobs`.
//! The views are pure (`forge_tui::{session_strip, sidebar, diff_overlay, jobs_overlay}`); this is
//! where git, the filesystem, the daemon and the terminal are touched, off the render loop.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;
use forge_tui::board::FleetRow;
use forge_tui::diff_overlay::{parse_unified, untracked_file, DiffAction, OverlayFile};
use forge_tui::jobs_overlay::{JobRow, JobsAction, TAIL_LINES};
use forge_tui::{App, KeyKind, Tui};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::board::client::{fetch_fleet, ws_url};
use crate::board::sockets::is_fleet_changed;

const FLEET_REFRESH: Duration = Duration::from_secs(15);
const FLEET_BACKOFF: [u64; 4] = [1, 2, 5, 10];
const ATTACH_HEARTBEAT: Duration = Duration::from_secs(15);
const JOBS_EVERY: Duration = Duration::from_secs(2);
const MCP_EVERY: Duration = Duration::from_secs(5);
/// Untracked files larger than this, or past this many, are listed without their content.
const UNTRACKED_MAX_BYTES: u64 = 200 * 1024;
const UNTRACKED_MAX_FILES: usize = 40;

enum Msg {
    Fleet(Result<Vec<FleetRow>, String>),
    Tree(Result<Vec<OverlayFile>, String>),
    Jobs(Vec<JobRow>),
    Tail(u32, String),
    JobNote(String),
}

pub(crate) struct PaneHost {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    strip_feed: Option<tokio::task::JoinHandle<()>>,
    tree_inflight: bool,
    jobs_at: Option<Instant>,
    jobs_inflight: bool,
    mcp_at: Option<Instant>,
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Apply a `[tui]` setting that changes a pane, so the `/config` editor takes effect at once.
pub(crate) fn apply_config(app: &mut App, path: &str, value: &str) {
    let on = value.trim() == "true";
    match path {
        "tui.session_strip" => app.session_strip.enabled = on,
        "tui.sidebar" => app.sidebar.enabled = on,
        _ => {}
    }
}

fn persist(app: &mut App, key: &str, on: bool) {
    if let Err(e) = forge_config::set_config_value(
        forge_config::ConfigScope::User,
        key,
        if on { "true" } else { "false" },
    ) {
        app.note(&format!("  could not save {key}: {e}"));
    }
}

/// The state `/strip` or `/sidebar` asks for: a flip with no argument, otherwise on or off.
fn wanted(current: bool, arg: &str) -> Result<bool, String> {
    match arg.trim() {
        "" | "toggle" => Ok(!current),
        "on" | "show" | "true" => Ok(true),
        "off" | "hide" | "false" => Ok(false),
        other => Err(format!("expected on or off, not '{other}'")),
    }
}

/// `/strip [on|off]` and `/sidebar [on|off]`.
pub(crate) fn toggle(app: &mut App, which: &str, arg: &str) {
    let strip = which == "strip";
    let current = if strip {
        app.session_strip.enabled
    } else {
        app.sidebar.enabled
    };
    let next = match wanted(current, arg) {
        Ok(next) => next,
        Err(e) => {
            app.note(&format!("/{which}: {e}"));
            return;
        }
    };
    if strip {
        app.session_strip.enabled = next;
        persist(app, "tui.session_strip", next);
        app.note(&format!(
            "session strip {}",
            if next { "on (Alt+1-9 switches)" } else { "off" }
        ));
    } else {
        app.sidebar.enabled = next;
        persist(app, "tui.sidebar", next);
        let hint = if !next {
            ""
        } else if app.fullscreen {
            " (shows on terminals 120 columns wide or more)"
        } else {
            " (full-screen chat only)"
        };
        app.note(&format!(
            "sidebar {}{hint}",
            if next { "on" } else { "off" }
        ));
    }
}

impl PaneHost {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            strip_feed: None,
            tree_inflight: false,
            jobs_at: None,
            jobs_inflight: false,
            mcp_at: None,
        }
    }

    fn workspace(app: &App) -> PathBuf {
        app.workspace_path()
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Handle a key for the overlays and the pane chords. True when it was consumed.
    pub(crate) fn key(
        &mut self,
        app: &mut App,
        tui: &mut Tui,
        key: KeyKind,
        attach_blocked: bool,
        heartbeat: &mut dyn FnMut(),
    ) -> Result<bool> {
        if app.diff_overlay.open {
            if app.diff_overlay.key(key) == DiffAction::Refresh {
                self.tree_inflight = false;
            }
            return Ok(true);
        }
        if app.jobs_overlay.open {
            match app.jobs_overlay.key(key) {
                JobsAction::None => {}
                JobsAction::Refresh => self.jobs_at = None,
                JobsAction::View(id) => self.spawn_tail(app, id),
                JobsAction::Stop(id) => self.spawn_stop(app, id),
            }
            return Ok(true);
        }
        match key {
            KeyKind::ToggleSidebar => {
                let next = !app.sidebar.enabled;
                toggle(app, "sidebar", if next { "on" } else { "off" });
                Ok(true)
            }
            KeyKind::SessionSlot(n) if app.session_strip.enabled => {
                match app.session_strip.slot(n).map(|row| row.id.clone()) {
                    Some(id) if id == app.session_id => app.note("already in that session"),
                    Some(_) if attach_blocked => app.note(
                        "finish or answer the current turn first: attaching pauses this session",
                    ),
                    Some(id) => attach(app, tui, &id, heartbeat),
                    None => app.note(&format!("no session in slot {n}")),
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn spawn_tail(&mut self, app: &App, id: u32) {
        let workspace = Self::workspace(app);
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let log = forge_tools::job_views(&workspace)
                .into_iter()
                .find(|j| j.id == id)
                .map(|j| j.log);
            let text = match log {
                Some(log) => forge_tools::job_tail(&log, TAIL_LINES).unwrap_or_else(|e| e),
                None => format!("job {id} is gone"),
            };
            let _ = tx.send(Msg::Tail(id, text));
        });
    }

    fn spawn_stop(&mut self, app: &App, id: u32) {
        let workspace = Self::workspace(app);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let note = match forge_tools::stop_job(&workspace, id).await {
                Ok(note) | Err(note) => note,
            };
            let _ = tx.send(Msg::JobNote(note));
        });
        self.jobs_at = None;
    }

    fn spawn_jobs(&mut self, app: &App) {
        self.jobs_inflight = true;
        self.jobs_at = Some(Instant::now());
        let workspace = Self::workspace(app);
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let now = now_secs();
            let rows = forge_tools::job_views(&workspace)
                .into_iter()
                .map(|j| JobRow {
                    id: j.id,
                    command: j.command,
                    running: j.running,
                    status: j.status,
                    age_secs: now.saturating_sub(j.started_at),
                })
                .collect();
            let _ = tx.send(Msg::Jobs(rows));
        });
    }

    fn spawn_tree(&mut self, app: &App) {
        self.tree_inflight = true;
        let workspace = Self::workspace(app);
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(Msg::Tree(collect_tree(&workspace)));
        });
    }

    fn keep_strip_feed(&mut self, app: &mut App) {
        match (app.session_strip.enabled, self.strip_feed.is_some()) {
            (true, false) => {
                let tx = self.tx.clone();
                self.strip_feed = Some(tokio::spawn(fleet_feed(tx)));
            }
            (false, true) => {
                if let Some(task) = self.strip_feed.take() {
                    task.abort();
                }
            }
            _ => {}
        }
    }

    /// Drive the background reads and apply their results. True when the screen needs a redraw.
    pub(crate) fn tick(
        &mut self,
        app: &mut App,
        session: &std::sync::Arc<tokio::sync::Mutex<forge_core::Session>>,
    ) -> bool {
        let mut dirty = false;
        self.keep_strip_feed(app);
        if app.diff_overlay.open && app.diff_overlay.loading && !self.tree_inflight {
            self.spawn_tree(app);
        }
        if app.jobs_overlay.open
            && !self.jobs_inflight
            && self.jobs_at.is_none_or(|at| at.elapsed() >= JOBS_EVERY)
        {
            self.spawn_jobs(app);
        }
        if app.sidebar.enabled && self.mcp_at.is_none_or(|at| at.elapsed() >= MCP_EVERY) {
            self.mcp_at = Some(Instant::now());
            if let Ok(s) = session.try_lock() {
                let lines = s.mcp_status();
                if lines.len() != app.sidebar.mcp.len()
                    || lines
                        .iter()
                        .zip(&app.sidebar.mcp)
                        .any(|(l, d)| l.status != d.status || l.name != d.name)
                {
                    app.sidebar.set_mcp(&lines);
                    dirty = true;
                }
            }
        }
        while let Ok(msg) = self.rx.try_recv() {
            dirty = true;
            match msg {
                Msg::Fleet(Ok(rows)) => app.session_strip.set_fleet(&rows),
                Msg::Fleet(Err(_)) => {
                    app.session_strip.set_fleet(&[]);
                    app.session_strip.offline = true;
                }
                Msg::Tree(result) => {
                    self.tree_inflight = false;
                    app.diff_overlay.set_tree(result);
                }
                Msg::Jobs(rows) => {
                    self.jobs_inflight = false;
                    app.jobs_overlay.set_rows(rows);
                }
                Msg::Tail(id, text) => app.jobs_overlay.set_tail(id, &text),
                Msg::JobNote(note) => app.jobs_overlay.notice = Some(note),
            }
        }
        dirty
    }
}

impl Drop for PaneHost {
    fn drop(&mut self) {
        if let Some(task) = self.strip_feed.take() {
            task.abort();
        }
    }
}

/// Hand the terminal to `forge attach <id>` and take it back when that exits. The daemon token is
/// not passed: the child reads the same persisted token itself, and argv is world-readable. The
/// parent keeps its fleet heartbeat going while it waits so it does not age out of the strip.
fn attach(app: &mut App, tui: &mut Tui, id: &str, heartbeat: &mut dyn FnMut()) {
    if let Err(e) = crate::attach::resolve_token(None) {
        app.note(&format!("could not attach: {e}"));
        return;
    }
    let base = crate::attach::resolve_base_url(None);
    let exe = forge_types::self_exe::forge_executable();
    let id = id.to_string();
    let outcome = tui.run_external(|| {
        let mut child = Command::new(exe)
            .arg("attach")
            .arg(&id)
            .arg("--url")
            .arg(&base)
            .spawn()?;
        let mut last = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok::<_, std::io::Error>(status);
            }
            if last.elapsed() >= ATTACH_HEARTBEAT {
                heartbeat();
                last = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    match outcome {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => app.note(&format!("could not run `forge attach`: {e}")),
        Err(e) => app.note(&format!("terminal could not be restored: {e}")),
    }
}

/// Keep the strip's rows current: read the fleet, then read it again whenever `/ws/fleet` says
/// it moved (and every so often regardless, so a dropped socket cannot freeze the strip).
async fn fleet_feed(tx: mpsc::UnboundedSender<Msg>) {
    let http = reqwest::Client::new();
    let mut attempt = 0usize;
    loop {
        let base = crate::attach::resolve_base_url(None);
        let token = match crate::attach::resolve_token(None) {
            Ok(token) => token,
            Err(e) => {
                let _ = tx.send(Msg::Fleet(Err(e.to_string())));
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        let read = fetch_fleet(&http, &base, &token).await;
        let ok = read.is_ok();
        if tx
            .send(Msg::Fleet(read.map_err(|e| e.to_string())))
            .is_err()
        {
            return;
        }
        if !ok {
            let wait = FLEET_BACKOFF[attempt.min(FLEET_BACKOFF.len() - 1)];
            attempt += 1;
            tokio::time::sleep(Duration::from_secs(wait)).await;
            continue;
        }
        attempt = 0;
        let Ok(url) = ws_url(&base, &token, "ws/fleet") else {
            tokio::time::sleep(FLEET_REFRESH).await;
            continue;
        };
        let Ok((mut socket, _)) = tokio_tungstenite::connect_async(&url).await else {
            tokio::time::sleep(FLEET_REFRESH).await;
            continue;
        };
        // Wait for a change, or for the safety refresh. A burst of changes is one re-read.
        loop {
            match tokio::time::timeout(FLEET_REFRESH, socket.next()).await {
                Ok(Some(Ok(Message::Text(text)))) if is_fleet_changed(&text) => {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    break;
                }
                Ok(Some(Ok(Message::Close(_) | Message::Frame(_))))
                | Ok(None)
                | Ok(Some(Err(_))) => break,
                Ok(Some(Ok(_))) => {}
                Err(_) => break,
            }
        }
    }
}

fn git(workspace: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Every uncommitted change in the workspace's repository, tracked and untracked.
pub(crate) fn collect_tree(workspace: &Path) -> Result<Vec<OverlayFile>, String> {
    if git(workspace, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Err(format!(
            "{} is not inside a git repository",
            workspace.display()
        ));
    }
    let has_head = git(workspace, &["rev-parse", "--verify", "-q", "HEAD"]).is_some();
    let base: &[&str] = if has_head {
        &["diff", "HEAD", "--no-color", "--no-ext-diff", "-M"]
    } else {
        &["diff", "--cached", "--no-color", "--no-ext-diff", "-M"]
    };
    let mut files = parse_unified(&git(workspace, base).unwrap_or_default());
    // Paths from `ls-files` are relative to the directory git was asked about, not the root.
    let untracked = git(
        workspace,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .unwrap_or_default();
    for (n, path) in untracked.split('\0').filter(|p| !p.is_empty()).enumerate() {
        let full = workspace.join(path);
        let content = (n < UNTRACKED_MAX_FILES)
            .then(|| read_text(&full))
            .flatten();
        let mut file = untracked_file(path, content.as_deref());
        if content.is_none() {
            file.identity = stat_identity(&full);
        }
        files.push(file);
    }
    Ok(files)
}

fn stat_identity(path: &Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return String::new();
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{}:{mtime}", meta.len())
}

fn read_text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > UNTRACKED_MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_tui::diff_overlay::FileStatus;

    fn run(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn the_tree_has_tracked_changes_and_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        if !run(dir.path(), &["init", "-q"]) {
            return;
        }
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        assert!(run(dir.path(), &["add", "a.txt"]));
        assert!(run(dir.path(), &["commit", "-q", "-m", "x"]));
        std::fs::write(dir.path().join("a.txt"), "one\n2\n").unwrap();
        std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
        std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2]).unwrap();
        let files = collect_tree(dir.path()).unwrap();
        let a = files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((a.adds, a.dels), (1, 1));
        let n = files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!((n.status, n.adds), (FileStatus::Untracked, 1));
        assert!(files.iter().find(|f| f.path == "blob.bin").unwrap().binary);
    }

    #[test]
    fn outside_a_repository_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let err = collect_tree(dir.path()).unwrap_err();
        assert!(err.contains("not inside a git repository"), "{err}");
    }

    #[test]
    fn toggling_without_an_argument_flips() {
        assert_eq!(wanted(true, ""), Ok(false));
        assert_eq!(wanted(false, "toggle"), Ok(true));
        assert_eq!(wanted(false, "on"), Ok(true));
        assert_eq!(wanted(true, "off"), Ok(false));
        assert!(wanted(true, "sideways").is_err());
    }
}
