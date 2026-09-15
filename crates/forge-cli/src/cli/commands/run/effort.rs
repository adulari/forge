//! `/effort [level]` and `/effort exact <level>` — the reasoning-rung controls.
//!
//! Lives in its own file because `run/dispatch.rs` sits at its CI file-size ratchet ceiling (same
//! split as `btw.rs` / `subagents.rs`); the dispatch arms just forward here.

use super::*;

/// Pin (or clear) the session's effort CEILING. A bare `/effort` opens the slider instead.
///
/// The ceiling is what the mesh optimises under: it picks the best-value rung at or below this from
/// the model's measured benchmark ladder.
pub(crate) async fn set_effort_ceiling(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    level: Option<String>,
) {
    match level {
        Some(ref s) => match forge_types::EffortLevel::parse(s) {
            Some(e) => {
                session.lock().await.set_effort(Some(e));
                app.apply(forge_tui::PresenterEvent::Effort(Some(e)));
                app.note(&format!(
                    "◎ effort pinned: {} — use /effort to adjust",
                    e.as_str()
                ));
            }
            None => app.note(&format!(
                "⚠ unknown effort level '{s}' — use low/medium/high/xhigh"
            )),
        },
        // Bare /effort → open the slider (same as Ctrl+R).
        None => app.effort_slider = true,
    }
}

/// Force (or clear) the EXACT rung the routed model runs at.
///
/// Distinct from the ceiling above: that one lets the mesh choose from the rungs a benchmark rated,
/// which is not the set a provider offers. Kimi Code serves low/high/max but is rated only at low
/// and max, so no ceiling can ask for its `high` — this can. The rung is still resolved against the
/// routed model's own provider ladder at routing time, so it can only name a rung that exists.
pub(crate) async fn set_exact_effort(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    level: Option<String>,
) {
    match level {
        Some(ref s) => match forge_types::EffortLevel::parse(s) {
            Some(e) => {
                session.lock().await.set_exact_effort(Some(e));
                app.note(&format!(
                    "◎ exact rung: {} — sent as-is, not capped. `/effort exact off` to clear",
                    e.as_str()
                ));
            }
            None => app.note(&format!(
                "⚠ unknown effort level '{s}' — use low/medium/high/xhigh/max"
            )),
        },
        None => {
            session.lock().await.set_exact_effort(None);
            app.note("◎ exact rung cleared — the mesh chooses again");
        }
    }
}
