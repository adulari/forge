//! IDE mode's host: the daemon's HTTP routes and sockets, the terminal, and the persisted
//! reviewed marks. `forge_tui::ide::IdeApp` is pure (ADR-0004); this module performs the
//! [`IdeAction`]s it asks for and feeds the answers back as [`IdeEvent`]s.
//!
//! It reuses the board's client pieces: `api_url`, the token-scoped routes, the fleet
//! invalidation socket and one `/ws?session=` socket (only for the agent on screen). Before
//! connecting it makes sure a daemon is running, starting the background service (or a local
//! `forge serve`) the same way the rest of the CLI does.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use crossterm::event::{Event as TermEvent, KeyEventKind};
use forge_tui::board::{BoardEvent, FleetRow, HistoryRow};
use forge_tui::ide::hunk::{Bucket, UndoRecord};
use forge_tui::ide::wire::{GitStatus, ProjectChanges, WireDiff, WireDiffFile};
use forge_tui::ide::{
    handle_key, handle_mouse, handle_paste, DiffLoaded, HunkFailure, HunkOutcome, IdeAction,
    IdeApp, IdeEvent,
};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;

use crate::board::client::{api_url, error_message};
use crate::board::sockets::{fleet_watcher, SocketPool};
use crate::board::{now_unix, Ev, InputReader, TerminalGuard};

const TICK: Duration = Duration::from_millis(100);
const DRAW_MIN_INTERVAL: Duration = Duration::from_millis(33);
const DEBOUNCE: Duration = Duration::from_millis(300);
const FALLBACK_REFRESH: Duration = Duration::from_secs(15);
const HISTORY_EVERY: Duration = Duration::from_secs(4);
const HISTORY_LIMIT: usize = 80;
const MARKS_FILE: &str = "tui-ide-reviewed.json";
const PREFS_FILE: &str = "tui-ide-state.json";
const TERMINAL_ID: &str = "term-ide";
/// A check's output is followed for at most this long.
const CHECK_FOLLOW: Duration = Duration::from_secs(30 * 60);

// ---------------------------------------------------------------------------
// Daemon discovery
// ---------------------------------------------------------------------------

/// A daemon that answers `GET /api/sessions` with this token.
async fn probe(http: &reqwest::Client, base: &str, token: &str) -> bool {
    http.get(api_url(base, token, "api/sessions"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|resp| resp.status().is_success())
}

/// The base URL and token of a running daemon, starting one if none answers. An explicit
/// `--url` is trusted as given and never started on the user's behalf.
pub(crate) async fn ensure_daemon(
    url: Option<String>,
    token: Option<String>,
) -> Result<(String, String)> {
    let explicit = url.is_some();
    let base = crate::attach::resolve_base_url(url);
    let http = crate::attach::local_http_client(&base);
    if let Ok(token) = crate::attach::resolve_token(token.clone()) {
        if probe(&http, &base, &token).await {
            return Ok((base, token));
        }
    }
    if explicit || token.is_some() {
        bail!("no forge serve daemon answers at {base} with that token");
    }
    start_daemon()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
        // The token file only exists once the daemon has started once.
        if let Ok(token) = crate::attach::resolve_token(None) {
            if probe(&http, &base, &token).await {
                return Ok((base, token));
            }
        }
    }
    bail!("started the Forge daemon but it did not answer at {base}; try `forge service status`")
}

/// The installed background service when there is one (what `forge service start` runs), else a
/// detached `forge serve --local`, as `forge anywhere` starts it.
fn start_daemon() -> Result<()> {
    let installed = crate::cli::commands::service::query_service_status()
        .map(|status| status.installed)
        .unwrap_or(false);
    if installed {
        return crate::cli::commands::service::start_service();
    }
    let executable = std::env::current_exe().context("locate the forge executable")?;
    let mut command = std::process::Command::new(&executable);
    command
        .args(["serve", "--local"])
        .current_dir(crate::daemon_cwd::stable_daemon_cwd())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .spawn()
        .with_context(|| format!("start `{}` serve --local", executable.display()))?;
    Ok(())
}

