//! `/btw <question>` (alias `/side`) background-task spawner — the render-loop-side half of
//! `docs/features/side-questions.md`. `run/dispatch.rs` sits at its CI file-size ratchet ceiling,
//! so this lives in its own file; the dispatch arm just forwards to
//! `DispatchOutcome::RunBtw`, which the render loop turns into this spawn (mirrors
//! `spawn_compact` in `autonomous.rs` — a side call is a background task exactly like a turn or a
//! compaction, so the spinner ticks while it's in flight).

use super::*;

/// Answer `/btw` beside a running turn: the turn holds the session lock, so the question goes to
/// the session's detached [`forge_core::BtwAsker`] and its answer card arrives through the
/// presenter like any other event. The turn's handle, generation and busy state are untouched.
pub(crate) fn spawn_busy_btw(question: String, app: &mut forge_tui::App) {
    match forge_core::busy_asker(&app.session_id) {
        Some(asker) => {
            app.note("◈ btw — asking beside the running turn…");
            tokio::spawn(async move { asker.ask_and_emit(&question).await });
        }
        None => app.note("⚠ /btw is not ready for this turn yet; try again in a moment"),
    }
}

/// Spawn `/btw` as a background task: it makes one model call, so the turn-busy spinner should
/// tick exactly like a real turn or `/compact`, even though the answer never joins the
/// transcript.
pub(crate) fn spawn_btw(
    question: String,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
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
        sess.ask_btw(&question).await;
    })
}
