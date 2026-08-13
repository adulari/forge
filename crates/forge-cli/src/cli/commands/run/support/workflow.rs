use super::*;

/// `/loop` runtime state: the generation of the in-flight loop turn and how many iterations have
/// run, so completion can be detected and capped.
pub(crate) struct LoopState {
    pub(crate) gen: u64,
    pub(crate) iter: usize,
    /// User-defined quality gates (`/loop --gate "<cmd>"`) that must all pass before the run is
    /// allowed to finish — empty when none were given (docs/features/autonomous-gates.md).
    pub(crate) gates: Vec<GateState>,
    pub(crate) gate_cfg: GateConfig,
    /// Token/wall-clock ceiling (`/loop --max-tokens`/`--max-minutes`) — unbounded when unset.
    pub(crate) budget: AutonomyBudget,
}

impl LoopState {
    pub(crate) fn new(
        gen: u64,
        gate_cmds: Vec<String>,
        max_tokens: Option<u64>,
        max_minutes: Option<u64>,
    ) -> Self {
        Self {
            gen,
            iter: 1,
            gates: gate_cmds
                .into_iter()
                .map(|cmd| GateState::new(GateSpec { cmd }))
                .collect(),
            gate_cfg: GateConfig::default(),
            budget: AutonomyBudget::new(max_tokens, max_minutes),
        }
    }
}

/// Iteration cap so a loop that never signals completion can't run forever.
pub(crate) const LOOP_MAX_ITERS: usize = 25;

/// Keys inside the workflow view's Enter-zoom transcript: mirrors the activity viewer's scrolling
/// (↑↓/PgUp/PgDn/Home/End, j/k/g/G) plus ←→/Tab to switch agents. An upward scroll first snaps
/// the stored offset from the "tail" sentinel down to the real max (recorded by the render path),
/// so scrolling back out of follow mode moves on the first keypress instead of unwinding a
/// `usize::MAX / 2` sentinel one line at a time.
pub(crate) fn workflow_zoom_key(app: &mut forge_tui::App, key: forge_tui::KeyKind) {
    use forge_tui::KeyKind;
    let max = app
        .workflow
        .zoom_geom
        .get()
        .map(|(wrapped_len, body_h)| wrapped_len.saturating_sub(body_h as usize));
    let clamp = |scroll: usize| max.map_or(scroll, |m| scroll.min(m));
    match key {
        KeyKind::Esc | KeyKind::Char('q') => app.workflow.zoom = None,
        KeyKind::Up | KeyKind::Char('k') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = false;
                z.scroll = clamp(z.scroll).saturating_sub(1);
            }
        }
        KeyKind::Down | KeyKind::Char('j') | KeyKind::Char(' ') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.scroll = clamp(z.scroll).saturating_add(1);
            }
            app.workflow.zoom_refollow_at_tail();
        }
        KeyKind::PageUp => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = false;
                z.scroll = clamp(z.scroll).saturating_sub(10);
            }
        }
        KeyKind::PageDown => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.scroll = clamp(z.scroll).saturating_add(10);
            }
            app.workflow.zoom_refollow_at_tail();
        }
        KeyKind::Home | KeyKind::Char('g') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = false;
                z.scroll = 0;
            }
        }
        KeyKind::End | KeyKind::Char('G') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = true;
                z.scroll = usize::MAX / 2;
            }
        }
        KeyKind::Left => {
            app.workflow.move_selection(-1);
            app.workflow.zoom = Some(Default::default());
        }
        KeyKind::Right | KeyKind::Tab => {
            app.workflow.move_selection(1);
            app.workflow.zoom = Some(Default::default());
        }
        _ => {}
    }
}

/// Spawn `/workflow run <name>` as a background task (docs/rfcs/forge-workflow.md): runs a saved
/// script directly, no authoring turn, same busy/spinner/interrupt semantics as a normal turn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_saved_workflow(
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
    name: String,
    args: serde_json::Value,
) -> tokio::task::JoinHandle<()> {
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.run_saved_workflow(&name, args).await {
            sess.notify_error(&format!("workflow '{name}' failed: {e}"));
        }
    })
}

/// Spawn `/duel <task>` as a background task (docs/features/duel.md): same busy/spinner/interrupt
/// semantics as a normal turn. Unlike `run_saved_workflow`, the result isn't just a presenter
/// event trail — the finished report + still-alive worktree guards must reach the render loop so
/// it can open a picker over the candidates, so they're written into `pending_duel` for the
/// done-signal drain to pick up.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_duel(
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
    task: String,
    pending_duel: Arc<std::sync::Mutex<PendingDuel>>,
) -> tokio::task::JoinHandle<()> {
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        match sess.run_duel(&task).await {
            Ok(result) => *pending_duel.lock().unwrap() = Some(result),
            Err(e) => sess.notify_error(&format!("duel failed: {e}")),
        }
    })
}
