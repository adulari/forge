//! `/heartbeat` command body — the user's own recurring re-entry prompt for a session.
//! Split from dispatch.rs to keep that file within its architecture-size budget.

use std::sync::Arc;

use forge_core::Session;

/// Apply one `/heartbeat` sub-action. The user heartbeat is per-session and singular; agent-created
/// heartbeats (`manage_heartbeats`) are a separate owner this never touches.
pub(crate) async fn dispatch_heartbeat(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    action: forge_tui::HeartbeatAction,
) -> anyhow::Result<()> {
    let sid = session.lock().await.id().to_string();
    let now = chrono::Utc::now().timestamp();
    match action {
        forge_tui::HeartbeatAction::Every { interval, prompt } => {
            let prompt = prompt.trim().to_string();
            if prompt.is_empty() {
                app.note("usage: /heartbeat every <interval> <prompt>");
            } else {
                match forge_core::heartbeat::parse_heartbeat_interval(&interval) {
                    Ok(secs) => {
                        let s = session.lock().await;
                        match s.store.set_user_heartbeat(
                            &forge_types::new_id(),
                            &sid,
                            &prompt,
                            secs,
                            now,
                        ) {
                            Ok(()) => app.note(&format!(
                                "◷ heartbeat set — every {} — \"{prompt}\"",
                                forge_core::heartbeat::format_heartbeat_interval(secs)
                            )),
                            Err(e) => app.note(&format!("⚠ failed to set heartbeat: {e}")),
                        }
                    }
                    Err(e) => app.note(&format!("⚠ {e}")),
                }
            }
        }
        forge_tui::HeartbeatAction::Status => {
            let s = session.lock().await;
            match s.store.user_heartbeat(&sid) {
                Ok(Some(hb)) => {
                    let remaining = (hb.next_due_at - now).max(0);
                    app.note(&format!(
                        "◷ heartbeat: every {} — {} — next in ~{remaining}s — \"{}\"",
                        forge_core::heartbeat::format_heartbeat_interval(hb.interval_secs),
                        hb.status,
                        hb.prompt
                    ));
                }
                Ok(None) => app.note("no heartbeat set — /heartbeat every <interval> <prompt>"),
                Err(e) => app.note(&format!("⚠ {e}")),
            }
        }
        forge_tui::HeartbeatAction::Pause | forge_tui::HeartbeatAction::Resume => {
            let resume = matches!(action, forge_tui::HeartbeatAction::Resume);
            let s = session.lock().await;
            match s.store.user_heartbeat(&sid) {
                Ok(Some(hb)) => {
                    let status = if resume { "active" } else { "paused" };
                    match s.store.set_heartbeat_status(&hb.id, status, now) {
                        Ok(true) => app.note(if resume {
                            "▶ heartbeat resumed"
                        } else {
                            "⏸ heartbeat paused"
                        }),
                        Ok(false) => app.note("no heartbeat set"),
                        Err(e) => app.note(&format!("⚠ {e}")),
                    }
                }
                Ok(None) => app.note("no heartbeat set"),
                Err(e) => app.note(&format!("⚠ {e}")),
            }
        }
        forge_tui::HeartbeatAction::Clear => {
            let s = session.lock().await;
            match s.store.clear_user_heartbeat(&sid) {
                Ok(true) => app.note("✕ heartbeat cleared"),
                Ok(false) => app.note("no heartbeat set"),
                Err(e) => app.note(&format!("⚠ {e}")),
            }
        }
    }
    Ok(())
}
