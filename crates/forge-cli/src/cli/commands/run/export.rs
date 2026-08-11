//! `/export [path]` — render the CURRENT session's transcript to one self-contained HTML file
//! (docs/features/session-replay.md, "Shipped (follow-up 3)"). Shares its renderer with
//! `forge replay <id> --html <path>` (`crate::replay::render_html`). Lives in its own file
//! because `run/dispatch.rs` sits at its CI file-size ratchet ceiling — the dispatch arm just
//! forwards here.

use super::*;

/// Default export path when `/export` is called with no argument: `forge-session-<id8>.html` in
/// the current directory, mirroring `forge replay`'s 8-char short id convention.
fn default_export_path(id: &str) -> String {
    format!("forge-session-{}.html", &id[..id.len().min(8)])
}

pub(crate) async fn export_cmd(
    session: &Arc<tokio::sync::Mutex<Session>>,
    path_arg: Option<String>,
    app: &mut forge_tui::App,
) -> Result<DispatchOutcome> {
    let (id, entries) = {
        let s = session.lock().await;
        let id = s.session_id().to_string();
        let entries = s.load_replay(&id).map_err(|e| anyhow::anyhow!("{e}"))?;
        (id, entries)
    };
    if entries.is_empty() {
        app.note("● nothing to export yet — this session has no messages");
        return Ok(DispatchOutcome::Handled);
    }
    let short = &id[..id.len().min(8)];
    let path = path_arg.unwrap_or_else(|| default_export_path(&id));
    let html = crate::replay::render_html(short, &entries);
    std::fs::write(&path, html).with_context(|| format!("writing {path}"))?;
    app.note(&format!("✓ exported session {short} to {path}"));
    Ok(DispatchOutcome::Handled)
}