mod perform;
mod streams;
mod tools_api;
use perform::perform;
use streams::{external_edit, stream_check, term_task, Runtime, TermCmd, TermLink};

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// A failed daemon call: the status (0 when it never answered), the daemon's `code` field and a
/// one-line reason.
#[derive(Debug)]
pub(crate) struct CallError {
    pub(crate) status: u16,
    pub(crate) code: String,
    pub(crate) message: String,
}

async fn finish<T: DeserializeOwned>(
    sent: Result<reqwest::Response, reqwest::Error>,
) -> Result<T, CallError> {
    let resp = sent.map_err(|e| CallError {
        status: 0,
        code: String::new(),
        message: format!("the daemon did not answer: {}", e.without_url()),
    })?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let code = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["code"].as_str().map(str::to_string))
            .unwrap_or_default();
        return Err(CallError {
            status: status.as_u16(),
            code,
            message: error_message(&body),
        });
    }
    resp.json::<T>().await.map_err(|e| CallError {
        status: status.as_u16(),
        code: String::new(),
        message: format!("unexpected reply: {e}"),
    })
}

async fn get<T: DeserializeOwned>(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<T, CallError> {
    finish(
        http.get(api_url(base, token, path))
            .query(query)
            .send()
            .await,
    )
    .await
}

async fn post(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, CallError> {
    finish(
        http.post(api_url(base, token, path))
            .json(&body)
            .send()
            .await,
    )
    .await
}

async fn put(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, CallError> {
    finish(
        http.put(api_url(base, token, path))
            .json(&body)
            .send()
            .await,
    )
    .await
}

#[derive(Clone)]
struct Api {
    http: reqwest::Client,
    base: String,
    token: String,
    cwd: String,
    ev: mpsc::UnboundedSender<Ev>,
}

impl Api {
    fn send(&self, event: IdeEvent) {
        let _ = self.ev.send(Ev::Ide(event));
    }

    async fn refresh(&self) {
        let query = [("cwd", self.cwd.as_str())];
        let (fleet, changes) = tokio::join!(
            get::<Vec<FleetRow>>(&self.http, &self.base, &self.token, "api/sessions", &[]),
            get::<ProjectChanges>(
                &self.http,
                &self.base,
                &self.token,
                "api/projects/changes",
                &query
            ),
        );
        if let Ok(rows) = fleet {
            self.send(IdeEvent::Fleet(rows));
        }
        self.send(IdeEvent::Changes(
            changes.map_err(|e| (matches!(e.status, 404 | 405), e.message)),
        ));
    }

    async fn status(&self, session: &str) {
        let result = get::<GitStatus>(
            &self.http,
            &self.base,
            &self.token,
            "api/git/status",
            &[("session", session)],
        )
        .await;
        self.send(IdeEvent::GitStatus(
            session.to_string(),
            result.map_err(|e| e.message),
        ));
    }

    async fn history(&self, session: &str) {
        let limit = HISTORY_LIMIT.to_string();
        if let Ok(rows) = get::<Vec<HistoryRow>>(
            &self.http,
            &self.base,
            &self.token,
            "api/history",
            &[
                ("session", session),
                ("limit", &limit),
                ("include_tools", "1"),
            ],
        )
        .await
        {
            self.send(IdeEvent::History(session.to_string(), rows));
        }
    }

    /// One file's diff and which git bucket it sits in. A worktree session's diff is its whole
    /// branch; a shared tree's is the unstaged side, falling back to the staged side.
    async fn diff(&self, session: &str, path: &str, worktree: bool) {
        let result = self.diff_inner(session, path, worktree).await;
        let (bucket, file) = match result {
            Ok(found) => (found.0, Ok(found.1)),
            Err(e) => (Bucket::Unstaged, Err(e.message)),
        };
        self.send(IdeEvent::Diff(DiffLoaded {
            session_id: session.to_string(),
            path: path.to_string(),
            bucket,
            file,
        }));
    }

    async fn diff_inner(
        &self,
        session: &str,
        path: &str,
        worktree: bool,
    ) -> Result<(Bucket, Option<WireDiffFile>), CallError> {
        let pick = |diff: WireDiff| {
            diff.files
                .iter()
                .position(|f| f.path == path)
                .map(|i| diff.files[i].clone())
                .or_else(|| diff.files.into_iter().next())
        };
        if worktree {
            let diff: WireDiff = get(
                &self.http,
                &self.base,
                &self.token,
                &format!("api/sessions/{session}/diff"),
                &[],
            )
            .await?;
            let file = diff.files.iter().find(|f| f.path == path).cloned();
            return Ok((Bucket::Worktree, file));
        }
        let status: GitStatus = get(
            &self.http,
            &self.base,
            &self.token,
            "api/git/status",
            &[("session", session)],
        )
        .await?;
        let has = |rows: &[forge_tui::ide::wire::GitRow]| rows.iter().any(|r| r.path == path);
        let staged = !has(&status.unstaged) && !has(&status.untracked) && has(&status.staged);
        let bucket = if staged {
            Bucket::Staged
        } else if has(&status.untracked) {
            Bucket::Untracked
        } else {
            Bucket::Unstaged
        };
        let diff: WireDiff = get(
            &self.http,
            &self.base,
            &self.token,
            "api/git/diff",
            &[
                ("session", session),
                ("path", path),
                ("staged", if staged { "true" } else { "false" }),
                ("context", "3"),
            ],
        )
        .await?;
        Ok((bucket, pick(diff)))
    }

    #[allow(clippy::too_many_arguments)]
    async fn hunk(
        &self,
        session: String,
        path: String,
        header: String,
        patch: String,
        action: forge_tui::ide::hunk::HunkAction,
        undo: Option<UndoRecord>,
        is_undo: bool,
    ) {
        let body = serde_json::json!({
            "session": session,
            "path": path,
            "hunk_header": header,
            "patch": patch,
            "action": action.wire(),
        });
        let result = post(&self.http, &self.base, &self.token, "api/git/hunk", body).await;
        self.send(IdeEvent::Hunk(HunkOutcome {
            session_id: session,
            path,
            action,
            undo: result.is_ok().then_some(undo).flatten(),
            is_undo,
            result: result.map(|_| ()).map_err(classify),
        }));
    }

    async fn discard(&self, session: String, path: String, undo: Option<UndoRecord>) {
        let body = serde_json::json!({ "session": session, "paths": [path] });
        let result = post(&self.http, &self.base, &self.token, "api/git/discard", body).await;
        self.send(IdeEvent::Hunk(HunkOutcome {
            session_id: session,
            path,
            action: forge_tui::ide::hunk::HunkAction::Revert,
            undo: result.is_ok().then_some(undo).flatten(),
            is_undo: false,
            result: result.map(|_| ()).map_err(classify),
        }));
    }

    async fn git_op(&self, route: &str, body: serde_json::Value, ok: String) {
        let result = post(&self.http, &self.base, &self.token, route, body).await;
        self.send(IdeEvent::GitOp(result.map(|_| ok).map_err(|e| e.message)));
    }
}

/// 409 `stale_patch` means the file moved on since the diff loaded: refetch rather than retry.
pub(crate) fn classify(error: CallError) -> HunkFailure {
    match (error.status, error.code.as_str()) {
        (409, "session_busy") => HunkFailure::Busy(error.message),
        (409, _) => HunkFailure::Stale("the file changed since this diff loaded: refreshed".into()),
        _ => HunkFailure::Error(error.message),
    }
}

// ---------------------------------------------------------------------------
// Reviewed marks
// ---------------------------------------------------------------------------

fn marks_path() -> Option<PathBuf> {
    forge_config::data_dir().map(|dir| dir.join(MARKS_FILE))
}

fn load_marks() -> forge_tui::ide::model::ReviewedMarks {
    marks_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|raw| forge_tui::ide::model::marks_from_json(&raw))
        .unwrap_or_default()
}

fn prefs_path() -> Option<PathBuf> {
    forge_config::data_dir().map(|dir| dir.join(PREFS_FILE))
}

fn load_prefs() -> bool {
    prefs_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|v| v["coach_seen"].as_bool().unwrap_or(false))
}

fn save_prefs(json: &str) {
    let Some(path) = prefs_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, json);
}

