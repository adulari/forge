//! `/fork [message]` and `/skills`: branch the live session into a child, and list the skills the
//! model can load. Split from `dispatch.rs`, which sits at its size ratchet.

use super::*;

/// Branch this session at its latest message, continue in the child, and (with a message) send
/// that message there as the first turn.
pub(crate) async fn fork(
    session: &Arc<tokio::sync::Mutex<Session>>,
    tui: Option<&mut forge_tui::Tui>,
    app: &mut forge_tui::App,
    message: Option<String>,
) -> Result<DispatchOutcome> {
    let parent = session.lock().await.id().to_string();
    let store = open_store()?;
    let last = store
        .load_replay(&parent)
        .context("loading the session to fork")?
        .last()
        .map(|e| e.seq);
    let Some(last) = last else {
        app.note("nothing to fork yet — send a message first");
        return Ok(DispatchOutcome::Handled);
    };
    let child = store
        .fork_session(&parent, last + 1)
        .context("forking the session")?;
    drop(store);
    let row = forge_tui::PickerRow {
        id: child.clone(),
        title: String::new(),
        subtitle: String::new(),
    };
    picker_accept(forge_tui::PickerKind::Sessions, &row, session, tui, app).await?;
    app.note(&format!(
        "⑂ forked {} → {} — the parent session is untouched",
        &parent[..parent.len().min(8)],
        &child[..child.len().min(8)]
    ));
    Ok(match message.filter(|m| !m.trim().is_empty()) {
        Some(prompt) => DispatchOutcome::RunTurn {
            prompt,
            guidance: Vec::new(),
            tier: None,
        },
        None => DispatchOutcome::Handled,
    })
}

/// The skills the model can load, one line each, grouped by scope.
pub(crate) fn skills(catalog: &forge_skills::Catalog, app: &mut forge_tui::App) {
    let mut all = catalog.all_skills();
    if all.is_empty() {
        app.note("no skills found — add one under .forge/skills/<name>/SKILL.md");
        return;
    }
    all.sort_by(|a, b| (a.scope.label(), &a.name).cmp(&(b.scope.label(), &b.name)));
    let mut lines = vec![format!(
        "skills · {} available (the model loads them by name)",
        all.len()
    )];
    for skill in all {
        let first = skill.description.lines().next().unwrap_or("").trim();
        let shown: String = first.chars().take(90).collect();
        lines.push(format!(
            "  {} ({}) — {shown}",
            skill.name,
            skill.scope.label()
        ));
    }
    app.note(&lines.join("\n"));
}
