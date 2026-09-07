//! `/subagents [free|pinned]` — whether this session's subagents may route off its model pin.
//! Lives in its own file because `run/dispatch.rs` sits at its CI file-size ratchet ceiling (same
//! split as `btw.rs` / `export.rs`); the dispatch arm just forwards here.

use super::*;

/// Set (or toggle, with `None`) this session's subagent pin release, and say what changed. The
/// override is persisted by the session, so a resume or a daemon restart keeps it.
pub(crate) async fn set_subagent_pin_release(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    explicit: Option<bool>,
) {
    let mut s = session.lock().await;
    let free = explicit.unwrap_or(!s.subagents_free());
    s.set_subagents_free(Some(free));
    let pin = s.pinned_model().map(|set| set.join(", "));
    drop(s);
    match (free, pin) {
        (true, Some(pin)) => app.note(&format!(
            "⛓ subagents: free — children route the mesh independently (this session stays pinned to {pin})"
        )),
        (true, None) => app.note("⛓ subagents: free — children route the mesh independently"),
        (false, Some(pin)) => app.note(&format!("⛓ subagents: pinned — children inherit {pin}")),
        (false, None) => {
            app.note("⛓ subagents: pinned — children inherit the session's pin when one is set")
        }
    }
}
