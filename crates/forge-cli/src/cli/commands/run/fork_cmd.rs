//! `/fork [message]` and `/skills`: branch the live session into a child, and list the skills the
//! model can load. Split from `dispatch.rs`, which sits at its size ratchet.

use super::*;

/// Split a leading `--model <id>` (or `--model=<id>`) off the `/fork` argument. The rest is the
/// first message for the child.
pub(crate) fn split_model_flag(arg: Option<String>) -> (Option<String>, Option<String>) {
    let Some(arg) = arg else {
        return (None, None);
    };
    let trimmed = arg.trim();
    let rest = if let Some(rest) = trimmed.strip_prefix("--model=") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("--model") {
        if !rest.starts_with(char::is_whitespace) {
            return (None, Some(arg));
        }
        rest.trim_start()
    } else {
        return (None, Some(arg));
    };
    let (model, message) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let message = message.trim();
    (
        (!model.is_empty()).then(|| model.to_string()),
        (!message.is_empty()).then(|| message.to_string()),
    )
}

/// Branch this session at its latest message, continue in the child, and (with a message) send
/// that message there as the first turn.
pub(crate) async fn fork(
    session: &Arc<tokio::sync::Mutex<Session>>,
    tui: Option<&mut forge_tui::Tui>,
    app: &mut forge_tui::App,
    message: Option<String>,
) -> Result<DispatchOutcome> {
    let (model, message) = split_model_flag(message);
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
    if let Some(model) = model {
        super::model_switch::pin_from_command(session, app, &model).await;
    }
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

#[cfg(test)]
mod tests {
    use super::split_model_flag;

    #[test]
    fn model_flag_is_split_from_the_message() {
        assert_eq!(split_model_flag(None), (None, None));
        assert_eq!(
            split_model_flag(Some("try again".into())),
            (None, Some("try again".into()))
        );
        assert_eq!(
            split_model_flag(Some("--model inferhub::x try again".into())),
            (Some("inferhub::x".into()), Some("try again".into()))
        );
        assert_eq!(
            split_model_flag(Some("--model=inferhub::x".into())),
            (Some("inferhub::x".into()), None)
        );
        assert_eq!(
            split_model_flag(Some("--modelx hi".into())),
            (None, Some("--modelx hi".into()))
        );
    }
}
