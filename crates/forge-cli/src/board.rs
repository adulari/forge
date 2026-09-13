//! `forge board` — the host half of the full-screen project board
//! (docs/features/project-board.md).
//!
//! The board's state and rendering live in `forge_tui::board` (renderer-independent, ADR-0004);
//! this module is everything that touches the outside world: the terminal, the daemon's HTTP
//! routes, its WebSockets, the clipboard, and the child `forge attach` process. It CONSUMES
//! exactly the daemon surface `forge attach` already uses — `GET /api/sessions`, the per-session
//! `/ws?session=<id>&rev=<n>` stream, and `RemoteInput` JSON back over it — plus the fleet
//! invalidation socket and the read-only detail routes. Auth is the same: the daemon token is the
//! leading path segment, a wrong token is a 404.
//!
//! The split is strict in one direction: [`forge_tui::board::BoardApp`] never performs an action,
//! it only asks for one ([`BoardAction`]), and every answer comes back as a
//! [`BoardEvent`]. Everything below is that translation layer.

mod actions;
mod client;
mod sockets;

#[cfg(test)]
mod http_tests;

use std::collections::HashMap;
use std::io::Stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event as TermEvent, KeyEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use forge_tui::board::{
    handle_key, handle_mouse, handle_paste, BoardAction, BoardApp, BoardEvent, ConnState, FleetRow,
    ToastLevel,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

/// Past sessions asked for on every refresh (the board's Done column shows recent work).
const PAST_LIMIT: usize = 30;
/// Rows of `GET /api/history` pulled for the detail pane's Tools/Live sections.
const HISTORY_LIMIT: usize = 80;
/// The animation clock. Everything time-relative on the board (spinners, ages, toast expiry)
/// advances on this.
const TICK: Duration = Duration::from_millis(100);
/// Hard ceiling on redraws (~15/s) so a burst of snapshot frames can't pin a core.
const DRAW_MIN_INTERVAL: Duration = Duration::from_millis(66);
/// How often the open detail pane's git status is re-read.
const GIT_REFRESH: Duration = Duration::from_secs(10);

/// Whether this process currently owns the alternate screen — read by the panic hook, which must
/// restore the terminal before the panic message prints or it lands on a screen that vanishes.
static IN_ALT_SCREEN: AtomicBool = AtomicBool::new(false);

/// Everything the render loop can be woken by.
// Inherited from `BoardEvent::Snapshot`, which carries a whole per-session frame. Boxing here
// would only move one allocation out of the channel and into the event, at ≤10 Hz.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Ev {
    /// News for the board's state machine.
    Board(BoardEvent),
    /// A raw terminal event, still to be interpreted by `forge_tui::board`'s key map.
    Term(TermEvent),
    /// `POST /api/sessions` created this session; its first prompt is waiting for a socket.
    Created(String, String),
}

pub(crate) async fn board_cmd(
    url: Option<String>,
    token: Option<String>,
    project: Option<String>,
    all: bool,
) -> Result<()> {
    let base = crate::attach::resolve_base_url(url);
    let token = crate::attach::resolve_token(token)?;
    let http = reqwest::Client::new();

    // Fetched BEFORE the alternate screen: a daemon that isn't running must print `forge attach`'s
    // friendly "is it running?" line to a normal terminal, not leave a blank full-screen board
    // behind an error the user never sees.
    let fleet = client::fetch_fleet(&http, &base, &token).await?;
    let past = client::fetch_past(&http, &base, &token, PAST_LIMIT)
        .await
        .unwrap_or_default();

    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string());

    let mut app = BoardApp::new(cwd.clone(), now_unix());
    app.apply(BoardEvent::Fleet(fleet.clone()));
    app.apply(BoardEvent::Past(past));
    match project {
        Some(name) => app.set_project_filter(Some(name)),
        None if !all => {
            if let Some(name) = preselect_project(&fleet, cwd.as_deref()) {
                app.set_project_filter(Some(name));
            }
        }
        None => {}
    }

    let mut guard = TerminalGuard::enter()?;
    let outcome = run_board(&mut app, &mut guard, &base, &token, &http).await;
    guard.suspend();
    println!("⚒ board closed — sessions keep running");
    outcome
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// The board opens filtered to the project the user is standing in, so `forge board` in a repo is
/// about that repo. Matching is on the last path component (the project name the cards show), and
/// only against LIVE rows — a directory whose only trace is an archived session shouldn't hide
/// everything else. Returns `None` when nothing running belongs to this directory, which leaves
/// the board unfiltered rather than empty.
pub(crate) fn preselect_project(rows: &[FleetRow], cwd: Option<&str>) -> Option<String> {
    let here = forge_tui::board::project_name(cwd?);
    if here.is_empty() {
        return None;
    }
    rows.iter()
        .any(|r| {
            forge_tui::board::project_name(&r.cwd) == here
                || r.worktree
                    .as_deref()
                    .is_some_and(|w| forge_tui::board::project_name(w) == here)
        })
        .then_some(here)
}

