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

/// What to run: the parameters of one daemon-hosted session.
pub(crate) struct DriverSpec {
    /// The session's working directory. When it differs from the daemon process's cwd, tool
    /// calls are rooted here via `Session::set_work_root` (the audited subagent-worktree rewrite).
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
}

/// The daemon-side handle to a running session driver — everything `forge serve`'s HTTP layer
/// needs: the live snapshot stream (+ replay log) to serve WS clients, the input queue to feed
/// them, and identity/metadata for `GET /api/sessions`. Mirrors `mcp_serve`'s
/// LocalSessionManager shape: one task per session, addressed by id.
pub(crate) struct SessionDriverHandle {
    pub session_id: String,
    pub title: String,
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
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SessionDriverHandle {
    /// Ask the driver to stop (archive): the loop aborts any running turn, runs SessionEnd
    /// hooks, broadcasts one final `closed` frame, and exits. Idempotent.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Wait (bounded) for the driver task to finish after [`Self::shutdown`].
    pub async fn join(&self, timeout: std::time::Duration) {
        let task = self.task.lock().await.take();
        if let Some(task) = task {
            let _ = tokio::time::timeout(timeout, task).await;
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
    // Root tool calls in the session's own directory when it differs from the process cwd —
    // without this every relative path/shell command would act on the DAEMON's cwd.
    let daemon_cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if spec.cwd != daemon_cwd {
        session.set_work_root(Some(std::path::PathBuf::from(&spec.cwd)));
        if spec.resume.is_none() {
            let _ = session.prime_guidance(&[format!(
                "This session's working directory is {} — resolve every relative path there and \
                 pass it as `cwd` to shell commands.",
                spec.cwd
            )]);
        }
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

    let task = tokio::spawn(drive_session(
        session,
        session_id.clone(),
        spec.title.clone(),
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
        title: spec.title,
        cwd: spec.cwd,
        worktree: spec.worktree,
        created_at: now_secs(),
        snapshot_rx,
        events,
        input_tx,
        last_activity,
        ws_clients,
        shutdown_tx,
        task: tokio::sync::Mutex::new(Some(task)),
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
}

#[allow(clippy::too_many_arguments)]
async fn drive_session(
    session: std::sync::Arc<tokio::sync::Mutex<Session>>,
    session_id: String,
    title: String,
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
        let hooks = session.lock().await.hooks().to_vec();
        forge_core::hooks::run_session_hooks(
            &hooks,
            forge_config::HookEvent::SessionStart,
            &session_id,
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
        st.poll_overlay_loads();
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
                    title: &title,
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
        let (hooks, sid) = {
            let s = st.session.lock().await;
            if let Some(json) = st.app.view_snapshot_json() {
                s.save_view_snapshot(&json);
            }
            (s.hooks().to_vec(), s.session_id().to_string())
        };
        forge_core::hooks::run_session_hooks(&hooks, forge_config::HookEvent::SessionEnd, &sid)
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

fn take_next_queued_prompt(queue: &mut Vec<String>, app: &mut App) -> Option<String> {
    let next = queue.first().cloned()?;
    queue.remove(0);
    app.set_queued(queue);
    Some(next)
}

impl DriverState {
    /// One remote input — the headless mirror of `run_chat_tui`'s remote drain, minus the
    /// host-terminal cases (`/remote` toggling, host clipboard).
    async fn handle_input(&mut self, input: remote::RemoteInput) -> Result<()> {
        match input {
            remote::RemoteInput::Prompt { text, attachments } => {
                // A fresh prompt starts a fresh interaction — drop stale notices + copy payload.
                self.notes.clear();
                self.copy_text = None;

                // A message-correlated attachment list (mobile upload race fix, mirrors
                // `run_chat_tui`'s remote drain) is authoritative for THIS turn when non-empty:
                // discard stale ambient state from an unrelated `Attach` up front (image
                // attachment happens inside `resolve_prompt_attachments` regardless of dispatch
                // branch, matching how it already applied ambiently); non-image mentions are
                // threaded into `submit_line` and only actually prepended in its plain-prompt
                // branch, exactly where the old ambient `pending_mentions` were.
                let has_explicit_attachments = !attachments.is_empty();
                if has_explicit_attachments {
                    self.pending_mentions.clear();
                }
                let explicit_mentions = resolve_prompt_attachments(
                    &self.session,
                    &mut self.app,
                    &mut self.notes,
                    &self.cwd,
                    attachments,
                )
                .await;

                if self.busy {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        // nothing to queue
                    } else if trimmed.starts_with('/') && !trimmed.starts_with("//") {
                        self.app
                            .note("⏳ commands run when the turn is idle — finish or Stop first");
                    } else {
                        self.queued_prompts.push(text.clone());
                        self.app.set_queued(&self.queued_prompts);
                        self.app.note(&format!(
                            "⏳ queued ({} pending) — runs after this turn",
                            self.queued_prompts.len()
                        ));
                    }
                    return Ok(());
                }
                self.submit_line(text, has_explicit_attachments.then_some(explicit_mentions))
                    .await?;
            }
            remote::RemoteInput::Allow { yes, seq } => {
                if !remote::prompt_seq_current(self.prompt_seq, seq) {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ stale answer ignored — the prompt changed; review the current one",
                    );
                } else if let Some((tool, reply)) = self.pending.take() {
                    let outcome = if yes {
                        ConfirmOutcome::Allow
                    } else {
                        ConfirmOutcome::Deny
                    };
                    let _ = reply.send(outcome);
                    self.app.prompt = None;
                    if yes {
                        self.app.note(&format!("✓ remote allowed {tool}"));
                    } else {
                        self.app.note(&format!("✗ remote denied {tool}"));
                    }
                }
            }
            remote::RemoteInput::Answer { text, seq } => {
                if !remote::prompt_seq_current(self.prompt_seq, seq) {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ stale answer ignored — the prompt changed; review the current one",
                    );
                } else if self.app.awaiting_question() {
                    if let Some(ans) = self.app.resolve_question(&text) {
                        if let Some(tx) = self.pending_question.take() {
                            let _ = tx.send(ans);
                        }
                    } else {
                        self.app.note("⚠ remote answer was invalid — re-asking");
                    }
                }
            }
            remote::RemoteInput::Interrupt => {
                if self.busy {
                    self.interrupt_turn();
                    self.app.note("⏹ remote interrupted — stopped responding");
                }
            }
            remote::RemoteInput::Dequeue { index, text } => {
                let idx = index as usize;
                if idx < self.queued_prompts.len() && self.queued_prompts[idx] == text {
                    self.queued_prompts.remove(idx);
                    self.app.set_queued(&self.queued_prompts);
                    self.app.note(&format!(
                        "✕ remote dequeued — {} pending",
                        self.queued_prompts.len()
                    ));
                } else {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ stale dequeue ignored — the queue changed; review the current list",
                    );
                }
            }
            remote::RemoteInput::Key { key } => {
                // Same guards as the TUI drain: prompts resolve ONLY via seq-checked
                // Allow/Answer, and a bare idle Esc must never do anything drastic.
                if self.pending.is_some() || self.app.awaiting_question() {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ a prompt is pending — answer it with its buttons",
                    );
                } else {
                    match remote::named_key(&key) {
                        Some(KeyKind::Esc) if !self.busy && !any_remote_modal_open(&self.app) => {
                            push_remote_note(&mut self.notes, "Esc ignored — nothing to close");
                        }
                        Some(k) => self.remote_keys.push_back(k),
                        None => push_remote_note(
                            &mut self.notes,
                            &format!("⚠ unknown key {key:?} ignored"),
                        ),
                    }
                }
            }
            remote::RemoteInput::OverlaySelect { id } => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Select(id));
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::OverlayNav { delta } => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Nav(delta));
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::OverlayFilter { text } => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Filter(text));
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::OverlayCancel => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Cancel);
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::Attach { path, image } => {
                let cwd = self.cwd.clone();
                handle_remote_attach(
                    &self.session,
                    &mut self.app,
                    &mut self.pending_mentions,
                    &cwd,
                    path,
                    image,
                )
                .await;
            }
        }
        Ok(())
    }

    /// Submit one idle-state line: `//` escape, `/command` dispatch, or a plain prompt —
    /// the same routing the TUI's submit path applies. `explicit_mentions`, when `Some`, is the
    /// current `Prompt`'s own message-correlated non-image attachment list (mobile upload race
    /// fix) and is authoritative in the plain-prompt branch below; `None` (every other caller —
    /// the local key-driven submit paths, which never carry a fresh attachment list) falls back
    /// to exactly the old ambient `pending_mentions` behavior.
    async fn submit_line(
        &mut self,
        line: String,
        explicit_mentions: Option<Vec<String>>,
    ) -> Result<()> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if !trimmed.is_empty() && self.prompt_history.last().map(String::as_str) != Some(trimmed) {
            self.prompt_history.push(trimmed.to_string());
        }
        if let Some(rest) = line.strip_prefix("//") {
            let hooks = self.session.lock().await.hooks().to_vec();
            let escaped = format!("/{rest}");
            match forge_core::hooks::run_prompt_hooks(&hooks, &escaped).await {
                Err(reason) => self
                    .app
                    .note(&format!("⎇ prompt blocked by hook: {reason}")),
                Ok(prompt) => self.start_turn(&prompt),
            }
            return Ok(());
        }
        if line.starts_with('/') {
            let outcome = dispatch_command(
                &line,
                &self.session,
                None,
                &mut self.app,
                &self.catalog,
                &mut self.armed_project,
                self.trust_project,
                self.busy,
                &mut self.assay_lenses,
                &mut self.assay_scope,
            )
            .await?;
            self.handle_outcome(outcome);
            return Ok(());
        }
        // Uploaded text files ride this prompt as @path mentions — the explicit,
        // message-correlated list (if this prompt carried one) is authoritative; otherwise fall
        // back to exactly the old ambient `Attach`-then-`Prompt` behavior.
        let line = match explicit_mentions {
            Some(mut mentions) => prepend_attach_mentions(&mut mentions, line),
            None => prepend_attach_mentions(&mut self.pending_mentions, line),
        };
        let hooks = self.session.lock().await.hooks().to_vec();
        match forge_core::hooks::run_prompt_hooks(&hooks, &line).await {
            Err(reason) => self
                .app
                .note(&format!("⎇ prompt blocked by hook: {reason}")),
            Ok(prompt) => {
                // Expand `@path` mentions exactly like the TUI submit path.
                let (file_blocks, included, skipped) = expand_at_files(&prompt);
                if !included.is_empty() {
                    self.app
                        .note(&format!("📎 included {}", included.join(", ")));
                }
                for s in &skipped {
                    self.app.note(&format!("⚠ skipped {s}"));
                }
                self.last_prompt = Some(prompt.clone());
                if file_blocks.is_empty() {
                    self.start_turn(&prompt);
                } else {
                    self.turn_gen += 1;
                    self.turn_handle = Some(spawn_turn_with(
                        prompt.clone(),
                        file_blocks,
                        None,
                        &self.session,
                        &self.done_tx,
                        self.turn_gen,
                        &mut self.app,
                        &mut self.busy,
                        &mut self.busy_since,
                    ));
                }
            }
        }
        Ok(())
    }

    fn start_turn(&mut self, prompt: &str) {
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
        take_next_queued_prompt(&mut self.queued_prompts, &mut self.app)
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

    /// The headless key router: modal surfaces first (same precedence as the TUI key loop),
    /// then plain input editing. Only surfaces a remote client can actually drive are handled;
    /// host-terminal-only hotkeys have no headless meaning and are ignored.
    async fn process_key(&mut self, key: KeyKind) -> Result<()> {
        // The workflow view is modal while open.
        if self.app.workflow.open {
            if self.app.workflow.zoom.is_some() {
                workflow_zoom_key(&mut self.app, key);
            } else {
                match key {
                    KeyKind::Esc | KeyKind::Char('q') => self.app.workflow.open = false,
                    KeyKind::Up | KeyKind::Char('k') => self.app.workflow.move_selection(-1),
                    KeyKind::Down | KeyKind::Char('j') => self.app.workflow.move_selection(1),
                    KeyKind::PageUp => self.app.workflow.move_selection(-5),
                    KeyKind::PageDown => self.app.workflow.move_selection(5),
                    KeyKind::Home => self.app.workflow.selected = 0,
                    KeyKind::End => {
                        self.app.workflow.selected = self.app.workflow.rows.len().saturating_sub(1);
                    }
                    KeyKind::Enter if !self.app.workflow.rows.is_empty() => {
                        self.app.workflow.zoom = Some(Default::default());
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        // The /config editor is modal while open — same actions as the TUI, except the
        // $EDITOR jump (host-terminal-only).
        if self.app.config_editor.open {
            match self.app.config_editor.handle_key(key) {
                forge_tui::ConfigAction::Save { path, value } => {
                    let result = if let Some(provider) = path.strip_prefix("key.") {
                        if value.trim().is_empty() {
                            forge_config::remove_api_key(provider)
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        } else {
                            forge_config::store_api_key(provider, value.trim())
                                .map_err(|e| e.to_string())
                        }
                    } else {
                        let scope = if self.app.config_editor.project_scope {
                            forge_config::ConfigScope::Project
                        } else {
                            forge_config::ConfigScope::User
                        };
                        forge_config::set_config_value(scope, &path, &value)
                            .map_err(|e| e.to_string())
                    };
                    match result {
                        Ok(()) => {
                            self.app.config_editor.rows = config_editor_rows();
                            self.app.config_editor.status = Some(format!("✓ saved {path}"));
                        }
                        Err(e) => self.app.config_editor.status = Some(format!("✗ {e}")),
                    }
                }
                forge_tui::ConfigAction::Reset { path } => {
                    let scope = if self.app.config_editor.project_scope {
                        forge_config::ConfigScope::Project
                    } else {
                        forge_config::ConfigScope::User
                    };
                    match forge_config::reset_config_value(scope, &path) {
                        Ok(()) => {
                            self.app.config_editor.rows = config_editor_rows();
                            self.app.config_editor.status =
                                Some(format!("✓ reset {path} to default"));
                        }
                        Err(e) => self.app.config_editor.status = Some(format!("✗ {e}")),
                    }
                }
                forge_tui::ConfigAction::Reload => {
                    self.app.config_editor.rows = config_editor_rows();
                }
                forge_tui::ConfigAction::EditFile => {
                    self.app.config_editor.status =
                        Some("⚠ editing this section needs $EDITOR on the host".to_string());
                }
                forge_tui::ConfigAction::Close | forge_tui::ConfigAction::None => {}
            }
            return Ok(());
        }
        // Effort slider (opened by bare /effort).
        if self.app.effort_slider {
            match key {
                KeyKind::Left | KeyKind::Right => {
                    if matches!(key, KeyKind::Left) {
                        self.app.effort_slider_left();
                    } else {
                        self.app.effort_slider_right();
                    }
                    if let Some(level) = self.app.effort {
                        if let Ok(mut s) = self.session.try_lock() {
                            s.set_effort(Some(level));
                        } else {
                            let s = self.session.clone();
                            tokio::spawn(async move { s.lock().await.set_effort(Some(level)) });
                        }
                    }
                }
                KeyKind::Esc | KeyKind::Enter | KeyKind::ToggleEffortSlider => {
                    self.app.effort_slider = false;
                }
                _ => {}
            }
            return Ok(());
        }
        // Command palette.
        if self.app.palette.open {
            match key {
                KeyKind::Esc => {
                    self.app.palette.close();
                    self.app.input.clear();
                }
                KeyKind::Up => self.app.palette.move_up(),
                KeyKind::Down => self.app.palette.move_down(),
                KeyKind::Tab => {
                    if let Some(name) = self.app.palette.selected_name().map(|s| s.to_string()) {
                        self.app.input = format!("/{name}");
                        self.app.input_cursor = self.app.input.len();
                        self.app.palette.query = name;
                        self.app.palette.clamp();
                    }
                }
                KeyKind::Enter => {
                    let leading =
                        self.app.input.starts_with('/') && !self.app.input.starts_with("//");
                    if !leading {
                        self.app.palette.close();
                        return Ok(());
                    }
                    let has_args = self.app.input.trim().contains(char::is_whitespace);
                    let line = if has_args {
                        self.app.input.clone()
                    } else {
                        self.app
                            .palette
                            .selected_name()
                            .map(|n| format!("/{n}"))
                            .unwrap_or_else(|| self.app.input.clone())
                    };
                    self.app.palette.close();
                    self.app.input.clear();
                    if self.busy {
                        self.app
                            .note("⏳ commands run when the turn is idle — finish or Stop first");
                    } else {
                        // Route through the shared submit path (identical DispatchOutcome
                        // handling to a directly-typed command).
                        Box::pin(self.submit_line(line, None)).await?;
                    }
                }
                _ => {
                    let _ = handle_key(&mut self.app.input, &mut self.app.input_cursor, key);
                    sync_palette_to_slash_token(&mut self.app);
                }
            }
            return Ok(());
        }
        // Usage overlay: informational, Esc closes.
        if self.app.usage_overlay.open {
            if matches!(key, KeyKind::Esc) {
                self.app.usage_overlay.open = false;
            }
            return Ok(());
        }
        // Mesh inspector overlay.
        if self.app.mesh_overlay.open {
            match key {
                KeyKind::Esc => {
                    self.app.mesh_overlay.open = false;
                    self.app.mesh_overlay.cursor = 0;
                }
                KeyKind::Down => {
                    let max = self.app.mesh_overlay.candidates.len().saturating_sub(1);
                    self.app.mesh_overlay.cursor = (self.app.mesh_overlay.cursor + 1).min(max);
                }
                KeyKind::Up => {
                    self.app.mesh_overlay.cursor = self.app.mesh_overlay.cursor.saturating_sub(1);
                }
                _ => {}
            }
            return Ok(());
        }
        // @path file picker.
        if self.app.at_picker.open {
            match key {
                KeyKind::Esc => self.app.at_picker.close(),
                KeyKind::Up => self.app.at_picker.move_up(),
                KeyKind::Down => self.app.at_picker.move_down(),
                KeyKind::Tab | KeyKind::Enter => {
                    if let Some(path) = self.app.at_picker.selected_path() {
                        if let Some(tok) = forge_tui::at_token_at(
                            &self.app.input,
                            self.app.input_cursor.min(self.app.input.len()),
                        ) {
                            self.app
                                .input
                                .replace_range(tok.start..tok.end, &format!("@{path} "));
                            self.app.input_cursor = self.app.input.len();
                        } else {
                            self.app.input = format!("@{path} ");
                            self.app.input_cursor = self.app.input.len();
                        }
                    }
                    self.app.at_picker.close();
                }
                KeyKind::Char(c) => {
                    self.app.input.push(c);
                    sync_at_picker_to_at_token(&mut self.app);
                }
                KeyKind::Backspace => {
                    self.app.input.pop();
                    sync_at_picker_to_at_token(&mut self.app);
                }
                _ => {}
            }
            return Ok(());
        }
        // The generic picker (sessions / checkpoints / models / tempers / assay / copy / duel…).
        if self.app.picker.open {
            match key {
                KeyKind::Esc => {
                    if self.app.picker.kind == Some(forge_tui::PickerKind::Models)
                        && self.app.models_drilled.is_some()
                    {
                        open_models_root(&self.session, &mut self.app).await?;
                    } else {
                        self.app.models_drilled = None;
                        self.app.models_pin_mode = false;
                        if self.app.picker.kind == Some(forge_tui::PickerKind::Duel) {
                            self.duel_state = None;
                            self.app.note("⚔ duel discarded — no candidate was merged");
                        }
                        self.app.picker.close();
                    }
                }
                KeyKind::Up => self.app.picker.move_up(),
                KeyKind::Down => self.app.picker.move_down(),
                KeyKind::Tab if self.app.picker.kind == Some(forge_tui::PickerKind::Sessions) => {
                    let query = self.app.picker.query.clone();
                    self.app.show_archived = !self.app.show_archived;
                    open_sessions_picker(&mut self.app, &query)?;
                }
                KeyKind::DeleteForward
                    if self.app.picker.kind == Some(forge_tui::PickerKind::Sessions) =>
                {
                    if let Some(row) = self.app.picker.selected_row() {
                        if !row.id.starts_with("observe:") {
                            let store = crate::open_store()?;
                            if self.app.show_archived {
                                store.unarchive_session(&row.id)?;
                            } else {
                                store.archive_session(&row.id)?;
                            }
                            let query = self.app.picker.query.clone();
                            open_sessions_picker(&mut self.app, &query)?;
                        }
                    }
                }
                KeyKind::Enter => {
                    self.picker_enter().await?;
                }
                KeyKind::Char(c) => {
                    self.app.picker.query.push(c);
                    self.app.picker.clamp();
                }
                KeyKind::Backspace => {
                    self.app.picker.query.pop();
                    self.app.picker.clamp();
                }
                _ => {}
            }
            return Ok(());
        }
        // Esc: interrupt a running turn; idle Esc was already filtered at the drain.
        if matches!(key, KeyKind::Esc) {
            if self.busy {
                self.interrupt_turn();
                self.app.note("⏹ interrupted — stopped responding");
            }
            return Ok(());
        }
        // Temper cycling (SHIFT+TAB on the page).
        if matches!(key, KeyKind::CycleTemper | KeyKind::TemperCycle) {
            let Some(new) = self
                .session
                .try_lock()
                .ok()
                .map(|mut sess| sess.cycle_temper())
            else {
                self.app.note("⚠ try again in a moment — session is busy");
                return Ok(());
            };
            self.app.set_temper(new.label());
            let _ = forge_config::write_permission_mode(new);
            return Ok(());
        }
        // Plain input editing: keys accumulate into the input line; Enter submits (queued if
        // busy, exactly like local typing mid-turn).
        let outcome = handle_key(&mut self.app.input, &mut self.app.input_cursor, key);
        if let InputOutcome::Submit(raw_line) = outcome {
            let (line, _imgs) = self.app.resolve_paste_blocks(raw_line);
            if self.busy {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    // nothing to queue
                } else if trimmed.starts_with('/') && !trimmed.starts_with("//") {
                    self.app
                        .note("⏳ commands run when the turn is idle — finish or Stop first");
                } else {
                    self.queued_prompts.push(line);
                    self.app.set_queued(&self.queued_prompts);
                }
            } else {
                Box::pin(self.submit_line(line, None)).await?;
            }
        } else {
            // Keep the palette/@-picker in sync with what the (remote) cursor sits in, same as
            // the TUI's editing branch.
            let cur = self.app.input_cursor.min(self.app.input.len());
            let tok = forge_tui::slash_token_at(&self.app.input, cur)
                .filter(|t| cur >= t.start && cur <= t.end);
            if let Some(tok) = tok {
                self.app.at_picker.close();
                self.app.palette.open_with(&tok.name);
            } else {
                self.app.palette.close();
                sync_at_picker_to_at_token(&mut self.app);
            }
        }
        Ok(())
    }

    /// Enter on the generic picker — the headless mirror of the TUI's picker-Enter branch.
    async fn picker_enter(&mut self) -> Result<()> {
        let chosen = self.app.picker.selected_row().cloned();
        let kind = self.app.picker.kind;
        if kind == Some(forge_tui::PickerKind::Models) {
            if let Some(row) = chosen {
                if self.app.models_drilled.is_none() && !row.id.contains("::") {
                    open_models_provider(&self.session, &mut self.app, &row.id).await?;
                } else if row.id.contains("::") && self.app.models_pin_mode {
                    let model_id = forge_provider::normalize_model_id(&row.id).into_owned();
                    if let Ok(mut s) = self.session.try_lock() {
                        s.pin_model(Some(model_id.clone()));
                    } else {
                        let s = self.session.clone();
                        let m = model_id.clone();
                        tokio::spawn(async move { s.lock().await.pin_model(Some(m)) });
                    }
                    self.app.models_pin_mode = false;
                    self.app.models_drilled = None;
                    self.app.picker.close();
                    self.app
                        .note(&format!("⊕ model pinned: {model_id} (clears with /model)"));
                }
            }
            return Ok(());
        }
        self.app.picker.close();
        let (Some(row), Some(kind)) = (chosen, kind) else {
            return Ok(());
        };
        if kind == forge_tui::PickerKind::AssayChoice {
            self.turn_gen += 1;
            let lenses = std::mem::take(&mut self.assay_lenses);
            let scope = std::mem::replace(&mut self.assay_scope, forge_types::AssayScope::Repo);
            self.turn_handle = spawn_assay(
                row.id == "cleanup",
                lenses,
                scope,
                &self.session,
                &self.done_tx,
                self.turn_gen,
                &mut self.app,
                &mut self.busy,
                &mut self.busy_since,
            )
            .await?;
        } else if kind == forge_tui::PickerKind::CopyBlocks {
            if let Some((_, text)) = row
                .id
                .parse::<usize>()
                .ok()
                .and_then(|i| self.app.copy_candidates.get(i).cloned())
            {
                let chars = text.chars().count();
                push_remote_note(
                    &mut self.notes,
                    &format!("✓ copy ready ({chars} chars) — tap “Copy here”"),
                );
                self.copy_text = Some(text);
            }
            self.app.copy_candidates.clear();
        } else if kind == forge_tui::PickerKind::Duel {
            if let Some((report, guards)) = self.duel_state.take() {
                let repo_root = std::path::PathBuf::from(&self.cwd);
                let repo_key = repo_root.display().to_string();
                let winner_branch = row.id.clone();
                let merge_note = match forge_core::duel::merge_winner(&repo_root, &winner_branch) {
                    Ok(m) if m.conflicted_files.is_empty() => "merged cleanly".to_string(),
                    Ok(m) => format!(
                        "merged with conflicts in: {}",
                        m.conflicted_files.join(", ")
                    ),
                    Err(e) => format!("merge failed: {e}"),
                };
                if let Ok(store) = crate::open_store() {
                    for c in &report.candidates {
                        let won = c.branch == winner_branch;
                        let _ = store.record_duel_outcome(&repo_key, &c.model, won, &report.task);
                    }
                }
                let winner_model = report
                    .candidates
                    .iter()
                    .find(|c| c.branch == winner_branch)
                    .map(|c| c.model.clone())
                    .unwrap_or_else(|| "?".to_string());
                self.app
                    .note(&format!("⚔ duel winner: {winner_model} — {merge_note}"));
                drop(guards);
            }
        } else if kind == forge_tui::PickerKind::Sessions && row.id.starts_with("observe:") {
            self.app
                .note("⚠ observing a live MCP session isn't available from the daemon page");
        } else {
            picker_accept(kind, &row, &self.session, None, &mut self.app).await?;
        }
        Ok(())
    }

    /// A turn-done signal: mirrors the TUI's done drain — duel picker, /loop continuation,
    /// queued prompts, auto-compact.
    async fn on_turn_done(&mut self, g: u64) {
        if !(self.busy && g == self.turn_gen) {
            return;
        }
        self.busy = false;
        self.turn_handle = None;
        if let Some(json) = self.app.view_snapshot_json() {
            self.session.lock().await.save_view_snapshot(&json);
        }
        if let Some((report, guards)) = self.pending_duel.lock().unwrap().take() {
            if report.candidates.is_empty() {
                self.app.note("⚔ duel produced no usable candidates");
            } else {
                let rows = duel_picker_rows(&report);
                self.app.picker.open_with(
                    forge_tui::PickerKind::Duel,
                    &format!("⚔ duel — pick the winner ({} candidates)", rows.len()),
                    rows,
                );
                self.duel_state = Some((report, guards));
            }
        }
        if let Some(ls) = self.loop_state.take() {
            if ls.gen == g {
                let last = {
                    self.session
                        .lock()
                        .await
                        .last_assistant_text()
                        .map(str::to_string)
                };
                match loop_stop_reason(last.as_deref(), ls.iter) {
                    Some(reason) => self.app.note(reason),
                    None => {
                        self.turn_gen += 1;
                        self.loop_state = Some(LoopState {
                            gen: self.turn_gen,
                            iter: ls.iter + 1,
                        });
                        self.turn_handle = Some(spawn_turn_with(
                            "Continue toward completion.".to_string(),
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
                }
            } else {
                self.loop_state = Some(ls);
            }
        }
        if let Some(gs) = self.goal_state.take() {
            if gs.gen == g {
                let (done, total) = {
                    let s = self.session.lock().await;
                    let tasks = s.tasks();
                    (
                        tasks
                            .iter()
                            .filter(|t| t.status == forge_types::TodoStatus::Done)
                            .count(),
                        tasks.len(),
                    )
                };
                let last = {
                    self.session
                        .lock()
                        .await
                        .last_assistant_text()
                        .map(str::to_string)
                };
                let said_complete = is_goal_complete_marker(last.as_deref());
                let progressed = done > gs.prev_done;
                let no_progress = if progressed { 0 } else { gs.no_progress + 1 };
                match goal_stop_reason(said_complete, done, total, gs.iter, no_progress) {
                    Some(reason) if is_goal_complete_reason(reason) => {}
                    Some(reason) => self.app.note(reason),
                    None => {
                        self.turn_gen += 1;
                        self.goal_state = Some(GoalState {
                            gen: self.turn_gen,
                            iter: gs.iter + 1,
                            prev_done: done,
                            no_progress,
                            goal: gs.goal,
                        });
                        self.turn_handle = Some(spawn_turn_with(
                            GOAL_CONTINUE_PROMPT.to_string(),
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
                }
            } else {
                self.goal_state = Some(gs);
            }
        }
        if self.turn_handle.is_none() {
            if let Some(next) = self.take_next_queued_prompt() {
                self.start_turn(&next);
            }
        }
        if self.turn_handle.is_none() && self.turn_gen > self.last_auto_compact_gen {
            if let Some(lim) = self.app.context_limit {
                let cap = self.session.lock().await.compact_cap_tokens();
                let trigger = forge_core::auto_compact_trigger_tokens(
                    lim as u64,
                    cap,
                    AUTO_COMPACT_THRESHOLD,
                );
                if self.app.context_tokens > trigger {
                    let fill = self.app.context_tokens as f64 / lim as f64;
                    self.app.note(&format!(
                        "⚒ context {:.0}% full — auto-compacting",
                        fill * 100.0
                    ));
                    self.turn_gen += 1;
                    self.last_auto_compact_gen = self.turn_gen;
                    self.turn_handle = Some(spawn_compact(
                        &self.session,
                        &self.done_tx,
                        self.turn_gen,
                        &mut self.app,
                        &mut self.busy,
                        &mut self.busy_since,
                    ));
                }
            }
        }
    }

    /// Poll the background overlay loads (/mesh, /usage) — same as the TUI's per-frame polls.
    fn poll_overlay_loads(&mut self) {
        if let Some(rx) = &mut self.mesh_load_rx {
            match rx.try_recv() {
                Ok(Some(overlay)) => {
                    let tick = self.app.mesh_overlay.anim_tick;
                    self.app.mesh_overlay = overlay;
                    self.app.mesh_overlay.anim_tick = tick;
                    self.mesh_load_rx = None;
                }
                Ok(None) => {
                    self.app.mesh_overlay.open = false;
                    self.mesh_load_rx = None;
                    self.app.push_scrollback_text(
                        "mesh: auto-discovery routing is off (no model catalog) — nothing to inspect",
                    );
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.app.mesh_overlay.open = false;
                    self.mesh_load_rx = None;
                }
            }
        }
        if let Some(rx) = &mut self.usage_load_rx {
            match rx.try_recv() {
                Ok(bstats) => {
                    let fracs = self
                        .session
                        .try_lock()
                        .map(|s| s.bridge_fractions())
                        .unwrap_or_default();
                    self.app.usage_overlay.claude_5h_in = bstats.claude_5h_in;
                    self.app.usage_overlay.claude_5h_out = bstats.claude_5h_out;
                    self.app.usage_overlay.claude_weekly_in = bstats.claude_weekly_in;
                    self.app.usage_overlay.claude_weekly_out = bstats.claude_weekly_out;
                    fill_subscription_pcts(&mut self.app.usage_overlay, &fracs, &bstats);
                    self.app.usage_overlay.loading = false;
                    self.usage_load_rx = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.app.usage_overlay.loading = false;
                    self.usage_load_rx = None;
                }
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
        }
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

    #[tokio::test]
    async fn stale_interrupt_done_signal_cannot_stop_fifo_drain() {
        let mut state = test_driver_state().await;
        state.busy = true;
        state.turn_handle = Some(tokio::spawn(std::future::pending()));
        state.queued_prompts = vec!["second".into(), "third".into()];

        state.interrupt_turn();
        assert_eq!(state.queued_prompts, vec!["third"]);
        assert_eq!(state.turn_gen, 12);
        assert!(state.busy);

        // The aborted generation's DoneGuard arrives after the replacement turn starts.
        state.on_turn_done(11).await;
        assert!(state.busy);
        assert_eq!(state.turn_gen, 12);
        assert!(state.turn_handle.is_some());
        assert_eq!(state.queued_prompts, vec!["third"]);

        // Completing the replacement turn drains the remaining prompt in FIFO order.
        state.turn_handle.take().unwrap().abort();
        state.on_turn_done(12).await;
        assert_eq!(state.queued_prompts, Vec::<String>::new());
        assert_eq!(state.turn_gen, 13);
        assert!(state.busy);
        assert!(state.turn_handle.is_some());
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
}
