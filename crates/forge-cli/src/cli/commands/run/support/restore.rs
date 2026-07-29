/// First use of a *project*-scope command/skill is confirmed by re-running it (its name is
/// "armed" on the first attempt and runs on the second) — unless project scope is trusted. User-
/// scope and builtins are never gated. Returns true when the invocation may proceed.
pub(crate) fn project_trust_ok(
    name: &str,
    scope: forge_skills::Scope,
    trust_project: bool,
    armed: &mut std::collections::HashSet<String>,
    app: &mut forge_tui::App,
) -> bool {
    if scope != forge_skills::Scope::Project || trust_project || armed.contains(name) {
        return true;
    }
    armed.insert(name.to_string());
    app.note(&format!(
        "⚠ /{name} is a project command — it can steer the model. Run it again to confirm."
    ));
    false
}

/// Populate + open the session picker from the store (newest first). `query` pre-fills the filter.
/// A clean, single-line title for a session row, derived from its first user prompt: newlines and
/// runs of whitespace collapse to single spaces, leading `/command` noise is kept, and the result
/// is trimmed to a readable length. Falls back to a placeholder when the session has no prompt.
pub(crate) fn session_title(preview: Option<&str>) -> String {
    let raw = preview.unwrap_or("").trim();
    if raw.is_empty() {
        return "(no prompt yet)".to_string();
    }
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let max = 64;
    if collapsed.chars().count() > max {
        format!("{}…", collapsed.chars().take(max - 1).collect::<String>())
    } else {
        collapsed
    }
}

/// Surface what an undo/restore did to the user's files.
pub(crate) fn note_restore(app: &mut forge_tui::App, report: &forge_core::snapshot::RestoreReport) {
    if !report.restored.is_empty() {
        app.note(&format!("↺ restored {} file(s)", report.restored.len()));
    }
    for w in &report.warnings {
        app.note(&format!(
            "⚠ {w} changed since Forge wrote it — overwrote your edit"
        ));
    }
    for f in &report.failed {
        app.note(&format!("✗ failed to restore {f}"));
    }
}

/// A short relative age like "3m ago" / "2h ago" / "5d ago" from an epoch-second timestamp.
pub(crate) fn fmt_age(created_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = (now - created_at).max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

pub(crate) fn find_starting_event_id(store: &forge_store::Store, session_id: &str) -> i64 {
    if let Ok(events) = store.live_events_after(session_id, 0) {
        for (id, json) in events.iter().rev() {
            if let Ok(ev) = serde_json::from_str::<crate::live_observer::LiveEvent>(json) {
                if matches!(ev, crate::live_observer::LiveEvent::AssistantDone) {
                    return *id;
                }
            }
        }
    }
    0
}