// ---------------------------------------------------------------------------
// Terminal ownership
// ---------------------------------------------------------------------------

/// Owns the raw-mode alternate screen for the board's lifetime. Every exit path — a clean quit, an
/// error, an unwind — goes through `Drop`, and [`suspend`](Self::suspend) hands the terminal back
/// mid-run so a child `forge attach` can have it.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    entered: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        install_panic_restore();
        enter_screen()?;
        let terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
        Ok(Self {
            terminal,
            entered: true,
        })
    }

    fn suspend(&mut self) {
        if std::mem::take(&mut self.entered) {
            leave_screen();
        }
    }

    fn resume(&mut self) -> Result<()> {
        if !self.entered {
            enter_screen()?;
            self.entered = true;
            self.terminal.clear()?;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.suspend();
    }
}

fn enter_screen() -> Result<()> {
    enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        EnableFocusChange,
        crossterm::cursor::Hide,
    )?;
    IN_ALT_SCREEN.store(true, Ordering::Relaxed);
    Ok(())
}

fn leave_screen() {
    IN_ALT_SCREEN.store(false, Ordering::Relaxed);
    let _ = crossterm::execute!(
        std::io::stdout(),
        DisableFocusChange,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show,
    );
    let _ = disable_raw_mode();
}

/// The chat TUI's hook restores raw mode and the cursor; it tracks ITS own alternate-screen flag,
/// which the board never sets, so this chains a second hook for the screen the board does own.
fn install_panic_restore() {
    forge_tui::install_panic_restore();
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if IN_ALT_SCREEN.load(Ordering::Relaxed) {
                leave_screen();
            }
            prev(info);
        }));
    });
}

// ---------------------------------------------------------------------------
// Terminal input
// ---------------------------------------------------------------------------