fn save_marks(json: &str) {
    let Some(path) = marks_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, json).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    }
}

// ---------------------------------------------------------------------------
// The host
// ---------------------------------------------------------------------------

/// IDE mode's state plus where to reach the daemon. Kept alive by the chat so reopening is
/// instant: the queue, the scroll position and the open file are still there.
pub(crate) struct IdeHost {
    app: IdeApp,
    base: String,
    token: String,
}

impl IdeHost {
    pub(crate) fn new(cwd: &str, base: String, token: String) -> Self {
        let mut app = IdeApp::new(cwd, now_unix());
        app.apply(IdeEvent::Reviewed(load_marks()));
        app.apply(IdeEvent::Prefs {
            coach_seen: load_prefs(),
        });
        Self { app, base, token }
    }

    pub(crate) fn cwd(&self) -> &str {
        self.app.cwd()
    }

    pub(crate) fn same_daemon(&self, base: &str, token: &str) -> bool {
        self.base == base && self.token == token
    }

    /// Run until the user closes IDE mode. Owns the terminal for the duration.
    pub(crate) async fn run(&mut self) -> Result<()> {
        let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Ev>();
        let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel::<()>();
        let api = Api {
            http: reqwest::Client::new(),
            base: self.base.clone(),
            token: self.token.clone(),
            cwd: self.app.cwd().to_string(),
            ev: ev_tx.clone(),
        };
        let input = InputReader::spawn(ev_tx.clone());
        let watcher = tokio::spawn(fleet_watcher(
            self.base.clone(),
            self.token.clone(),
            refresh_tx.clone(),
            ev_tx.clone(),
        ));
        let refresher = {
            let api = api.clone();
            tokio::spawn(async move {
                let mut fallback = tokio::time::interval(FALLBACK_REFRESH);
                fallback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        request = refresh_rx.recv() => {
                            if request.is_none() { return; }
                            let deadline = tokio::time::Instant::now() + DEBOUNCE;
                            loop {
                                tokio::select! {
                                    _ = tokio::time::sleep_until(deadline) => break,
                                    more = refresh_rx.recv() => if more.is_none() { return; },
                                }
                            }
                        }
                        _ = fallback.tick() => {}
                    }
                    api.refresh().await;
                    fallback.reset();
                }
            })
        };
        let mut pool = SocketPool::new(self.base.clone(), self.token.clone(), ev_tx.clone());
        let mut guard = TerminalGuard::enter()?;
        // The screen still holds whatever was there (the chat); the new terminal must repaint it all.
        guard.terminal.clear()?;
        if let Ok((w, h)) = crossterm::terminal::size() {
            self.app.apply(IdeEvent::Resize(w, h));
        }
        let _ = refresh_tx.send(());
        let mut pending: Vec<IdeAction> = self.app.sync_watch();
        pending.extend(self.app.reconnect_terminal());
        pending.push(IdeAction::RefreshChanges);
        let mut runtime = Runtime::default();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut history_at = Instant::now();
        let mut dirty = true;
        let mut last_draw = Instant::now() - DRAW_MIN_INTERVAL;
        let mut closing = false;
        let mut break_err: Option<anyhow::Error> = None;
        let result = loop {
            for action in std::mem::take(&mut pending) {
                match action {
                    IdeAction::Close => closing = true,
                    IdeAction::ExternalEdit { session_id, path } => {
                        input.pause().await;
                        guard.suspend();
                        external_edit(&api, &session_id, &path, &refresh_tx).await;
                        if let Err(e) = guard.resume() {
                            break_err = Some(e);
                        }
                        let _ = guard.terminal.clear();
                        input.resume();
                        dirty = true;
                    }
                    other => perform(other, &api, &mut pool, &refresh_tx, &mut runtime),
                }
            }
            if let Some(e) = break_err.take() {
                break Err(e);
            }
            if closing {
                break Ok(());
            }
            if dirty && last_draw.elapsed() >= DRAW_MIN_INTERVAL {
                let app = &mut self.app;
                if let Err(e) = guard
                    .terminal
                    .draw(|frame| forge_tui::ide::draw(app, frame))
                {
                    break Err(anyhow::Error::from(e));
                }
                dirty = false;
                last_draw = Instant::now();
            }
            tokio::select! {
                _ = tick.tick() => {
                    self.app.set_now(now_unix());
                    pending.extend(self.app.apply(IdeEvent::Tick));
                    dirty |= self.app.animating();
                    if history_at.elapsed() >= HISTORY_EVERY {
                        history_at = Instant::now();
                        if let Some(id) = self.app.panel_session() {
                            pending.push(IdeAction::FetchHistory(id));
                        }
                    }
                }
                event = ev_rx.recv() => {
                    let Some(event) = event else { break Ok(()) };
                    dirty = true;
                    match event {
                        Ev::Term(term) => pending.extend(self.term(term)),
                        Ev::Ide(event) => pending.extend(self.app.apply(event)),
                        Ev::Board(board) => {
                            if let Some(event) = translate(board) {
                                pending.extend(self.app.apply(event));
                            }
                        }
                        Ev::Created(..) => {}
                    }
                }
            }
        };
        pool.shutdown();
        runtime.shutdown();
        watcher.abort();
        refresher.abort();
        drop(input);
        guard.suspend();
        result
    }

    fn term(&mut self, event: TermEvent) -> Vec<IdeAction> {
        match event {
            TermEvent::Key(key) if key.kind != KeyEventKind::Release => {
                handle_key(&mut self.app, key)
            }
            TermEvent::Mouse(mouse) => handle_mouse(&mut self.app, mouse),
            TermEvent::Paste(text) => {
                handle_paste(&mut self.app, &text);
                Vec::new()
            }
            TermEvent::Resize(w, h) => self.app.apply(IdeEvent::Resize(w, h)),
            _ => Vec::new(),
        }
    }
}

