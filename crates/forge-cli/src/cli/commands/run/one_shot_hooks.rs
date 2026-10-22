//! The prompt-side hooks outside the turn loop (docs/features/hooks.md): each `forge run` turn
//! fires what `claude -p` fires (`UserPromptExpansion` for a `/command`, then `UserPromptSubmit`),
//! and a typed `/command` in chat fires `UserPromptExpansion` before it runs.

use super::*;

/// `/name args…` → (`name`, `args…`); `None` for anything else, including a `//` escape.
fn slash_parts(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.trim().strip_prefix('/')?;
    if rest.starts_with('/') || rest.is_empty() {
        return None;
    }
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    Some((name, args.trim()))
}

/// The prompt-side hooks of one `forge run` turn: `UserPromptExpansion` when `expanded_from` is
/// the `/command` line that expanded into `prompt`, then `UserPromptSubmit`. Returns the prompt to
/// run and the guidance with any hook context added, or the hook's reason when one blocked it.
/// `SessionStart` / `SessionEnd` fire once per run, around every turn (see `one_shot.rs`).
pub(super) async fn prepare_turn(
    session: &mut Session,
    expanded_from: Option<&str>,
    prompt: &str,
    guidance: &[String],
) -> Result<(String, Vec<String>)> {
    let mut guidance = guidance.to_vec();
    let typed = expanded_from.and_then(|raw| slash_parts(raw).map(|parts| (raw, parts)));
    if let Some((raw, (name, args))) = typed {
        match session.run_expansion_hooks(name, args, "user", raw).await {
            Ok(context) => guidance.extend(context),
            Err(why) => anyhow::bail!("/{name} blocked by a UserPromptExpansion hook: {why}"),
        }
    }
    match session.run_prompt_submit_hooks(prompt).await {
        Ok(prompt) => Ok((prompt, guidance)),
        Err(why) => anyhow::bail!("prompt blocked by hook: {why}"),
    }
}

/// The typed line when `expand_one_shot_slash` turned it into something else (a `/command` or
/// skill), for `UserPromptExpansion`.
pub(super) fn expanded_from(
    raw: &str,
    prompt: &str,
    command_guidance: &[String],
) -> Option<String> {
    (!command_guidance.is_empty() || prompt != raw.trim()).then(|| raw.to_string())
}

/// `UserPromptExpansion` for a `/command` typed in chat. `Some(context)` to run it (with the
/// hooks' context to add), `None` when a hook blocked it (the reason is shown).
pub(super) async fn expansion_gate(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    line: &str,
    scope: forge_skills::Scope,
) -> Option<Vec<String>> {
    let Some((name, args)) = slash_parts(line) else {
        return Some(Vec::new());
    };
    let source = match scope {
        forge_skills::Scope::Project => "project",
        _ => "user",
    };
    let gated = session
        .lock()
        .await
        .run_expansion_hooks(name, args, source, line)
        .await;
    match gated {
        Ok(context) => Some(context),
        Err(why) => {
            app.note(&format!("⎇ /{name} blocked by hook: {why}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::slash_parts;

    #[test]
    fn slash_lines_split_into_command_and_arguments() {
        assert_eq!(
            slash_parts("/rust fix the parser "),
            Some(("rust", "fix the parser"))
        );
        assert_eq!(slash_parts("/deploy"), Some(("deploy", "")));
        assert_eq!(slash_parts("//etc/hosts"), None);
        assert_eq!(slash_parts("plain prompt"), None);
    }
}
