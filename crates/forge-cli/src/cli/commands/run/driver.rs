//! The headless session driver behind `forge serve` (docs/features/remote-control.md).
//!
//! [`spawn_session_driver`] runs ONE session as a plain tokio task: the same `App` + turn
//! machinery + remote-input handling `run_chat_tui` uses, with **no terminal attached** — the
//! output sink is the remote snapshot channel (`watch<Snapshot>` + the reconnect [`remote::
//! EventLog`]), and the only input is the [`remote::RemoteInput`] queue a browser feeds over the
//! WebSocket. Everything a remote client can drive goes through the SAME shared primitives the
//! TUI path uses — [`dispatch_command`] (with no `Tui`), [`picker_accept`], [`apply_overlay_
//! input`], the `spawn_turn*` family, [`build_snapshot_frame`] — so a command dispatched from the
//! phone produces the identical `DispatchOutcome` handling in both worlds.
//!
//! Sessions driven this way keep running with ZERO clients connected: the driver task never
//! blocks on a client, and a reconnecting page replays what it missed from the event log
//! (`?rev=` handshake, Phase 3). That is the core property that beats a one-session-per-process
//! remote: close the phone, reopen it an hour later, and the turn that kept running is all there.

use super::*;

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use forge_tui::{handle_key, App, ChannelPresenter, ConfirmOutcome, InputOutcome, KeyKind, UiMsg};

mod daemon_fleet;
mod input;
mod submit;

/// What to run: the parameters of one daemon-hosted session.
pub(crate) struct DriverSpec {
    /// Immutable session workspace. It is canonicalized before the driver is spawned and
    /// is persisted on the Session; daemon cwd is never used for tool execution.
    pub cwd: String,
    /// The isolated worktree the session runs in, if it was created with `worktree: true`.
    /// Informational here (the `cwd` already points inside it) — persisted + broadcast.
    pub worktree: Option<String>,
    /// Display title ("" = unnamed; the page falls back to the id).
    pub title: String,
    /// Offline deterministic mock provider (testing).
    pub mock: bool,
    /// Pin a model id, bypassing mesh classification.
    pub model: Option<String>,
    /// Resume an existing session id instead of starting fresh.
    pub resume: Option<String>,
    /// Start (or switch a resumed session into) this temper/permission-mode instead of whatever
    /// it already has — the API equivalent of picking a row in the `/mode` picker
    /// (`forge_tui::PickerKind::Tempers`). `None` leaves the temper untouched.
    pub temper: Option<forge_types::PermissionMode>,
    /// The daemon's Web Push sender (`None` = push disabled). The driver fires it on
    /// notification-worthy snapshot transitions ([`crate::push::detect_trigger`]) — but only
    /// while zero WS clients are attached ([`crate::push::should_push`]).
    pub push: Option<std::sync::Arc<crate::push::PushNotifier>>,
    /// The daemon's native (APNs) sender (`None` = native push disabled). Fired alongside `push`
    /// on the same notification-worthy transitions, plus a Live Activity content-state update at
    /// the same moments (see the dispatch site in `drive_session`).
    pub apns: Option<std::sync::Arc<crate::apns::ApnsNotifier>>,
    /// The daemon's live fleet (`None` outside `forge serve`, e.g. tests that don't exercise
    /// fleet messaging). When present, wires `message_session` on the constructed session —
    /// see [`daemon_fleet::DaemonFleetMessaging`].
    pub registry: Option<std::sync::Arc<crate::serve::SessionRegistry>>,
}

