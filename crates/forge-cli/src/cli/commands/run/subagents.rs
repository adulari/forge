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

/// `/delegation [on|off]`: allow or withhold subagent delegation for this thread, and say which.
pub(crate) async fn set_delegation(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    explicit: Option<bool>,
) {
    let mut s = session.lock().await;
    let allowed = explicit.unwrap_or(!s.delegation_allowed());
    s.set_delegation_allowed(allowed);
    drop(s);
    app.note(if allowed {
        "⛓ delegation: on - the model may spawn subagents"
    } else {
        "⛓ delegation: off - the model works alone in this thread"
    });
}

/// `/crew [name|off]`: with no argument list the saved crews; with a name select that crew for
/// every turn of this session; `off` clears it.
pub(crate) async fn run_crew(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    choice: Option<String>,
) {
    let mut s = session.lock().await;
    match choice.as_deref() {
        None => {
            let crews = s.saved_crews();
            let active = s.active_crew().map(str::to_string);
            drop(s);
            if crews.is_empty() {
                app.note("⛓ no saved crews. Add <name>.toml under ~/.config/forge/crews, or use Settings > Crews in the app.");
                return;
            }
            for crew in crews {
                let roles: Vec<&str> = crew.members.iter().map(|m| m.role.as_str()).collect();
                let mark = if active.as_deref() == Some(crew.name.as_str()) {
                    " (selected)"
                } else {
                    ""
                };
                app.note(&format!("⛓ {}{mark}: {}", crew.name, roles.join(", ")));
            }
            app.note("⛓ /crew <name> selects one for this session; @crew:<name> in a prompt uses it once");
        }
        Some("off" | "none") => {
            let _ = s.set_crew(None);
            drop(s);
            app.note("⛓ crew: cleared");
        }
        Some(name) => {
            let result = s.set_crew(Some(name));
            drop(s);
            match result {
                Ok(()) => app.note(&format!("⛓ crew: {name} - turns now delegate to it")),
                Err(e) => app.note(&format!("⚠ {e}")),
            }
        }
    }
}
