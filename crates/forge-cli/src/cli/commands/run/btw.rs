//! `/btw <question>` (alias `/side`) background-task spawner — the render-loop-side half of
//! `docs/features/side-questions.md`. `run/dispatch.rs` sits at its CI file-size ratchet ceiling,
//! so this lives in its own file; the dispatch arm just forwards to
//! `DispatchOutcome::RunBtw`, which the render loop turns into this spawn (mirrors
//! `spawn_compact` in `autonomous.rs` — a side call is a background task exactly like a turn or a
//! compaction, so the spinner ticks while it's in flight).

use super::*;

/// Start a `/btw` side question: open the answer card and run the call on its own task, so it
/// never takes over the turn's spinner, handle or generation (Claude Code's side-question popup).
/// While a turn holds the session lock, the question goes to the session's detached
/// [`forge_core::BtwAsker`]; when idle it takes the lock briefly and asks directly. Either way the
/// answer arrives through the presenter and fills the card.
pub(crate) fn start_btw(
    question: String,
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    busy: bool,
) {
    app.open_btw(&question);
    if !busy {
        let s = session.clone();
        tokio::spawn(async move { s.lock().await.ask_btw(&question).await });
        return;
    }
    match forge_core::busy_asker(&app.session_id) {
        Some(asker) => {
            tokio::spawn(async move { asker.ask_and_emit(&question).await });
        }
        None => app.apply(forge_tui::PresenterEvent::Warning(
            "/btw is not ready for this turn yet; try again in a moment".to_string(),
        )),
    }
}