/// Crossterm's blocking reader on its own thread, forwarding into the render loop's channel.
///
/// A dedicated thread (rather than crossterm's event-stream feature) keeps the dependency surface
/// identical to the chat TUI's, and gives the one thing the board needs that a stream can't do
/// cheaply: [`pause`](Self::pause), which parks the reader so a child `forge attach` gets the
/// keyboard to itself instead of racing us for every keystroke.
struct InputReader {
    paused: Arc<AtomicBool>,
    parked: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl InputReader {
    fn spawn(tx: mpsc::UnboundedSender<Ev>) -> Self {
        let reader = Self {
            paused: Arc::new(AtomicBool::new(false)),
            parked: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (paused, parked, stop) = (
            reader.paused.clone(),
            reader.parked.clone(),
            reader.stop.clone(),
        );
        std::thread::spawn(move || loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if paused.load(Ordering::Relaxed) {
                parked.store(true, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            parked.store(false, Ordering::Relaxed);
            match crossterm::event::poll(Duration::from_millis(50)) {
                Ok(true) => match crossterm::event::read() {
                    // Re-check `paused`: the event may have arrived in the window between the
                    // pause request and this thread noticing it, and it belongs to the child.
                    Ok(ev) => {
                        if !paused.load(Ordering::Relaxed) && tx.send(Ev::Term(ev)).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        });
        reader
    }

    /// Park the reader and wait (briefly) for it to confirm, so the child process starts with a
    /// terminal nobody else is reading.
    async fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        for _ in 0..25 {
            if self.parked.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn resume(&self) {
        self.parked.store(false, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// The render loop
// ---------------------------------------------------------------------------

async fn run_board(
    app: &mut BoardApp,
    guard: &mut TerminalGuard,
    base: &str,
    token: &str,
    http: &reqwest::Client,
) -> Result<()> {
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Ev>();
    let (refresh_tx, refresh_rx) = mpsc::unbounded_channel::<()>();
    let input = InputReader::spawn(ev_tx.clone());

    let refresher = tokio::spawn(client::fleet_refresher(
        http.clone(),
        base.to_string(),
        token.to_string(),
        refresh_rx,
        ev_tx.clone(),
    ));
    let fleet_watcher = tokio::spawn(sockets::fleet_watcher(
        base.to_string(),
        token.to_string(),
        refresh_tx.clone(),
        ev_tx.clone(),
    ));

    let mut pool = sockets::SocketPool::new(base.to_string(), token.to_string(), ev_tx.clone());
    let mut pending_prompts: HashMap<String, String> = HashMap::new();
    let mut clipboard = arboard::Clipboard::new().ok();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut git_tick = tokio::time::interval(GIT_REFRESH);
    git_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut dirty = true;
    let mut last_draw = Instant::now() - DRAW_MIN_INTERVAL;

    pool.sync(&app.watch_ids());

    let result = loop {
        if dirty && last_draw.elapsed() >= DRAW_MIN_INTERVAL {
            if let Err(e) = guard.terminal.draw(|frame| app.draw(frame)) {
                break Err(anyhow::Error::from(e));
            }
            dirty = false;
            last_draw = Instant::now();
        }

        let mut actions: Vec<BoardAction> = Vec::new();
        tokio::select! {
            _ = tick.tick() => {
                let animating = app.needs_animation();
                app.apply(BoardEvent::Tick);
                dirty |= animating || app.needs_animation();
            }
            _ = git_tick.tick() => {
                if let Some(id) = app.detail_session() {
                    tokio::spawn(client::fetch_git(
                        http.clone(), base.to_string(), token.to_string(), id, ev_tx.clone(),
                    ));
                }
            }
            event = ev_rx.recv() => {
                let Some(event) = event else { break Ok(()) };
                dirty = true;
                match event {
                    Ev::Term(term) => actions = apply_term(app, term),
                    Ev::Created(id, prompt) => {
                        // The socket usually opens a moment later (the session shows up in the
                        // next fleet); if it is already there, the prompt goes now.
                        pending_prompts.insert(id, prompt);
                        send_pending(&mut pool, &mut pending_prompts);
                    }
                    Ev::Board(board) => {
                        let resync = matches!(
                            board,
                            BoardEvent::Fleet(_) | BoardEvent::SessionClosed(_)
                        );
                        app.apply(board);
                        if resync {
                            pool.sync(&app.watch_ids());
                            send_pending(&mut pool, &mut pending_prompts);
                        }
                    }
                }
            }
        }

        let mut quit = false;
        for action in actions {
            let mut host = actions::Host {
                http,
                base,
                token,
                ev: &ev_tx,
                refresh: &refresh_tx,
                pool: &mut pool,
                clipboard: &mut clipboard,
            };
            match actions::perform(action, &mut host) {
                actions::After::None => {}
                actions::After::Quit => quit = true,
                actions::After::Attach(id) => {
                    attach_child(guard, &input, base, token, &id, &ev_tx, &refresh_tx).await;
                    last_draw = Instant::now() - DRAW_MIN_INTERVAL;
                }
            }
        }
        if quit {
            break Ok(());
        }
    };

    pool.shutdown();
    refresher.abort();
    fleet_watcher.abort();
    result
}

fn apply_term(app: &mut BoardApp, event: TermEvent) -> Vec<BoardAction> {
    match event {
        // Terminals with the kitty protocol report releases too; only presses/repeats are input.
        TermEvent::Key(key) if key.kind != KeyEventKind::Release => handle_key(app, key),
        TermEvent::Mouse(mouse) => handle_mouse(app, mouse),
        TermEvent::Paste(text) => {
            handle_paste(app, &text);
            Vec::new()
        }
        TermEvent::Resize(w, h) => {
            app.apply(BoardEvent::Resize(w, h));
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Hand every queued first prompt to its session's socket, once it exists.
fn send_pending(pool: &mut sockets::SocketPool, pending: &mut HashMap<String, String>) {
    for (id, json) in pending_prompt_frames(pending, &pool.open_ids()) {
        pool.send(&id, &json);
    }
}

/// A brand-new session's first prompt can't be sent with the `POST` that created it — the daemon
/// takes the prompt over the session's own socket, which doesn't exist yet. This drains the ones
/// whose socket has since opened, exactly once each: a prompt is removed from `pending` as its
/// frame is produced, so a later fleet refresh can't re-send it.
pub(crate) fn pending_prompt_frames(
    pending: &mut HashMap<String, String>,
    open: &[String],
) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    for id in open {
        if let Some(text) = pending.remove(id) {
            out.push((
                id.clone(),
                serde_json::json!({ "kind": "prompt", "text": text }),
            ));
        }
    }
    out
}

/// Leave the board, run `forge attach` on this session, come back.
///
/// `forge attach` rather than `forge chat --resume`: `chat --resume` opens a SECOND driver against
/// the same session row in the same store (`resolve_resume_mode` never consults the daemon), so
/// two writers would allocate message sequence numbers independently and interleave the
/// transcript. `attach` is the daemon's own thin client — one writer, over the socket the board is
/// already watching.
async fn attach_child(
    guard: &mut TerminalGuard,
    input: &InputReader,
    base: &str,
    token: &str,
    id: &str,
    ev: &mpsc::UnboundedSender<Ev>,
    refresh: &mpsc::UnboundedSender<()>,
) {
    input.pause().await;
    guard.suspend();
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("forge"));
    let spawned = tokio::process::Command::new(exe)
        .arg("attach")
        .arg(id)
        .arg("--url")
        .arg(base)
        .arg("--token")
        .arg(token)
        .status()
        .await;
    let failure = spawned.err().map(|e| e.to_string());
    let restored = guard.resume();
    input.resume();
    if let Some(e) = failure {
        let _ = ev.send(Ev::Board(BoardEvent::Toast(
            ToastLevel::Error,
            format!("could not run `forge attach`: {e}"),
        )));
    }
    if let Err(e) = restored {
        let _ = ev.send(Ev::Board(BoardEvent::Toast(
            ToastLevel::Error,
            format!("terminal could not be restored: {e}"),
        )));
    }
    let _ = refresh.send(());
}

/// The first 8 characters of a session id — how the board names one in a message.
pub(crate) fn short_id(id: &str) -> &str {
    let end = id.char_indices().nth(8).map_or(id.len(), |(byte, _)| byte);
    &id[..end]
}

/// Connectivity the host reports, so a change is only announced when it IS a change.
pub(crate) fn set_conn(
    ev: &mpsc::UnboundedSender<Ev>,
    current: &mut Option<ConnState>,
    next: ConnState,
) {
    if current.as_ref() == Some(&next) {
        return;
    }
    *current = Some(next.clone());
    let _ = ev.send(Ev::Board(BoardEvent::Connection(next)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, cwd: &str, worktree: Option<&str>) -> FleetRow {
        FleetRow {
            id: id.into(),
            cwd: cwd.into(),
            worktree: worktree.map(str::to_string),
            ..FleetRow::default()
        }
    }

    #[test]
    fn the_board_opens_filtered_to_the_project_the_user_is_standing_in() {
        let rows = vec![
            row("a", "/home/me/code/forge", None),
            row("b", "/tmp/other", None),
        ];
        assert_eq!(
            preselect_project(&rows, Some("/home/me/code/forge")),
            Some("forge".to_string())
        );
    }

    #[test]
    fn a_sessions_worktree_still_counts_as_that_project() {
        // A worktree session's cwd is the worktree, but the board is being run from the repo.
        let rows = vec![row("a", "/tmp/wt-9f2", Some("/home/me/code/forge"))];
        assert_eq!(
            preselect_project(&rows, Some("/home/me/code/forge")),
            Some("forge".to_string())
        );
    }

    #[test]
    fn nothing_running_here_leaves_the_board_unfiltered() {
        let rows = vec![row("a", "/srv/elsewhere", None)];
        assert_eq!(preselect_project(&rows, Some("/home/me/code/forge")), None);
        assert_eq!(preselect_project(&rows, None), None);
        // A past-only project must not preselect either: `rows` is live sessions only.
        assert_eq!(preselect_project(&[], Some("/home/me/code/forge")), None);
    }

    #[test]
    fn a_first_prompt_is_handed_over_exactly_once() {
        let mut pending = HashMap::from([
            ("new-1".to_string(), "fix the parser".to_string()),
            ("new-2".to_string(), "later".to_string()),
        ]);
        let frames = pending_prompt_frames(&mut pending, &["new-1".to_string()]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "new-1");
        assert_eq!(frames[0].1["kind"], "prompt");
        assert_eq!(frames[0].1["text"], "fix the parser");
        // The second fleet refresh must not re-send it.
        assert!(pending_prompt_frames(&mut pending, &["new-1".to_string()]).is_empty());
        assert_eq!(pending.len(), 1, "the other session is still waiting");
    }

    /// The board's first prompt has to deserialize into the daemon's OWN input enum — the same
    /// load-bearing contract `forge attach` pins down for its typed lines.
    #[test]
    fn the_first_prompt_frame_is_a_real_remote_input() {
        let mut pending = HashMap::from([("s".to_string(), "go".to_string())]);
        let frames = pending_prompt_frames(&mut pending, &["s".to_string()]);
        assert_eq!(
            serde_json::from_value::<crate::remote::RemoteInput>(frames[0].1.clone()).unwrap(),
            crate::remote::RemoteInput::Prompt {
                text: "go".into(),
                attachments: Vec::new(),
            }
        );
    }

    #[test]
    fn short_id_never_splits_a_character() {
        assert_eq!(short_id("0123456789abcdef"), "01234567");
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id("ünïcødé-session"), "ünïcødé-");
    }

    #[tokio::test]
    async fn connection_state_is_only_announced_when_it_changes() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut current = None;
        set_conn(&tx, &mut current, ConnState::Live);
        set_conn(&tx, &mut current, ConnState::Live);
        set_conn(&tx, &mut current, ConnState::Reconnecting);
        drop(tx);

        let mut seen = Vec::new();
        while let Some(Ev::Board(BoardEvent::Connection(state))) = rx.recv().await {
            seen.push(state);
        }
        assert_eq!(seen, vec![ConnState::Live, ConnState::Reconnecting]);
    }
}