/// The daemon-side handle to a running session driver — everything `forge serve`'s HTTP layer
/// needs: the live snapshot stream (+ replay log) to serve WS clients, the input queue to feed
/// them, and identity/metadata for `GET /api/sessions`. Mirrors `mcp_serve`'s
/// LocalSessionManager shape: one task per session, addressed by id.
pub(crate) struct SessionDriverHandle {
    pub session_id: String,
    title: std::sync::Arc<std::sync::RwLock<String>>,
    pub cwd: String,
    pub worktree: Option<String>,
    pub created_at: i64,
    /// Latest broadcast snapshot (busy/cost/model ride in it — the session list reads these).
    pub snapshot_rx: tokio::sync::watch::Receiver<std::sync::Arc<remote::SnapshotFrame>>,
    /// Reconnect replay log (`?rev=` handshake), same shape as the in-TUI server's.
    pub events: std::sync::Arc<std::sync::Mutex<remote::EventLog>>,
    /// Feed remote inputs to the driver (the WS receive half pushes here).
    pub input_tx: tokio::sync::mpsc::Sender<remote::RemoteInput>,
    /// Unix seconds of the last broadcast state change — "last activity" in the session list.
    pub last_activity: std::sync::Arc<AtomicI64>,
    /// How many WebSocket clients are currently attached (the daemon's WS route holds a guard
    /// per connection). The push debounce: any client connected ⇒ no push.
    pub ws_clients: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SessionDriverHandle {
    pub(crate) fn title(&self) -> String {
        self.title
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set_title(&self, title: String) {
        *self
            .title
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = title;
    }

    /// Ask the driver to stop (archive): the loop aborts any running turn, runs SessionEnd
    /// hooks, broadcasts one final `closed` frame, and exits. Idempotent.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    pub fn is_finished(&self) -> bool {
        self.task
            .lock()
            .expect("driver task lock poisoned")
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
    }

    /// Wait (bounded) for the driver task to finish after [`Self::shutdown`].
    pub async fn join(&self, timeout: std::time::Duration) {
        let task = self.task.lock().expect("driver task lock poisoned").take();
        if let Some(mut task) = task {
            if tokio::time::timeout(timeout, &mut task).await.is_err() {
                // Dropping a JoinHandle detaches rather than cancels. Abort explicitly so a stuck
                // driver cannot retain its Session, App, replay ring, and any active turn forever.
                task.abort();
                let _ = task.await;
            }
        }
    }
}

impl Drop for SessionDriverHandle {
    fn drop(&mut self) {
        if let Some(task) = self
            .task
            .get_mut()
            .expect("driver task lock poisoned")
            .take()
        {
            task.abort();
        }
    }
}

/// Build the session and spawn its driver task. Returns once the session exists (id known) —
/// the driver keeps running until [`SessionDriverHandle::shutdown`].
pub(crate) async fn spawn_session_driver(spec: DriverSpec) -> Result<SessionDriverHandle> {
    let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiMsg>();
    let session = build_session_with_self_mcp(
        Box::new(ChannelPresenter::new(ui_tx)),
        spec.mock,
        None,
        spec.resume.clone(),
        spec.model.clone(),
        true,
        true,
        Some(&spec.cwd),
    )
    .await?;
    let session_id = session.session_id().to_string();

    // Persist identity: title + worktree land on the session row (schema v8) so they survive
    // daemon restarts and show up in `forge sessions`.
    if !spec.title.is_empty() {
        let _ = session.store.set_session_title(&session_id, &spec.title);
    }
    if let Some(wt) = &spec.worktree {
        let _ = session.store.set_session_worktree(&session_id, wt);
    }

    let mut session = session;
    // API-requested starting temper (`POST /api/sessions {"temper": ...}`) — reuses the exact
    // setter `picker_accept` calls for `PickerKind::Tempers` (including the best-effort
    // persist-as-next-default), so a session created this way starts exactly where picking that
    // row in the `/mode` picker would have left it. Full is included: picker-level availability
    // is the bar, and the request-level parse already rejected anything else before we got here.
    if let Some(mode) = spec.temper {
        session.set_temper(mode);
        let _ = forge_config::write_permission_mode(mode);
    }
    // The workspace is validated during session construction. Keep the legacy setter only
    // as a compatibility no-op; every session now roots tools unconditionally.
    if spec.resume.is_none() {
        let _ = session.prime_guidance(&[format!(
            "This session's working directory is {} — resolve every relative path there and \
                 pass it as `cwd` to shell commands.",
            spec.cwd
        )]);
    }

    // A worktree-backed daemon session is an isolated BUILD session — the client spun up a
    // dedicated git worktree specifically to make changes. Arm the completion-quality guards:
    // the empty-diff nudge ("implement it, don't describe it") and the progress-gated re-drive
    // only fire when the session `expect_code_change`. Without this, a serve/app session that ran
    // tools but edited nothing — a weaker model that investigated then stopped, or a bridge that
    // hallucinated a completion — was silently accepted as "done" (the biggest serve reliability
    // gap: every completion guard Forge already built was inert outside `bench swe`). The nudge
    // still only triggers when tools actually ran and the tree is unchanged, so a pure-answer turn
    // that touches nothing is unaffected.
    if spec.worktree.is_some() {
        session.set_expect_code_change(true);
    }

    if let Some(registry) = spec.registry.clone() {
        session.set_fleet_messaging(Some(std::sync::Arc::new(
            daemon_fleet::DaemonFleetMessaging {
                registry,
                store: session.store.clone(),
                self_id: session_id.clone(),
            },
        )));
    }

    let session = std::sync::Arc::new(tokio::sync::Mutex::new(session));
    let (snapshot_tx, snapshot_rx) = tokio::sync::watch::channel(std::sync::Arc::new(
        remote::SnapshotFrame::new(remote::Snapshot::default()),
    ));
    let (input_tx, input_rx) = tokio::sync::mpsc::channel::<remote::RemoteInput>(64);
    let events = std::sync::Arc::new(std::sync::Mutex::new(remote::EventLog::new(
        remote::EVENT_LOG_CAP,
    )));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let last_activity = std::sync::Arc::new(AtomicI64::new(now_secs()));
    let ws_clients = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let title = std::sync::Arc::new(std::sync::RwLock::new(spec.title));

    let task = tokio::spawn(drive_session(
        session,
        session_id.clone(),
        title.clone(),
        spec.cwd.clone(),
        spec.worktree.clone(),
        ui_rx,
        input_rx,
        snapshot_tx,
        events.clone(),
        shutdown_rx,
        last_activity.clone(),
        spec.push,
        spec.apns,
        ws_clients.clone(),
    ));

    Ok(SessionDriverHandle {
        session_id,
        title,
        cwd: spec.cwd,
        worktree: spec.worktree,
        created_at: now_secs(),
        snapshot_rx,
        events,
        input_tx,
        last_activity,
        ws_clients,
        shutdown_tx,
        task: std::sync::Mutex::new(Some(task)),
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// All mutable state of one headless driver loop — the same locals `run_chat_tui` keeps on its
/// stack, gathered so the input/outcome/key handlers can be real methods instead of a single
/// 2000-line loop body.
struct DriverState {
    session: std::sync::Arc<tokio::sync::Mutex<Session>>,
    app: App,
    catalog: std::sync::Arc<forge_skills::Catalog>,
    armed_project: std::collections::HashSet<String>,
    trust_project: bool,
    done_tx: std::sync::mpsc::Sender<u64>,
    busy: bool,
    busy_since: Instant,
    turn_gen: u64,
    last_auto_compact_gen: u64,
    turn_handle: Option<tokio::task::JoinHandle<()>>,
    loop_state: Option<LoopState>,
    goal_state: Option<GoalState>,
    pending: Option<(String, std::sync::mpsc::Sender<ConfirmOutcome>)>,
    pending_question: Option<std::sync::mpsc::Sender<String>>,
    pending_duel: Arc<std::sync::Mutex<PendingDuel>>,
    duel_state: PendingDuel,
    assay_lenses: Vec<forge_types::FindingCategory>,
    assay_scope: forge_types::AssayScope,
    queued_prompts: Vec<String>,
    prompt_history: Vec<String>,
    last_prompt: Option<String>,
    prompt_seq: u64,
    notes: Vec<String>,
    copy_text: Option<String>,
    /// Uploaded text files (`POST /api/upload`) waiting to ride the next prompt as `@path`
    /// mentions — images go straight to `Session::attach_images` at Attach time instead.
    pending_mentions: Vec<String>,
    remote_keys: std::collections::VecDeque<KeyKind>,
    mesh_load_rx: Option<tokio::sync::oneshot::Receiver<Option<forge_tui::MeshOverlay>>>,
    usage_load_rx: Option<tokio::sync::oneshot::Receiver<bridge_stats::BridgeStats>>,
    cwd: String,
    /// Rate-limits the idle heartbeat check (see [`HEARTBEAT_CHECK_INTERVAL`]) — this loop ticks
    /// every ~30ms, far too often to query the store on every iteration.
    last_heartbeat_check: Instant,
}

/// How often the driver loop checks for a due session heartbeat while idle. Turn-end already
/// checks immediately (`on_turn_done`); this coarse tick is only for a session that sits fully
/// idle for a while, no new activity, with nothing else waking the loop up.
const HEARTBEAT_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

impl Drop for DriverState {
    fn drop(&mut self) {
        if let Some(turn) = self.turn_handle.take() {
            turn.abort();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn drive_session(
    session: std::sync::Arc<tokio::sync::Mutex<Session>>,
    session_id: String,
    title: std::sync::Arc<std::sync::RwLock<String>>,
    cwd: String,
    worktree: Option<String>,
    ui_rx: std::sync::mpsc::Receiver<UiMsg>,
    mut input_rx: tokio::sync::mpsc::Receiver<remote::RemoteInput>,
    snapshot_tx: tokio::sync::watch::Sender<std::sync::Arc<remote::SnapshotFrame>>,
    events: std::sync::Arc<std::sync::Mutex<remote::EventLog>>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    last_activity: std::sync::Arc<AtomicI64>,
    push: Option<std::sync::Arc<crate::push::PushNotifier>>,
    apns: Option<std::sync::Arc<crate::apns::ApnsNotifier>>,
    ws_clients: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let (done_tx, done_rx) = std::sync::mpsc::channel::<u64>();
    let mut app = App::default();
    app.transcript_follow = true;
    {
        let s = session.lock().await;
        app.temper = s.temper().label().to_string();
        app.effort = s.pinned_effort();
    }
    // Populate the palette from the session's skill catalog so `/help` + command completion
    // work from the page exactly as in the TUI.
    let catalog: std::sync::Arc<forge_skills::Catalog> = {
        let s = session.lock().await;
        s.skills().cloned().unwrap_or_else(|| {
            std::sync::Arc::new(forge_skills::Catalog::load(&forge_config::command_sources()))
        })
    };
    app.palette.extra = catalog
        .entries()
        .iter()
        .map(|e| forge_tui::PaletteEntry {
            name: e.name.clone(),
            desc: if e.is_skill {
                format!("{}  (skill)", e.description)
            } else {
                e.description.clone()
            },
            usage: String::new(),
        })
        .collect();
    let trust_project = session.lock().await.commands_trust_project();
    {
        let (hooks, workspace) = {
            let s = session.lock().await;
            (s.hooks().to_vec(), s.workspace_root().to_path_buf())
        };
        forge_core::hooks::run_session_hooks_in(
            &hooks,
            forge_config::HookEvent::SessionStart,
            &session_id,
            Some(&workspace),
        )
        .await;
    }
    // Resumed session: rebuild the transcript ring so the first snapshot isn't empty.
    {
        let s = session.lock().await;
        let items = s.replay_items_full();
        if !items.is_empty() {
            app.replay_history(&items);
        }
    }

    let auto_setup = forge_config::load()
        .map(|config| config.project.auto_initialize)
        .unwrap_or(false)
        && !forge_config::project_initialization(std::path::Path::new(&cwd)).initialized
        && !forge_config::project_auto_setup_attempted(std::path::Path::new(&cwd));

    let mut st = DriverState {
        session,
        app,
        catalog,
        armed_project: std::collections::HashSet::new(),
        trust_project,
        done_tx,
        busy: false,
        busy_since: Instant::now(),
        turn_gen: 0,
        last_auto_compact_gen: 0,
        turn_handle: None,
        loop_state: None,
        goal_state: None,
        pending: None,
        pending_question: None,
        pending_duel: Arc::new(std::sync::Mutex::new(None)),
        duel_state: None,
        assay_lenses: Vec::new(),
        assay_scope: forge_types::AssayScope::Repo,
        queued_prompts: Vec::new(),
        prompt_history: Vec::new(),
        last_prompt: None,
        prompt_seq: 0,
        notes: Vec::new(),
        copy_text: None,
        pending_mentions: Vec::new(),
        remote_keys: std::collections::VecDeque::new(),
        mesh_load_rx: None,
        usage_load_rx: None,
        cwd: cwd.clone(),
        last_heartbeat_check: Instant::now(),
    };

    let mut last_snap: Option<remote::Snapshot> = None;
    let mut revision: u64 = 0;
    let mut dirty = true;
    // The most recent genuine turn failure (PresenterEvent::Error), latched so the busy falling
    // edge pushes "failed" instead of "done". Cleared when the next turn starts.
    let mut turn_error: Option<String> = None;

    if auto_setup {
        let _ = forge_config::mark_project_auto_setup_attempted(std::path::Path::new(&cwd));
        st.app
            .note("⚙ Setting up Forge for this project automatically…");
        st.handle_outcome(project_setup_outcome());
    }

    loop {
        if *shutdown_rx.borrow_and_update() {
            break;
        }
        let current_title = title
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if last_snap
            .as_ref()
            .is_some_and(|snapshot| snapshot.title != current_title)
        {
            dirty = true;
        }
        // 1. Presenter events from the (possibly running) turn task.
        while let Ok(msg) = ui_rx.try_recv() {
            dirty = true;
            match msg {
                UiMsg::Event(e) => {
                    if let forge_tui::PresenterEvent::Error(m) = &e {
                        turn_error = Some(m.clone());
                        // A turn-ending error only reached `view.transcript` (scrollback) before
                        // this — never `Snapshot::notes`, the remote toast/banner mechanism the
                        // mobile app renders (see the doc comment on `mobile/src/app/session/
                        // [id]/_layout.tsx` re: `snapshot.notes`). `busy` already clears correctly
                        // via `on_turn_done` regardless of this event, so the gap was purely a
                        // missing user-visible signal, not a stuck turn.
                        push_remote_note(&mut st.notes, &format!("⚠ {m}"));
                    }
                    st.app.apply(e)
                }
                UiMsg::Permission {
                    tool,
                    side_effect,
                    reply,
                } => {
                    st.app.prompt = Some(format!("allow {tool} ({side_effect:?}) [y/n]"));
                    st.pending = Some((tool, reply));
                    // New prompt, new identity: stale remote answers must never resolve it.
                    st.prompt_seq += 1;
                }
                UiMsg::Question {
                    question,
                    options,
                    allow_other,
                    reply,
                } => {
                    st.app.set_question(&question, &options, allow_other);
                    st.pending_question = Some(reply);
                    st.prompt_seq += 1;
                }
            }
        }
        // 2. Remote inputs (prompts / answers / keys / overlay verbs).
        while let Ok(input) = input_rx.try_recv() {
            dirty = true;
            if let Err(e) = st.handle_input(input).await {
                st.app.note(&format!("⚠ {e}"));
            }
        }
        // 3. Keys queued by the drain (named keys + synthesized overlay commits) through the
        //    headless modal router — same precedence as the TUI key loop.
        while let Some(key) = st.remote_keys.pop_front() {
            dirty = true;
            if let Err(e) = st.process_key(key).await {
                st.app.note(&format!("⚠ {e}"));
            }
        }
        // 4. Turn-complete signals: queued prompts, /loop continuation, /duel picker, auto-compact.
        while let Ok(g) = done_rx.try_recv() {
            dirty = true;
            st.on_turn_done(g).await;
        }
        // 5. Background overlay loads (/mesh, /usage).
        if st.poll_overlay_loads() {
            dirty = true;
        }
        // 5b. Session heartbeats: `on_turn_done` already checks immediately when a turn ends; this
        // coarse periodic check (see [`HEARTBEAT_CHECK_INTERVAL`]) catches a heartbeat coming due
        // while the session just sits idle with nothing else waking this loop up.
        if st.last_heartbeat_check.elapsed() >= HEARTBEAT_CHECK_INTERVAL {
            st.last_heartbeat_check = Instant::now();
            if st.try_deliver_due_heartbeats() {
                dirty = true;
            }
        }
        // 6. Fold finalized lines into the transcript ring (there is no terminal to print to)
        //    and broadcast a snapshot when anything changed. Change-only, like the TUI loop.
        let _ = st.app.drain_flush_remote();
        if dirty || st.busy {
            st.app.busy = st.busy;
            if st.busy {
                st.app.turn_elapsed_secs = st.busy_since.elapsed().as_secs();
            }
            let project = forge_config::project_initialization(std::path::Path::new(&cwd));
            let mut snap = build_snapshot_frame(
                &st.app,
                SnapshotIdentity {
                    session_id: &session_id,
                    title: &current_title,
                    cwd: &cwd,
                    worktree: worktree.as_deref(),
                    project_initialized: project.initialized,
                    project_init_hint: project.hint,
                    exposure: "daemon".to_string(),
                },
                st.copy_text.clone(),
                st.prompt_seq,
                st.notes.clone(),
                revision,
            );
            if last_snap.as_ref() != Some(&snap) {
                revision += 1;
                snap.revision = revision;
                // A fresh turn starting clears the previous turn's failure latch.
                if snap.busy && last_snap.as_ref().is_none_or(|p| !p.busy) {
                    turn_error = None;
                }
                // Actionable notifications: needs-a-decision / turn-done / turn-failed
                // transitions, debounced to zero-connected-clients, dispatched fire-and-forget
                // across every configured channel (Web Push + native APNs) — the broadcast below
                // never waits on delivery. Computed once regardless of which channels are
                // configured, so native-only (no Web Push) deployments still notify.
                if crate::push::should_push(ws_clients.load(std::sync::atomic::Ordering::Relaxed)) {
                    if let Some(msg) = crate::push::detect_trigger(
                        last_snap.as_ref(),
                        &snap,
                        turn_error.as_deref(),
                    ) {
                        if let Some(notifier) = &apns {
                            // Also nudge this session's Live Activity (if any) at the same
                            // discrete moments rather than on every streaming-token snapshot
                            // tick — Apple throttles overly-frequent remote updates.
                            notifier.dispatch_live_activity(
                                session_id.clone(),
                                crate::apns::LiveActivityContentState {
                                    busy: snap.busy,
                                    waiting: snap.permission_prompt.is_some()
                                        || snap.question.is_some(),
                                    cost_usd: snap.cost_usd,
                                    context_tokens: snap.context_tokens,
                                    context_limit: snap.context_limit.unwrap_or(200_000) as u64,
                                    question: snap
                                        .permission_prompt
                                        .clone()
                                        .or_else(|| snap.question.clone()),
                                    prompt_seq: Some(snap.prompt_seq),
                                    tasks_done: Some(
                                        snap.tasks.iter().filter(|t| t.status == "done").count()
                                            as u64,
                                    ),
                                    tasks_total: Some(snap.tasks.len() as u64),
                                    state_since: Some(now_secs().max(0) as u64),
                                },
                            );
                            notifier.dispatch_alert(msg.clone());
                        }
                        if let Some(notifier) = &push {
                            notifier.dispatch(msg);
                        }
                    }
                }
                last_snap = Some(snap.clone());
                let frame = std::sync::Arc::new(remote::SnapshotFrame::new(snap));
                if let Ok(mut log) = events.lock() {
                    log.push(revision, frame.clone());
                }
                let _ = snapshot_tx.send(frame);
                last_activity.store(now_secs(), Ordering::Relaxed);
            }
            dirty = false;
        }
        // Headless pacing: ~30ms keeps streaming snappy at a fraction of the TUI's frame work.
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(30)) => {}
            _ = shutdown_rx.changed() => {}
        }
    }

    // Shutdown (archive): stop the turn, run SessionEnd hooks, tell clients to stop reconnecting.
    if let Some(h) = st.turn_handle.take() {
        h.abort();
    }
    st.pending = None;
    st.pending_question = None;
    {
        let (hooks, sid, workspace) = {
            let s = st.session.lock().await;
            if let Some(json) = st.app.view_snapshot_json() {
                s.save_view_snapshot(&json);
            }
            (
                s.hooks().to_vec(),
                s.session_id().to_string(),
                s.workspace_root().to_path_buf(),
            )
        };
        forge_core::hooks::run_session_hooks_in(
            &hooks,
            forge_config::HookEvent::SessionEnd,
            &sid,
            Some(&workspace),
        )
        .await;
    }
    let mut closed = last_snap.unwrap_or_default();
    closed.closed = true;
    closed.revision += 1;
    let closed = std::sync::Arc::new(remote::SnapshotFrame::new(closed));
    if let Ok(mut log) = events.lock() {
        log.push(closed.snapshot.revision, closed.clone());
    }
    let _ = snapshot_tx.send(closed);
}

impl DriverState {
    fn start_turn(&mut self, prompt: &str) {
        self.last_prompt = Some(prompt.to_string());
        self.turn_gen += 1;
        self.turn_handle = Some(spawn_turn(
            prompt,
            &self.session,
            &self.done_tx,
            self.turn_gen,
            &mut self.app,
            &mut self.busy,
            &mut self.busy_since,
        ));
    }

    fn interrupt_turn(&mut self) {
        if let Some(h) = self.turn_handle.take() {
            h.abort();
        }
        self.turn_gen += 1;
        self.busy = false;
        self.loop_state = None;
        self.goal_state = None;
        self.pending = None;
        self.pending_question = None;
        self.app.prompt = None;
        self.app.clear_question();
        self.app.workflow.on_interrupt();
        self.app.apply(forge_tui::PresenterEvent::AssistantDone);

        // An interrupt cancels only the active turn. Prompts submitted while it was running are
        // still valid work and must drain FIFO; start the head under the new generation so the
        // aborted turn's DoneGuard signal remains harmlessly stale.
        if let Some(next) = self.take_next_queued_prompt() {
            self.start_turn(&next);
        }
    }

    fn take_next_queued_prompt(&mut self) -> Option<String> {
        dequeue_prompt(
            &mut self.queued_prompts,
            &mut self.app,
            &mut self.prompt_history,
        )
    }

    /// See [`try_deliver_due_heartbeats`] — the daemon-driver call site (turn-end + periodic tick).
    fn try_deliver_due_heartbeats(&mut self) -> bool {
        try_deliver_due_heartbeats(
            &self.session,
            &mut self.queued_prompts,
            &mut self.app,
            &mut self.prompt_history,
            &mut self.last_prompt,
            &self.done_tx,
            &mut self.turn_gen,
            &mut self.turn_handle,
            &mut self.busy,
            &mut self.busy_since,
        )
    }

    /// Act on a [`DispatchOutcome`] — the headless twin of the TUI's outcome match arms.
    fn handle_outcome(&mut self, outcome: DispatchOutcome) {
        match outcome {
            DispatchOutcome::Handled => {}
            DispatchOutcome::Quit => {
                // The daemon owns the process; a phone-sent /quit must never kill every OTHER
                // session hosted here. Archiving is the session-scoped exit.
                self.app
                    .note("⏻ /quit is host-only — archive this session from the session list");
            }
            DispatchOutcome::RunTurn {
                prompt,
                guidance,
                tier,
            } => {
                self.turn_gen += 1;
                self.turn_handle = Some(spawn_turn_with(
                    prompt,
                    guidance,
                    tier,
                    &self.session,
                    &self.done_tx,
                    self.turn_gen,
                    &mut self.app,
                    &mut self.busy,
                    &mut self.busy_since,
                ));
            }
            DispatchOutcome::RunCompact => {
                self.turn_gen += 1;
                self.turn_handle = Some(spawn_compact(
                    &self.session,
                    &self.done_tx,
                    self.turn_gen,
                    &mut self.app,
                    &mut self.busy,
                    &mut self.busy_since,
                ));
            }
            DispatchOutcome::RunSavedWorkflow { name, args } => {
                self.turn_gen += 1;
                self.turn_handle = Some(spawn_saved_workflow(
                    &self.session,
                    &self.done_tx,
                    self.turn_gen,
                    &mut self.app,
                    &mut self.busy,
                    &mut self.busy_since,
                    name,
                    args,
                ));
            }
            DispatchOutcome::RunDuel { task } => {
                self.turn_gen += 1;
                self.turn_handle = Some(spawn_duel(
                    &self.session,
                    &self.done_tx,
                    self.turn_gen,
                    &mut self.app,
                    &mut self.busy,
                    &mut self.busy_since,
                    task,
                    Arc::clone(&self.pending_duel),
                ));
            }
            DispatchOutcome::StartLoop { prompt } => {
                self.turn_gen += 1;
                self.loop_state = Some(LoopState {
                    gen: self.turn_gen,
                    iter: 1,
                });
                self.app.note("↻ loop started — Stop to interrupt");
                self.turn_handle = Some(spawn_turn_with(
                    prompt,
                    vec![LOOP_GUIDANCE.to_string()],
                    None,
                    &self.session,
                    &self.done_tx,
                    self.turn_gen,
                    &mut self.app,
                    &mut self.busy,
                    &mut self.busy_since,
                ));
            }
            DispatchOutcome::StartGoal { prompt, goal } => {
                self.turn_gen += 1;
                self.goal_state = Some(GoalState {
                    gen: self.turn_gen,
                    iter: 1,
                    prev_done: 0,
                    no_progress: 0,
                    goal,
                });
                self.app
                    .note("🎯 goal running autonomously — Stop to interrupt");
                self.turn_handle = Some(spawn_turn_with(
                    prompt,
                    vec![GOAL_GUIDANCE.to_string()],
                    Some(forge_types::TaskTier::Complex),
                    &self.session,
                    &self.done_tx,
                    self.turn_gen,
                    &mut self.app,
                    &mut self.busy,
                    &mut self.busy_since,
                ));
            }
            DispatchOutcome::PendingMesh(rx) => self.mesh_load_rx = Some(rx),
            DispatchOutcome::PendingUsage(rx) => self.usage_load_rx = Some(rx),
            DispatchOutcome::PendingVoice(start) => {
                // This daemon-hosted session has no local `Tui` (no PTT push/pop, no waveform
                // tick loop) — /voice isn't supported headless. Release whatever
                // `dispatch_command` already started (a live mic stream, or a download) rather
                // than leaking it, and tell the client why.
                match start {
                    VoiceStart::Recording { handle, .. } => handle.cancel(),
                    VoiceStart::Downloading { .. } | VoiceStart::Error => {}
                }
                self.app.voice = None;
                push_remote_note(
                    &mut self.notes,
                    "voice: /voice needs the TUI — not available on a `forge serve`-hosted session",
                );
            }
            DispatchOutcome::ToggleRemote { .. } => {
                push_remote_note(
                    &mut self.notes,
                    "◉ this session is served by the forge serve daemon — remote is always on",
                );
            }
            DispatchOutcome::CopyToClipboard(text) => {
                let chars = text.chars().count();
                push_remote_note(
                    &mut self.notes,
                    &format!("✓ copy ready ({chars} chars) — tap “Copy here”"),
                );
                self.copy_text = Some(text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_driver_state() -> DriverState {
        let session = super::build_session_with(
            Box::new(forge_tui::HeadlessPresenter::default()),
            true,
            None,
            None,
            None,
            true,
        )
        .await
        .expect("build mock session");
        let catalog =
            std::sync::Arc::new(forge_skills::Catalog::load(&forge_config::command_sources()));
        let (done_tx, _) = std::sync::mpsc::channel();
        DriverState {
            session: std::sync::Arc::new(tokio::sync::Mutex::new(session)),
            app: App::default(),
            catalog,
            armed_project: std::collections::HashSet::new(),
            trust_project: false,
            done_tx,
            busy: false,
            busy_since: Instant::now(),
            turn_gen: 10,
            last_auto_compact_gen: 0,
            turn_handle: None,
            loop_state: None,
            goal_state: None,
            pending: None,
            pending_question: None,
            pending_duel: std::sync::Arc::new(std::sync::Mutex::new(None)),
            duel_state: None,
            assay_lenses: Vec::new(),
            assay_scope: forge_types::AssayScope::Repo,
            queued_prompts: Vec::new(),
            prompt_history: Vec::new(),
            last_prompt: None,
            prompt_seq: 0,
            notes: Vec::new(),
            copy_text: None,
            pending_mentions: Vec::new(),
            remote_keys: std::collections::VecDeque::new(),
            mesh_load_rx: None,
            usage_load_rx: None,
            cwd: String::new(),
            last_heartbeat_check: Instant::now(),
        }
    }

    #[tokio::test]
    async fn dropping_the_last_handle_aborts_its_driver_task() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = started_tx.send(());
            struct NotifyDrop(Option<tokio::sync::oneshot::Sender<()>>);
            impl Drop for NotifyDrop {
                fn drop(&mut self) {
                    let _ = self.0.take().expect("drop once").send(());
                }
            }
            let _notify = NotifyDrop(Some(done_tx));
            std::future::pending::<()>().await;
        });
        started_rx
            .await
            .expect("driver must start before it is dropped");
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let (_, snapshot_rx) = tokio::sync::watch::channel(std::sync::Arc::new(
            remote::SnapshotFrame::new(remote::Snapshot::default()),
        ));
        let (input_tx, _) = tokio::sync::mpsc::channel(1);
        drop(SessionDriverHandle {
            session_id: "test".into(),
            title: std::sync::Arc::new(std::sync::RwLock::new(String::new())),
            cwd: String::new(),
            worktree: None,
            created_at: 0,
            snapshot_rx,
            events: std::sync::Arc::new(std::sync::Mutex::new(remote::EventLog::new(1))),
            input_tx,
            last_activity: std::sync::Arc::new(AtomicI64::new(0)),
            ws_clients: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            shutdown_tx,
            task: std::sync::Mutex::new(Some(task)),
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), done_rx)
            .await
            .expect("dropped handle must abort retained driver")
            .expect("driver drop notifier");
    }

    #[tokio::test]
    async fn timed_out_join_aborts_the_retained_driver_task() {
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let (snapshot_tx, snapshot_rx) = tokio::sync::watch::channel(std::sync::Arc::new(
            remote::SnapshotFrame::new(remote::Snapshot::default()),
        ));
        drop(snapshot_tx);
        let (input_tx, _) = tokio::sync::mpsc::channel(1);
        let handle = SessionDriverHandle {
            session_id: "test".into(),
            title: std::sync::Arc::new(std::sync::RwLock::new(String::new())),
            cwd: String::new(),
            worktree: None,
            created_at: 0,
            snapshot_rx,
            events: std::sync::Arc::new(std::sync::Mutex::new(remote::EventLog::new(1))),
            input_tx,
            last_activity: std::sync::Arc::new(AtomicI64::new(0)),
            ws_clients: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            shutdown_tx,
            task: std::sync::Mutex::new(Some(tokio::spawn(std::future::pending()))),
        };
        handle.join(std::time::Duration::from_millis(1)).await;
        assert!(handle
            .task
            .lock()
            .expect("driver task lock poisoned")
            .is_none());
    }

    #[tokio::test]
    async fn interrupt_with_queue_starts_fifo_head_and_keeps_driver_busy() {
        let mut state = test_driver_state().await;
        state.busy = true;
        state.turn_handle = Some(tokio::spawn(std::future::pending()));
        state.queued_prompts = vec!["second".into(), "third".into()];

        state.interrupt_turn();

        assert_eq!(state.queued_prompts, vec!["third"]);
        assert!(state.turn_handle.is_some());
        assert!(state.busy);
        assert_eq!(state.turn_gen, 12);
        state.turn_handle.take().unwrap().abort();
    }

    /// Fleet-messaging `RemoteInput::Steer` while a turn is BUSY jumps the queue (front, not
    /// back) instead of touching the running turn — it must never interrupt it.
    #[tokio::test]
    async fn steer_input_while_busy_jumps_the_queue_without_touching_the_turn() {
        let mut state = test_driver_state().await;
        state.busy = true;
        state.turn_handle = Some(tokio::spawn(std::future::pending()));
        state.queued_prompts = vec!["backlog".into()];

        state
            .handle_input(remote::RemoteInput::Steer {
                text: "urgent".into(),
            })
            .await
            .unwrap();

        assert_eq!(state.queued_prompts, vec!["urgent", "backlog"]);
        assert!(state.busy, "steer must not interrupt the running turn");
        assert!(state.turn_handle.is_some());
        state.turn_handle.take().unwrap().abort();
    }

    /// The same input while IDLE has no backlog to outrank, so it behaves exactly like a normal
    /// prompt — it starts a turn immediately instead of sitting in the queue.
    #[tokio::test]
    async fn steer_input_while_idle_starts_a_turn_immediately() {
        let mut state = test_driver_state().await;
        assert!(!state.busy);

        state
            .handle_input(remote::RemoteInput::Steer { text: "go".into() })
            .await
            .unwrap();

        assert!(state.queued_prompts.is_empty());
        assert!(state.busy, "an idle steer starts a turn immediately");
        if let Some(h) = state.turn_handle.take() {
            h.abort();
        }
    }

    // Regression: the idle loop only broadcasts dirty frames, so a /mesh load resolving without
    // reporting a change left remote clients on the loading spinner until an unrelated input.
    #[tokio::test]
    async fn mesh_overlay_resolution_reports_a_dirty_frame() {
        let mut state = test_driver_state().await;
        state.app.mesh_overlay.open = true;
        state.app.mesh_overlay.loading = true;
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.mesh_load_rx = Some(rx);

        assert!(!state.poll_overlay_loads(), "pending load is not a change");

        tx.send(Some(forge_tui::MeshOverlay {
            open: true,
            loading: false,
            ..Default::default()
        }))
        .unwrap();
        assert!(
            state.poll_overlay_loads(),
            "resolution must dirty the frame"
        );
        assert!(!state.app.mesh_overlay.loading);
        assert!(state.mesh_load_rx.is_none());
    }

    #[tokio::test]
    async fn stale_interrupt_done_signal_cannot_stop_fifo_drain() {
        let mut state = test_driver_state().await;
        state.busy = true;
        state.turn_handle = Some(tokio::spawn(std::future::pending()));
        state.queued_prompts = vec!["second".into(), "third".into()];

        state.interrupt_turn();
        assert_eq!(state.last_prompt.as_deref(), Some("second"));
        assert_eq!(state.queued_prompts, vec!["third"]);
        assert_eq!(state.turn_gen, 12);
        assert!(state.busy);

        // The aborted generation's DoneGuard arrives after the replacement turn starts.
        state.on_turn_done(10).await;
        assert!(state.busy);
        assert_eq!(state.turn_gen, 12);
        assert!(state.turn_handle.is_some());
        assert_eq!(state.queued_prompts, vec!["third"]);

        // Completing the replacement turn drains the remaining prompt in FIFO order.
        state.turn_handle.take().unwrap().abort();
        state.on_turn_done(12).await;
        assert_eq!(state.queued_prompts, Vec::<String>::new());
        assert_eq!(state.last_prompt.as_deref(), Some("third"));
        assert_eq!(state.turn_gen, 13);
        assert!(state.busy);
        assert!(state.turn_handle.is_some());
        state.turn_handle.take().unwrap().abort();
    }

    #[tokio::test]
    async fn queued_reprompt_steers_the_next_loop_iteration() {
        let mut state = test_driver_state().await;
        state.busy = true;
        state.loop_state = Some(LoopState { gen: 10, iter: 1 });
        state.queued_prompts = vec!["apply the correction".into(), "then verify".into()];

        state.on_turn_done(10).await;

        assert_eq!(state.last_prompt.as_deref(), Some("apply the correction"));
        assert_eq!(state.queued_prompts, vec!["then verify"]);
        assert_eq!(state.prompt_history, vec!["apply the correction"]);
        assert!(matches!(
            state.loop_state,
            Some(LoopState { gen: 11, iter: 2 })
        ));
        assert!(state.busy);
        state.turn_handle.take().unwrap().abort();
    }

    #[tokio::test]
    async fn queued_reprompt_steers_the_next_goal_iteration() {
        let mut state = test_driver_state().await;
        state.busy = true;
        state.goal_state = Some(GoalState {
            gen: 10,
            iter: 1,
            prev_done: 0,
            no_progress: 0,
            goal: "finish the goal".into(),
        });
        state.queued_prompts = vec!["prioritize the regression".into()];

        state.on_turn_done(10).await;

        assert_eq!(
            state.last_prompt.as_deref(),
            Some("prioritize the regression")
        );
        assert!(state.queued_prompts.is_empty());
        assert_eq!(state.prompt_history, vec!["prioritize the regression"]);
        assert!(matches!(
            state.goal_state,
            Some(GoalState {
                gen: 11,
                iter: 2,
                no_progress: 1,
                ..
            })
        ));
        assert!(state.busy);
        state.turn_handle.take().unwrap().abort();
    }

    #[tokio::test]
    async fn interrupt_without_queue_leaves_driver_idle() {
        let mut state = test_driver_state().await;
        state.busy = true;
        state.turn_handle = Some(tokio::spawn(std::future::pending()));

        state.interrupt_turn();

        assert!(state.queued_prompts.is_empty());
        assert!(state.turn_handle.is_none());
        assert!(!state.busy);
        assert_eq!(state.turn_gen, 11);
    }

    #[tokio::test]
    async fn over_a_thousand_queued_reprompts_drain_fifo_without_stale_done_corruption() {
        // More than the largest real Codex/Claude history observed by the aggregate-only
        // history profiler (654 user turns).
        const PROMPTS: usize = 1_024;

        let mut state = test_driver_state().await;
        state.busy = true;
        state.turn_handle = Some(tokio::spawn(std::future::pending()));
        state.queued_prompts = (0..PROMPTS)
            .map(|index| format!("reprompt-{index:03}"))
            .collect();

        state.interrupt_turn();
        for index in 0..PROMPTS {
            assert_eq!(
                state.last_prompt.as_deref(),
                Some(format!("reprompt-{index:03}").as_str())
            );
            let generation = state.turn_gen;
            state.turn_handle.take().unwrap().abort();
            state.on_turn_done(generation).await;
        }

        assert!(state.queued_prompts.is_empty());
        assert!(state.turn_handle.is_none());
        assert!(!state.busy);
        assert_eq!(state.prompt_history.len(), PROMPTS);
        assert_eq!(state.prompt_history.first().unwrap(), "reprompt-000");
        assert_eq!(
            state.prompt_history.last().unwrap(),
            &format!("reprompt-{:03}", PROMPTS - 1)
        );
    }
}