fn translate(event: BoardEvent) -> Option<IdeEvent> {
    match event {
        BoardEvent::Snapshot(id, snap) => Some(IdeEvent::Snapshot(id, snap)),
        BoardEvent::SessionClosed(id) => Some(IdeEvent::SessionClosed(id)),
        BoardEvent::Connection(state) => Some(IdeEvent::Connection(state)),
        BoardEvent::Toast(level, text) => Some(IdeEvent::Toast(level, text)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

fn resolve_cwd(cwd: Option<String>) -> String {
    let path = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    path.canonicalize()
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// `forge ide [--cwd] [--url] [--token]`.
pub(crate) async fn ide_cmd(
    cwd: Option<String>,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    let (base, token) = ensure_daemon(url, token).await?;
    let mut host = IdeHost::new(&resolve_cwd(cwd), base, token);
    host.run().await
}

/// IDE mode on top of a board that has already handed over its terminal.
pub(crate) async fn open_over(cwd: &str, base: &str, token: &str) -> Result<()> {
    let mut host = IdeHost::new(
        &resolve_cwd(Some(cwd.to_string())),
        base.to_string(),
        token.to_string(),
    );
    host.run().await
}

/// IDE mode from the chat TUI: keeps `slot` alive between openings. The chat's own loop is paused
/// while this runs, so it is called inside `Tui::run_fullscreen`.
pub(crate) fn toggle_blocking(slot: &mut Option<IdeHost>, cwd: &str) -> Result<()> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| {
        handle.block_on(async {
            let (base, token) = ensure_daemon(None, None).await?;
            let stale = slot
                .as_ref()
                .is_some_and(|host| host.cwd() != cwd || !host.same_daemon(&base, &token));
            if stale || slot.is_none() {
                *slot = Some(IdeHost::new(cwd, base, token));
            }
            match slot.as_mut() {
                Some(host) => host.run().await,
                None => Ok(()),
            }
        })
    })
}

/// F2 and `/ide` in the chat: IDE mode takes the whole screen (inline chats leave their small
/// viewport for it), the chat `App` stays untouched underneath, and everything is back on exit.
pub(crate) fn open_from_chat(
    slot: &mut Option<IdeHost>,
    app: &mut forge_tui::App,
    tui: &mut forge_tui::Tui,
) {
    let cwd = resolve_cwd(
        app.workspace_path()
            .map(|path| path.to_string_lossy().into_owned()),
    );
    app.note("opening IDE mode…");
    tui.draw(app);
    match tui.run_fullscreen(|| toggle_blocking(slot, &cwd).map_err(std::io::Error::other)) {
        Ok(()) => {}
        Err(e) => app.note(&format!("IDE mode: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Query, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse as _;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use forge_tui::ide::hunk::HunkAction;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    const TOKEN: &str = "tok";

    #[derive(Default)]
    struct Seen {
        hunks: Mutex<Vec<serde_json::Value>>,
        stage: Mutex<Vec<serde_json::Value>>,
        stale: bool,
    }

    async fn serve(seen: Arc<Seen>) -> String {
        let app = Router::new()
            .route(&format!("/{TOKEN}/api/sessions"), get(|| async { Json(serde_json::json!([{"id":"a","title":"x","cwd":"/repo","last_activity":5,"model":"m::n"}])) }))
            .route(&format!("/{TOKEN}/api/projects/changes"), get(|Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q.get("cwd").map(String::as_str), Some("/repo"));
                Json(serde_json::json!({"cwd":"/repo","sessions":[{"session_id":"a","title":"x","branch":"main","waiting":false,"busy":false,"truncated":0,
                    "files":[{"path":"src/a.rs","kind":"modified","adds":1,"dels":1,"content_hash":"h1"}]}]}))
            }))
            .route(&format!("/{TOKEN}/api/git/status"), get(|| async { Json(serde_json::json!({"root":"/repo","branch":"main","staged":[],"unstaged":[{"path":"src/a.rs","status":"M","adds":1,"dels":1,"binary":false}],"untracked":[],"conflicted":[],"truncated":0,"ahead":0,"behind":0,"has_remote":false})) }))
            .route(&format!("/{TOKEN}/api/git/diff"), get(|Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q.get("staged").map(String::as_str), Some("false"));
                assert_eq!(q.get("path").map(String::as_str), Some("src/a.rs"));
                Json(serde_json::json!({"root":"/repo","staged":false,"files":[{"path":"src/a.rs","kind":"modified","binary":false,"adds":1,"dels":1,"skipped_lines":0,"offset":0,"total_lines":3,
                    "hunks":[{"header":"@@ -1,2 +1,2 @@","lines":[" k","-a","+b"]}]}]}))
            }))
            .route(&format!("/{TOKEN}/api/git/hunk"), post(|State(seen): State<Arc<Seen>>, Json(body): Json<serde_json::Value>| async move {
                seen.hunks.lock().unwrap().push(body);
                if seen.stale {
                    return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"stale","code":"stale_patch"}))).into_response();
                }
                Json(serde_json::json!({"ok":true})).into_response()
            }))
            .route(&format!("/{TOKEN}/api/git/stage"), post(|State(seen): State<Arc<Seen>>, Json(body): Json<serde_json::Value>| async move {
                seen.stage.lock().unwrap().push(body);
                Json(serde_json::json!({"ok":true}))
            }))
            .with_state(seen);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    fn api(base: String) -> (Api, mpsc::UnboundedReceiver<Ev>) {
        let (ev, rx) = mpsc::unbounded_channel();
        (
            Api {
                http: reqwest::Client::new(),
                base,
                token: TOKEN.into(),
                cwd: "/repo".into(),
                ev,
            },
            rx,
        )
    }

    async fn next(rx: &mut mpsc::UnboundedReceiver<Ev>) -> IdeEvent {
        loop {
            if let Some(Ev::Ide(event)) = rx.recv().await {
                return event;
            }
        }
    }

    #[tokio::test]
    async fn a_refresh_reads_the_fleet_and_the_projects_changes() {
        let (api, mut rx) = api(serve(Arc::default()).await);
        api.refresh().await;
        assert!(matches!(next(&mut rx).await, IdeEvent::Fleet(rows) if rows[0].id == "a"));
        match next(&mut rx).await {
            IdeEvent::Changes(Ok(changes)) => {
                assert_eq!(changes.sessions[0].files[0].content_hash, "h1");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_diff_asks_status_then_the_unstaged_diff_of_that_path() {
        let (api, mut rx) = api(serve(Arc::default()).await);
        api.diff("a", "src/a.rs", false).await;
        match next(&mut rx).await {
            IdeEvent::Diff(loaded) => {
                assert_eq!(loaded.bucket, Bucket::Unstaged);
                assert_eq!(loaded.file.unwrap().unwrap().hunks.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_hunk_posts_the_daemons_exact_shape() {
        let seen = Arc::new(Seen::default());
        let (api, mut rx) = api(serve(seen.clone()).await);
        api.hunk(
            "a".into(),
            "src/a.rs".into(),
            "@@ -1,2 +1,2 @@".into(),
            "p".into(),
            HunkAction::Stage,
            None,
            false,
        )
        .await;
        assert!(matches!(next(&mut rx).await, IdeEvent::Hunk(o) if o.result.is_ok()));
        let body = seen.hunks.lock().unwrap()[0].clone();
        assert_eq!(body["action"], "stage");
        assert_eq!(body["hunk_header"], "@@ -1,2 +1,2 @@");
        assert_eq!(body["session"], "a");
    }

    #[tokio::test]
    async fn a_409_stale_patch_reads_as_stale_and_keeps_no_undo() {
        let seen = Arc::new(Seen {
            stale: true,
            ..Default::default()
        });
        let (api, mut rx) = api(serve(seen).await);
        let undo = UndoRecord {
            session_id: "a".into(),
            path: "p".into(),
            label: "l".into(),
            steps: vec![],
        };
        api.hunk(
            "a".into(),
            "p".into(),
            "@@ -1 +1 @@".into(),
            "x".into(),
            HunkAction::Revert,
            Some(undo),
            false,
        )
        .await;
        match next(&mut rx).await {
            IdeEvent::Hunk(o) => {
                assert!(matches!(o.result, Err(HunkFailure::Stale(_))));
                assert!(o.undo.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn staging_posts_paths_and_reports_one_git_op() {
        let seen = Arc::new(Seen::default());
        let (api, mut rx) = api(serve(seen.clone()).await);
        api.git_op(
            "api/git/stage",
            serde_json::json!({"session":"a","paths":["x"]}),
            "staged x".into(),
        )
        .await;
        assert!(matches!(next(&mut rx).await, IdeEvent::GitOp(Ok(note)) if note == "staged x"));
        assert_eq!(seen.stage.lock().unwrap()[0]["paths"][0], "x");
    }

    #[tokio::test]
    async fn a_missing_route_is_distinguished_from_a_failure() {
        let (api, mut rx) = api(serve(Arc::default()).await);
        // Point at a path the stand-in does not serve.
        let err = super::get::<serde_json::Value>(&api.http, &api.base, TOKEN, "api/nope", &[])
            .await
            .unwrap_err();
        assert_eq!(err.status, 404);
        drop(rx.try_recv());
        assert!(matches!(
            classify(CallError {
                status: 409,
                code: "session_busy".into(),
                message: "m".into()
            }),
            HunkFailure::Busy(_)
        ));
    }

    #[tokio::test]
    async fn an_explicit_dead_url_is_an_error_not_a_daemon_start() {
        let err = ensure_daemon(Some("http://127.0.0.1:9".into()), Some("t".into()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no forge serve daemon"), "{err}");
    }

    #[test]
    fn marks_persist_under_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let json = forge_tui::ide::model::marks_to_json(
            &{
                let mut m = forge_tui::ide::model::ReviewedMarks::new();
                forge_tui::ide::model::mark_reviewed(&mut m, "s", "f", "h");
                m
            },
            10,
        );
        let path = dir.path().join(MARKS_FILE);
        std::fs::write(&path, &json).unwrap();
        let back = forge_tui::ide::model::marks_from_json(&std::fs::read_to_string(path).unwrap());
        assert_eq!(back.len(), 1);
    }
}

#[cfg(test)]
mod tools_tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::IntoResponse as _;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use forge_tui::ide::SaveFailure;
    use std::sync::{Arc, Mutex};

    const TOKEN: &str = "tok";

    #[derive(Default)]
    struct Seen {
        order: Mutex<Vec<String>>,
        puts: Mutex<Vec<serde_json::Value>>,
    }

    async fn serve(seen: Arc<Seen>) -> String {
        let note = |name: &'static str| {
            move |State(seen): State<Arc<Seen>>| async move {
                seen.order.lock().unwrap().push(name.to_string());
                Json(serde_json::json!({"ok": true, "message": "fix(core): tidy"}))
            }
        };
        let app = Router::new()
            .route(
                &format!("/{TOKEN}/api/workspace/file"),
                get(|| async {
                    Json(serde_json::json!({"content":"one\n","hash":"disk-now","read_only":false}))
                })
                .put(
                    |State(seen): State<Arc<Seen>>, Json(body): Json<serde_json::Value>| async move {
                        seen.puts.lock().unwrap().push(body.clone());
                        if body["expected_hash"] == "stale" {
                            return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"changed"})))
                                .into_response();
                        }
                        Json(serde_json::json!({"hash":"disk-new"})).into_response()
                    },
                ),
            )
            .route(&format!("/{TOKEN}/api/git/stage"), post(note("stage")))
            .route(&format!("/{TOKEN}/api/git/commit-message"), post(note("message")))
            .route(&format!("/{TOKEN}/api/git/commit"), post(note("commit")))
            .route(&format!("/{TOKEN}/api/git/push"), post(note("push")))
            .with_state(seen);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    fn api(base: String) -> (Api, mpsc::UnboundedReceiver<Ev>) {
        let (ev, rx) = mpsc::unbounded_channel();
        (
            Api {
                http: reqwest::Client::new(),
                base,
                token: TOKEN.into(),
                cwd: "/repo".into(),
                ev,
            },
            rx,
        )
    }

    async fn next(rx: &mut mpsc::UnboundedReceiver<Ev>) -> IdeEvent {
        loop {
            if let Some(Ev::Ide(event)) = rx.recv().await {
                return event;
            }
        }
    }

    #[tokio::test]
    async fn a_stale_hash_is_a_conflict_and_overwrite_reads_the_current_one() {
        let seen = Arc::new(Seen::default());
        let (api, mut rx) = api(serve(seen.clone()).await);
        api.save("a", "f.txt", "two\n".into(), Some("stale".into()))
            .await;
        match next(&mut rx).await {
            IdeEvent::Saved { result, .. } => assert_eq!(result, Err(SaveFailure::Conflict)),
            other => panic!("{other:?}"),
        }
        api.save("a", "f.txt", "two\n".into(), None).await;
        match next(&mut rx).await {
            IdeEvent::Saved { result, .. } => assert_eq!(result, Ok("disk-new".to_string())),
            other => panic!("{other:?}"),
        }
        let puts = seen.puts.lock().unwrap();
        assert_eq!(puts[1]["expected_hash"], "disk-now");
        assert_eq!(puts[1]["content"], "two\n");
    }

    #[tokio::test]
    async fn ship_stages_writes_a_message_commits_and_pushes_in_order() {
        let seen = Arc::new(Seen::default());
        let (api, mut rx) = api(serve(seen.clone()).await);
        api.ship("a".into(), vec!["x.rs".into()], None, true).await;
        match next(&mut rx).await {
            IdeEvent::GitOp(result) => assert_eq!(result, Ok("committed and pushed".to_string())),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            *seen.order.lock().unwrap(),
            ["stage", "message", "commit", "push"]
        );
    }

    #[tokio::test]
    async fn ship_with_a_message_and_nothing_to_stage_only_commits() {
        let seen = Arc::new(Seen::default());
        let (api, mut rx) = api(serve(seen.clone()).await);
        api.ship("a".into(), Vec::new(), Some("feat: x".into()), false)
            .await;
        assert!(matches!(next(&mut rx).await, IdeEvent::GitOp(Ok(_))));
        assert_eq!(*seen.order.lock().unwrap(), ["commit"]);
    }
}
